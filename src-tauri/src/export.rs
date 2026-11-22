//! 导出题目包：单 JSON 与「带图 ZIP」。
//!
//! 格式与导入共用同一份 v1 契约（`shared/question-format/question-pack.schema.json`），
//! 所以导出的 ZIP 能原样再导入：题干里 `![](nbasset:<id>)` 会改写成包内的
//! `images/...`，图片文件一并打进去。JSON 带不走图片，就把图片换成「【图】」占位，
//! 并把名字写进 `missingImages`——导入侧会用这条提示用户补图。

use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

use crate::repo;
use crate::text;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub count: usize,
    pub images: usize,
    /// 库里有这张图、但文件已经不在了的张数（题干里已占位）。
    pub missing: usize,
    pub bytes: u64,
    /// 导出得出来、但在别处可能被判不合格的题（例如没挂知识点）。
    pub warnings: Vec<String>,
}

struct AssetRow {
    id: String,
    kind: String,
    relative_path: String,
    content_type: String,
}

struct Item {
    /// 包里这份题的数据，`images` / `missingImages` 由调用方决定填哪种。
    value: Value,
    assets: Vec<AssetRow>,
    /// markdown 里给每张图写的替代文字，按 `nbasset:<id>` 分段取。
    alt_of: Vec<(String, String)>,
    /// 导出得出来、但换台机器导入会被挑刺的地方。
    warnings: Vec<String>,
}

fn asset_rows(conn: &Connection, question_id: &str) -> Result<Vec<AssetRow>, String> {
    let mut stmt = conn
        .prepare(
            "select id, kind, relative_path, content_type from question_asset
             where question_id = ?1 order by sort, created_at",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([question_id], |row| {
            Ok(AssetRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                relative_path: row.get(2)?,
                content_type: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// 教材 / 章节 / 知识点三段，和导入侧的匹配口径一致（三段都在时才按完整路径比对）。
fn knowledge_path(conn: &Connection, point_id: &str) -> Vec<String> {
    let row: Option<(String, Option<String>, Option<String>)> = conn
        .query_row(
            "select kp.name, ch.name, tb.name
             from knowledge_point kp
             left join chapter ch on ch.id = kp.chapter_id
             left join textbook tb on tb.id = ch.textbook_id
             where kp.id = ?1",
            [point_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    let Some((point, chapter, textbook)) = row else {
        return Vec::new();
    };
    let mut path = Vec::new();
    if let Some(textbook) = textbook.filter(|value| !value.trim().is_empty()) {
        path.push(textbook);
    }
    if let Some(chapter) = chapter.filter(|value| !value.trim().is_empty()) {
        path.push(chapter);
    }
    if !point.trim().is_empty() {
        path.push(point);
    }
    path
}

fn collect(conn: &Connection, ids: &[String]) -> Result<Vec<Item>, String> {
    if ids.is_empty() {
        return Err("还没有选题".into());
    }
    let mut items = Vec::new();
    for id in ids {
        let question = repo::get(conn, id)?;
        if question.deleted_at.is_some() {
            return Err("回收站里的题不能导出，先恢复".into());
        }
        let mut paths: Vec<Vec<String>> = Vec::new();
        for point in &question.knowledge {
            let path = knowledge_path(conn, &point.id);
            if !path.is_empty() && !paths.contains(&path) {
                paths.push(path);
            }
        }
        let primary = paths.first().cloned().unwrap_or_default();

        let mut value = json!({
            "clientId": question.id,
            "type": question.kind,
            "stem": question.stem,
            "options": question.options,
            "answer": question.answer,
            "analysis": question.analysis,
            "difficulty": question.difficulty,
            "tags": question.tags,
            "knowledgePath": primary,
            "source": question.source,
            "year": question.year,
        });
        // 一题挂多个知识点时，v1 的 knowledgePath 只能写一条；多出来的用
        // knowledgePaths 带上（schema 允许额外字段），导入侧会一并解析。
        if paths.len() > 1 {
            value["knowledgePaths"] = json!(paths);
        }
        if question.needs_review {
            value["review"] = json!(true);
            if let Some(note) = question
                .review_note
                .clone()
                .filter(|note| !note.trim().is_empty())
            {
                value["reviewNote"] = json!(note);
            }
        }

        // 题干、选项、解析都可能引用配图；选项本身是一张图时也要把图注带出去。
        let alt_of = collect_alts(&value);

        let mut warnings = Vec::new();
        if question.knowledge.is_empty() {
            warnings.push(format!(
                "第 {} 题没挂知识点，导入后组卷抽不到它",
                items.len() + 1
            ));
        }

        items.push(Item {
            value,
            assets: asset_rows(conn, id)?,
            alt_of,
            warnings,
        });
    }
    Ok(items)
}

fn subject_of(conn: &Connection, ids: &[String]) -> String {
    let mut names: Vec<String> = Vec::new();
    for id in ids {
        if let Ok(question) = repo::get(conn, id) {
            if !question.subject_name.is_empty() && !names.contains(&question.subject_name) {
                names.push(question.subject_name);
            }
        }
    }
    match names.len() {
        0 => String::new(),
        1 => names.remove(0),
        _ => "多学科".to_string(),
    }
}

fn envelope(
    conn: &Connection,
    ids: &[String],
    questions: Vec<Value>,
    warnings: &[String],
    with_images: bool,
) -> Value {
    let mut body = json!({
        "format": "neural-question-pack",
        "version": 1,
        "generatedAt": text::now(),
        "generator": "neuralkb-local",
        "withImages": with_images,
        "subject": subject_of(conn, ids),
        "count": questions.len(),
        "questions": questions,
    });
    if !warnings.is_empty() {
        body["warnings"] = json!(warnings);
    }
    body
}

/// 题干里的图片是以 `![alt](nbasset:<id>)` 整段存在的，替换时也必须整段换掉，
/// 只换括号里的目标会留下 `![图1]` 这种半截 markdown。
fn swap_image(text: &str, asset_id: &str, build: &dyn Fn(Option<&str>) -> String) -> String {
    let target = format!("nbasset:{asset_id}");
    let mut out = text.to_string();
    for md in text::extract_md_images(text) {
        if md.path.trim() != target {
            continue;
        }
        let alt = if md.alt.trim().is_empty() {
            None
        } else {
            Some(md.alt.as_str())
        };
        out = out.replace(&md.whole, &build(alt));
    }
    out
}

fn push_alts(out: &mut Vec<(String, String)>, text: &str) {
    for md in text::extract_md_images(text) {
        if let Some(id) = md.path.strip_prefix("nbasset:") {
            let id = id.trim().to_string();
            if !id.is_empty() && !out.iter().any(|(existing, _)| existing == &id) {
                out.push((id, md.alt));
            }
        }
    }
}

/// 题干 / 选项 / 解析里给每张图写的替代文字，按 `nbasset:<id>` 取。
fn collect_alts(value: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(stem) = value.get("stem").and_then(Value::as_str) {
        push_alts(&mut out, stem);
    }
    if let Some(options) = value.get("options").and_then(Value::as_array) {
        for option in options {
            if let Some(text) = option.get("text").and_then(Value::as_str) {
                push_alts(&mut out, text);
            }
        }
    }
    if let Some(analysis) = value.get("analysis").and_then(Value::as_str) {
        push_alts(&mut out, analysis);
    }
    out
}

/// 一道题里所有可能出现 `nbasset:<id>` 的位置一起替换。
/// 返回是否真的换到了；只改题干会让选项图在导出后指着本机路径，换个机器就成了丢图。
fn swap_image_fields(
    value: &mut Value,
    asset_id: &str,
    build: &dyn Fn(Option<&str>) -> String,
) -> bool {
    let mut changed = false;
    if let Some(stem) = value
        .get("stem")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        let next = swap_image(&stem, asset_id, build);
        if next != stem {
            value["stem"] = json!(next);
            changed = true;
        }
    }
    if let Some(options) = value.get_mut("options").and_then(Value::as_array_mut) {
        for option in options {
            let Some(text) = option
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };
            let next = swap_image(&text, asset_id, build);
            if next != text {
                option["text"] = json!(next);
                changed = true;
            }
        }
    }
    if let Some(analysis) = value
        .get("analysis")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        let next = swap_image(&analysis, asset_id, build);
        if next != analysis {
            value["analysis"] = json!(next);
            changed = true;
        }
    }
    changed
}

/// 单 JSON：图片带不走，题干里的图换成「【图】」，名字进 `missingImages`。
pub fn pack_json(conn: &Connection, ids: &[String]) -> Result<String, String> {
    let items = collect(conn, ids)?;
    let mut warnings = Vec::new();
    let mut questions = Vec::new();
    for item in items {
        warnings.extend(item.warnings.iter().cloned());
        let mut value = item.value;
        let mut missing: Vec<String> = Vec::new();
        for (index, asset) in item.assets.iter().enumerate() {
            let alt = item
                .alt_of
                .iter()
                .find(|(id, _)| id == &asset.id)
                .map(|(_, alt)| alt.clone())
                .filter(|alt| !alt.trim().is_empty())
                .unwrap_or_else(|| format!("图{}", index + 1));
            // 图注留在 missingImages 里，正文位置放占位符。
            let swapped = swap_image_fields(&mut value, &asset.id, &|_| "【图】".to_string());
            if !swapped {
                continue;
            }
            missing.push(alt);
        }
        if !missing.is_empty() {
            value["missingImages"] = json!(missing);
        }
        questions.push(value);
    }
    Ok(envelope(conn, ids, questions, &warnings, false).to_string())
}

fn extension(content_type: &str, relative: &str) -> String {
    if let Some(ext) = Path::new(relative)
        .extension()
        .and_then(|value| value.to_str())
    {
        if !ext.is_empty() {
            return ext.to_lowercase();
        }
    }
    match content_type {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
    .to_string()
}

/// 带图 ZIP：`questions.json` + `images/`，图片路径写进 `images[]`，题干同步改写。
pub fn export_zip(
    conn: &Connection,
    data_dir: &Path,
    ids: &[String],
    dest: &Path,
) -> Result<ExportResult, String> {
    let items = collect(conn, ids)?;
    let mut warnings = Vec::new();
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut questions = Vec::new();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut missing = 0usize;

    for (order, item) in items.into_iter().enumerate() {
        warnings.extend(item.warnings.iter().cloned());
        let mut value = item.value;
        let mut images = Vec::new();
        let mut missing_names: Vec<String> = Vec::new();
        for (index, asset) in item.assets.iter().enumerate() {
            let alt = item
                .alt_of
                .iter()
                .find(|(id, _)| id == &asset.id)
                .map(|(_, alt)| alt.clone())
                .unwrap_or_default();
            // relative_path 相对的是 data_dir/assets（和 asset_data 命令同一口径）。
            let source = data_dir.join("assets").join(&asset.relative_path);
            if !source.exists() {
                let swapped = swap_image_fields(&mut value, &asset.id, &|_| "【图】".to_string());
                if !swapped {
                    continue;
                }
                missing += 1;
                missing_names.push(if alt.trim().is_empty() {
                    format!("图{}", index + 1)
                } else {
                    alt.clone()
                });
                continue;
            }
            let name = format!(
                "images/{}-{}.{}",
                order + 1,
                index + 1,
                extension(&asset.content_type, &asset.relative_path)
            );
            let swapped = swap_image_fields(&mut value, &asset.id, &|md_alt| {
                let label = md_alt.unwrap_or(alt.as_str());
                format!("![{label}]({name})")
            });
            if !swapped {
                continue;
            }
            images.push(json!({"file": name, "kind": asset.kind, "alt": alt}));
            files.push((name, source));
        }
        if !images.is_empty() {
            value["images"] = json!(images);
        }
        if !missing_names.is_empty() {
            value["missingImages"] = json!(missing_names);
        }
        questions.push(value);
    }

    let image_count = files.len();
    let body = envelope(conn, ids, questions, &warnings, true).to_string();

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("建目录失败：{e}"))?;
    }
    let file = std::fs::File::create(dest).map_err(|e| format!("建导出文件失败：{e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("questions.json", options)
        .map_err(|e| e.to_string())?;
    zip.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    for (name, source) in &files {
        let bytes = std::fs::read(source).map_err(|e| format!("读 {name} 失败：{e}"))?;
        zip.start_file(name.as_str(), options)
            .map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;

    let bytes = std::fs::metadata(dest).map(|meta| meta.len()).unwrap_or(0);
    Ok(ExportResult {
        path: dest.to_string_lossy().to_string(),
        count: ids.len(),
        images: image_count,
        missing,
        bytes,
        warnings,
    })
}
