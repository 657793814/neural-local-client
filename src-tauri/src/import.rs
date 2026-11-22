//! 题目包导入：JSON / JSONL / ZIP。
//!
//! 只认这三种容器，不解析 Excel / CSV——只有一套校验口径，就不会出现
//! 「Excel 能进、JSON 进不去」的两套逻辑。逐题校验，单题失败不阻断整包。

use crate::models::OptionItem;
use crate::text;
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const MAX_ENTRIES: usize = 5000;
const MAX_TOTAL_BYTES: u64 = 200 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_PREVIEW_IMAGE_BYTES: u64 = 800 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub client_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub stem: String,
    pub options: Vec<OptionItem>,
    pub answer: String,
    pub analysis: String,
    pub difficulty: i64,
    pub tags: Vec<String>,
    pub knowledge_path: Vec<String>,
    /// 一题挂多个知识点时的全部路径；`knowledge_path` 是其中的第一条。
    #[serde(default)]
    pub knowledge_paths: Vec<Vec<String>>,
    pub source: Option<String>,
    pub year: Option<i64>,
    pub review: bool,
    pub review_note: Option<String>,
    pub subject_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewImage {
    pub file: String,
    pub kind: String,
    pub alt: String,
    pub exists: bool,
    pub data_url: Option<String>,
    #[serde(skip)]
    pub source: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    pub row_no: usize,
    /// READY / DUPLICATE / SIMILAR / INVALID
    pub status: String,
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
    pub draft: Draft,
    pub images: Vec<PreviewImage>,
    pub duplicate_of: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub id: String,
    pub filename: String,
    pub format: String,
    pub subject_hint: Option<String>,
    pub items: Vec<PreviewItem>,
    pub total: usize,
    pub ready: usize,
    pub duplicate: usize,
    pub similar: usize,
    pub invalid: usize,
    #[serde(skip)]
    pub workspace: Option<PathBuf>,
    #[serde(skip)]
    pub base_dir: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    pub batch_id: String,
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
    pub question_ids: Vec<String>,
}

fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn as_string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(|text| text.trim().to_string()))
            .filter(|item| !item.is_empty())
            .collect(),
        Some(Value::String(text)) => text
            .split([';', '；', ',', '，', '|'])
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_options(value: Option<&Value>, problems: &mut Vec<String>) -> Vec<OptionItem> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => {
            let mut out = Vec::new();
            for (index, item) in items.iter().enumerate() {
                match item {
                    Value::Object(map) => {
                        let key = map
                            .get("key")
                            .or_else(|| map.get("label"))
                            .and_then(|node| node.as_str())
                            .map(|text| text.trim().to_string())
                            .unwrap_or_else(|| ((b'A' + index as u8) as char).to_string());
                        let text = map
                            .get("text")
                            .or_else(|| map.get("content"))
                            .and_then(|node| node.as_str())
                            .unwrap_or("")
                            .to_string();
                        out.push(OptionItem { key, text });
                    }
                    Value::String(text) => {
                        let (key, body) = match text.split_once(['|', '．', '.', '、', ' ']) {
                            Some((key, body)) if key.trim().len() <= 2 => {
                                (key.trim().to_string(), body.trim().to_string())
                            }
                            _ => (
                                ((b'A' + index as u8) as char).to_string(),
                                text.trim().to_string(),
                            ),
                        };
                        out.push(OptionItem { key, text: body });
                    }
                    _ => problems.push(format!("第 {} 个选项不是对象或字符串", index + 1)),
                }
            }
            out
        }
        Value::String(text) => text
            .split(';')
            .enumerate()
            .filter_map(|(index, part)| {
                let part = part.trim();
                if part.is_empty() {
                    return None;
                }
                match part.split_once('|') {
                    Some((key, body)) => Some(OptionItem {
                        key: key.trim().to_string(),
                        text: body.trim().to_string(),
                    }),
                    None => Some(OptionItem {
                        key: ((b'A' + index as u8) as char).to_string(),
                        text: part.to_string(),
                    }),
                }
            })
            .collect(),
        _ => {
            problems.push("options 需要是数组或 `A|4;B|5` 这样的字符串".into());
            Vec::new()
        }
    }
}

fn parse_images(
    value: Option<&Value>,
    problems: &mut Vec<String>,
) -> Vec<(String, String, String)> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let Some(map) = item.as_object() else {
            continue;
        };
        let file = map
            .get("file")
            .and_then(|node| node.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if file.is_empty() {
            problems.push("images 里的 file 不能为空".into());
            continue;
        }
        if !safe_relative(&file) {
            problems.push(format!("图片路径不安全：{file}"));
            continue;
        }
        let kind = match map
            .get("kind")
            .and_then(|node| node.as_str())
            .unwrap_or("FIGURE")
        {
            "PAGE" => "PAGE",
            "CAPTURE" => "CAPTURE",
            _ => "FIGURE",
        };
        let alt = map
            .get("alt")
            .and_then(|node| node.as_str())
            .unwrap_or("")
            .to_string();
        out.push((file, kind.to_string(), alt));
    }
    out
}

/// 正文里的 Markdown 图片并入附件表；按路径去重，同一张图在题干和选项里都出现只算一次。
fn add_text_images(
    images: &mut Vec<(String, String, String)>,
    content: &str,
    label: &str,
    problems: &mut Vec<String>,
) {
    for md in text::extract_md_images(content) {
        if !safe_relative(&md.path) {
            problems.push(format!("{label}里的图片路径不安全：{}", md.path));
            continue;
        }
        if !images
            .iter()
            .any(|(file, _, _)| same_image_path(file, &md.path))
        {
            images.push((md.path, "FIGURE".to_string(), md.alt));
        }
    }
}

/// `images` 里写 `q1.png`、正文里写 `images/q1.png` 是同一张图；
/// 只比较后缀，不按文件名合并不同目录下的同名图。
fn same_image_path(left: &str, right: &str) -> bool {
    left == right || left.ends_with(&format!("/{right}")) || right.ends_with(&format!("/{left}"))
}

fn safe_relative(path: &str) -> bool {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return false;
    }
    candidate
        .components()
        .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn parse_item(row_no: usize, value: &Value, subject_hint: Option<&str>) -> PreviewItem {
    let mut problems = Vec::new();
    let mut warnings = Vec::new();
    let map = value.as_object();

    let stem = map
        .and_then(|map| map.get("stem"))
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if stem.is_empty() {
        problems.push("缺少题干 stem".into());
    }

    let raw_type = map
        .and_then(|map| map.get("type"))
        .and_then(|node| node.as_str())
        .unwrap_or("");
    let kind = text::normalize_type(raw_type)
        .unwrap_or("SOLVED")
        .to_string();
    if text::normalize_type(raw_type).is_none() {
        problems.push(format!(
            "题型「{raw_type}」认不出来，只能是 SINGLE / MULTI / FILL / SOLVED"
        ));
    }

    let options = parse_options(map.and_then(|map| map.get("options")), &mut problems);
    if (kind == "SINGLE" || kind == "MULTI") && options.len() < 2 {
        problems.push("选择题至少要有 2 个选项".into());
    }
    if options.len() > 7 {
        problems.push("选项最多 7 个".into());
    }

    let answer = map
        .and_then(|map| map.get("answer"))
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if answer.is_empty() {
        problems.push("缺少答案 answer".into());
    }

    let analysis = map
        .and_then(|map| map.get("analysis"))
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let difficulty = map
        .and_then(|map| map.get("difficulty"))
        .and_then(|node| node.as_i64())
        .map(|value| value.clamp(1, 5))
        .unwrap_or(3);

    let mut tags = as_string_list(map.and_then(|map| map.get("tags")));
    if tags.len() > 12 {
        warnings.push("标签超过 12 个，只保留前 12 个".into());
        tags.truncate(12);
    }

    let knowledge_path = as_string_list(map.and_then(|map| map.get("knowledgePath")));
    let mut knowledge_paths: Vec<Vec<String>> =
        match map.and_then(|node| node.get("knowledgePaths")) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| {
                    let path = as_string_list(Some(item));
                    if path.is_empty() {
                        None
                    } else {
                        Some(path)
                    }
                })
                .collect(),
            _ => Vec::new(),
        };
    // 旧包只带 examPoints：那些字符串就是知识点名，当单段路径并进来一起解析，
    // 不再单独校验、也不再单独落库。
    for name in as_string_list(map.and_then(|map| map.get("examPoints"))) {
        let path = vec![name];
        if !knowledge_paths.contains(&path) {
            knowledge_paths.push(path);
        }
    }
    if knowledge_path.is_empty() && knowledge_paths.is_empty() {
        problems.push("缺少知识点 knowledgePath（组卷与统计都靠它）".into());
    }

    let mut images = parse_images(map.and_then(|map| map.get("images")), &mut problems);
    let missing = as_string_list(map.and_then(|map| map.get("missingImages")));
    if !missing.is_empty() {
        warnings.push(format!(
            "这道题有 {} 张图没能导出（{}），导入后需要补图",
            missing.len(),
            missing.join("、")
        ));
    }

    let review = map
        .and_then(|map| map.get("review"))
        .and_then(|node| node.as_bool())
        .unwrap_or(false);
    if review {
        warnings.push("智能体标记了「需要人工确认」".into());
    }
    let review_note = map
        .and_then(|map| map.get("reviewNote"))
        .and_then(|node| node.as_str())
        .map(|text| text.to_string());

    // 题干 / 选项 / 解析里的 Markdown 图片都算附件：智能体有时只写正文不写 images。
    // 选项本身是一张图时（`![](images/a.png)`），漏掉这一处会让选项在入库后变成空文本。
    add_text_images(&mut images, &stem, "题干", &mut problems);
    for option in &options {
        add_text_images(&mut images, &option.text, "选项", &mut problems);
    }
    add_text_images(&mut images, &analysis, "解析", &mut problems);

    let client_id = map
        .and_then(|map| map.get("clientId"))
        .and_then(|node| node.as_str())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(uid);

    let source = map
        .and_then(|map| map.get("source"))
        .and_then(|node| node.as_str())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    let year = map
        .and_then(|map| map.get("year"))
        .and_then(|node| node.as_i64());

    let status = if problems.is_empty() {
        "READY"
    } else {
        "INVALID"
    };
    PreviewItem {
        row_no,
        status: status.to_string(),
        problems,
        warnings,
        draft: Draft {
            client_id,
            kind,
            stem,
            options,
            answer,
            analysis,
            difficulty,
            tags,
            knowledge_path,
            knowledge_paths,
            source,
            year,
            review,
            review_note,
            subject_hint: subject_hint.map(|value| value.to_string()),
        },
        images: images
            .into_iter()
            .map(|(file, kind, alt)| PreviewImage {
                file,
                kind,
                alt,
                exists: false,
                data_url: None,
                source: None,
            })
            .collect(),
        duplicate_of: None,
    }
}

fn attach_images(item: &mut PreviewItem, base_dir: &Path) {
    for image in &mut item.images {
        let candidate = base_dir.join(&image.file);
        let resolved = if candidate.exists() {
            Some(candidate)
        } else {
            candidate.file_name().and_then(|name| {
                let fallback = base_dir.join("images").join(name);
                if fallback.exists() {
                    Some(fallback)
                } else {
                    None
                }
            })
        };
        match resolved {
            Some(path) => {
                image.exists = true;
                if let Ok(meta) = std::fs::metadata(&path) {
                    if meta.len() <= MAX_PREVIEW_IMAGE_BYTES {
                        if let Ok(bytes) = std::fs::read(&path) {
                            if let Some((mime, _)) = text::sniff_image(&bytes) {
                                image.data_url = Some(format!(
                                    "data:{mime};base64,{}",
                                    base64::engine::general_purpose::STANDARD.encode(&bytes)
                                ));
                            }
                        }
                    }
                }
                image.source = Some(path);
            }
            None => {
                image.exists = false;
                item.warnings
                    .push(format!("找不到图片 {}，导入后请补图", image.file));
            }
        }
    }
    if item.images.iter().any(|image| !image.exists) {
        item.warnings.push("有图片缺失".into());
    }
}

fn dedupe(
    item: &mut PreviewItem,
    existing: &[(String, String)],
    batch: &mut Vec<(String, String)>,
) {
    if item.status == "INVALID" {
        return;
    }
    let normalized = text::normalize_stem(&item.draft.stem);
    if normalized.is_empty() {
        item.status = "INVALID".into();
        item.problems.push("题干去掉符号后是空的".into());
        return;
    }
    if let Some((id, _)) = existing
        .iter()
        .find(|(_, fingerprint)| fingerprint == &normalized)
    {
        item.status = "DUPLICATE".into();
        item.duplicate_of = Some(id.clone());
        item.warnings.push("题库里已有完全相同的题干".into());
        return;
    }
    let grams = text::bigrams(&normalized);
    let mut best: Option<(String, f64)> = None;
    for (id, fingerprint) in existing {
        let score = text::jaccard(&grams, &text::bigrams(fingerprint));
        if score >= 0.86 && best.as_ref().map(|(_, top)| score > *top).unwrap_or(true) {
            best = Some((id.clone(), score));
        }
    }
    if let Some((id, score)) = best {
        item.status = "SIMILAR".into();
        item.duplicate_of = Some(id);
        item.warnings
            .push(format!("和题库里一道题相似度 {:.0}%", score * 100.0));
        return;
    }
    for (id, fingerprint) in batch.iter() {
        if fingerprint == &normalized {
            item.status = "DUPLICATE".into();
            item.duplicate_of = Some(id.clone());
            item.warnings.push("本批次里有重复题".into());
            return;
        }
    }
    batch.push((item.draft.client_id.clone(), normalized));
}

fn finalize(mut preview: Preview, existing: &[(String, String)]) -> Preview {
    let mut batch: Vec<(String, String)> = Vec::new();
    if let Some(base) = preview.base_dir.clone() {
        for item in &mut preview.items {
            attach_images(item, &base);
        }
    }
    for item in &mut preview.items {
        dedupe(item, existing, &mut batch);
    }
    preview.total = preview.items.len();
    preview.ready = preview
        .items
        .iter()
        .filter(|item| item.status == "READY")
        .count();
    preview.duplicate = preview
        .items
        .iter()
        .filter(|item| item.status == "DUPLICATE")
        .count();
    preview.similar = preview
        .items
        .iter()
        .filter(|item| item.status == "SIMILAR")
        .count();
    preview.invalid = preview
        .items
        .iter()
        .filter(|item| item.status == "INVALID")
        .count();
    preview
}

fn pack_questions(value: &Value) -> Result<(Vec<Value>, Option<String>), String> {
    match value {
        Value::Array(items) => Ok((items.clone(), None)),
        Value::Object(map) => {
            let format = map
                .get("format")
                .and_then(|node| node.as_str())
                .unwrap_or("");
            if !format.is_empty() && format != "neural-question-pack" {
                return Err(format!("这是「{format}」格式，不是 Neural Question Pack"));
            }
            if let Some(version) = map.get("version").and_then(|node| node.as_i64()) {
                if version > 1 {
                    return Err(format!(
                        "题目包版本 v{version} 比当前客户端新，请先升级客户端"
                    ));
                }
            }
            let questions = map
                .get("questions")
                .and_then(|node| node.as_array())
                .cloned()
                .ok_or_else(|| "题目包里没有 questions 数组".to_string())?;
            let subject = map
                .get("subject")
                .and_then(|node| node.as_str())
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty());
            Ok((questions, subject))
        }
        _ => Err("题目包需要是一个 JSON 对象".into()),
    }
}

fn build_preview(
    filename: String,
    format: &str,
    values: Vec<Value>,
    subject_hint: Option<String>,
    base_dir: Option<PathBuf>,
    workspace: Option<PathBuf>,
    existing: &[(String, String)],
) -> Preview {
    let items: Vec<PreviewItem> = values
        .iter()
        .enumerate()
        .map(|(index, value)| parse_item(index + 1, value, subject_hint.as_deref()))
        .collect();
    let preview = Preview {
        id: uid(),
        filename,
        format: format.to_string(),
        subject_hint,
        items,
        total: 0,
        ready: 0,
        duplicate: 0,
        similar: 0,
        invalid: 0,
        workspace,
        base_dir,
    };
    finalize(preview, existing)
}

/// 从文件扫描题目包。ZIP 会解到 `<数据目录>/tmp/`，提交或丢弃后清理。
pub fn scan(
    path: &Path,
    data_dir: &Path,
    existing: &[(String, String)],
) -> Result<Preview, String> {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("未命名")
        .to_string();
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "json" => {
            let raw = std::fs::read_to_string(path).map_err(|e| format!("读不了文件：{e}"))?;
            let value: Value = serde_json::from_str(&raw)
                .map_err(|e| format!("JSON 不合法（第 {} 行）：{e}", e.line()))?;
            let (values, subject) = pack_questions(&value)?;
            Ok(build_preview(
                filename,
                "json",
                values,
                subject,
                path.parent().map(|parent| parent.to_path_buf()),
                None,
                existing,
            ))
        }
        "jsonl" | "ndjson" => {
            let raw = std::fs::read_to_string(path).map_err(|e| format!("读不了文件：{e}"))?;
            let mut values = Vec::new();
            let mut errors = 0;
            for line in raw.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(line) {
                    Ok(value) => values.push(value),
                    Err(_) => errors += 1,
                }
            }
            if values.is_empty() {
                return Err("这个 JSONL 里没有可用的行".into());
            }
            let mut preview = build_preview(
                filename,
                "jsonl",
                values,
                None,
                path.parent().map(|parent| parent.to_path_buf()),
                None,
                existing,
            );
            if errors > 0 {
                if let Some(first) = preview.items.first_mut() {
                    first
                        .warnings
                        .push(format!("有 {errors} 行不是合法 JSON，已跳过"));
                }
            }
            Ok(preview)
        }
        "zip" => scan_zip(path, data_dir, filename, existing),
        _ => Err(
            "只支持 .json / .jsonl / .zip。Excel 或 Word 先用「用 AI 转换」里的提示词转成 JSON。"
                .into(),
        ),
    }
}

pub fn scan_text(
    raw: &str,
    data_dir: &Path,
    existing: &[(String, String)],
) -> Result<Preview, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("还没有粘贴内容".into());
    }
    let value: Value = serde_json::from_str(trimmed).map_err(|e| {
        format!(
            "粘贴的内容不是合法 JSON（第 {} 行）：{e}。也可以只粘贴 {{\"questions\":[…]}}",
            e.line()
        )
    })?;
    let (values, subject) = pack_questions(&value)?;
    let _ = data_dir;
    Ok(build_preview(
        "粘贴导入".to_string(),
        "paste",
        values,
        subject,
        None,
        None,
        existing,
    ))
}

fn scan_zip(
    path: &Path,
    data_dir: &Path,
    filename: String,
    existing: &[(String, String)],
) -> Result<Preview, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("打不开压缩包：{e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("这不是有效的 ZIP：{e}"))?;
    if archive.len() > MAX_ENTRIES {
        return Err(format!(
            "压缩包里文件太多（{} 个），上限 {MAX_ENTRIES}",
            archive.len()
        ));
    }
    let workspace = data_dir.join("tmp").join(format!("import-{}", uid()));
    std::fs::create_dir_all(&workspace).map_err(|e| format!("建临时目录失败：{e}"))?;

    let mut total: u64 = 0;
    let mut questions_entry: Option<String> = None;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("读压缩包第 {} 项失败：{e}", index + 1))?;
        if entry.is_dir() {
            continue;
        }
        if entry
            .unix_mode()
            .map(|mode| mode & 0o170000 == 0o120000)
            .unwrap_or(false)
        {
            return Err("压缩包里有符号链接，已拒绝导入".into());
        }
        let name = entry
            .enclosed_name()
            .ok_or_else(|| "压缩包里有不安全的路径（含 .. 或绝对路径）".to_string())?
            .to_path_buf();
        let size = entry.size();
        if size > MAX_FILE_BYTES {
            return Err(format!("压缩包里 {} 太大（超过 20MB）", name.display()));
        }
        total += size;
        if total > MAX_TOTAL_BYTES {
            return Err("压缩包解压后超过 200MB，已拒绝导入".into());
        }
        let target = workspace.join(&name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("建目录失败：{e}"))?;
        }
        let mut buffer = Vec::with_capacity(size as usize);
        entry
            .read_to_end(&mut buffer)
            .map_err(|e| format!("解压 {} 失败：{e}", name.display()))?;
        std::fs::write(&target, &buffer).map_err(|e| format!("写入失败：{e}"))?;
        let lower = name.to_string_lossy().to_lowercase();
        if lower.ends_with("questions.json") {
            let depth = name.components().count();
            if questions_entry
                .as_ref()
                .map(|current| depth < Path::new(current).components().count())
                .unwrap_or(true)
            {
                questions_entry = Some(name.to_string_lossy().to_string());
            }
        }
    }

    let entry = questions_entry.ok_or_else(|| {
        "压缩包里没有 questions.json。请按 `questions.json + images/` 的结构打包".to_string()
    })?;
    let raw = std::fs::read_to_string(workspace.join(&entry))
        .map_err(|e| format!("读不了 questions.json：{e}"))?;
    let value: Value =
        serde_json::from_str(&raw).map_err(|e| format!("questions.json 不合法：{e}"))?;
    let (values, subject) = pack_questions(&value)?;
    let base_dir = workspace
        .join(&entry)
        .parent()
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(|| workspace.clone());

    Ok(build_preview(
        filename,
        "zip",
        values,
        subject,
        Some(base_dir),
        Some(workspace),
        existing,
    ))
}

fn resolve_subject(
    conn: &Connection,
    item: &PreviewItem,
    pack_subject: Option<&str>,
) -> Option<String> {
    let candidates: Vec<&str> = item
        .draft
        .subject_hint
        .as_deref()
        .into_iter()
        .chain(pack_subject)
        .chain(
            item.draft
                .knowledge_path
                .first()
                .map(|value| value.as_str()),
        )
        .collect();
    for name in candidates {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(id) = conn.query_row(
            "select id from subject where name = ?1 or code = ?1 limit 1",
            [trimmed],
            |row| row.get::<_, String>(0),
        ) {
            return Some(id);
        }
    }
    None
}

/// 已落盘、还没入库的图片附件。等题目写进去之后再插行，避免外键失败。
struct PendingAsset {
    id: String,
    kind: String,
    relative: String,
    content_type: String,
    sha256: String,
    width: Option<i64>,
    height: Option<i64>,
    sort: i64,
}

fn copy_asset(
    data_dir: &Path,
    question_id: &str,
    source: &Path,
    kind: &str,
    index: usize,
) -> Result<(String, String, String, Option<i64>, Option<i64>), String> {
    let bytes = std::fs::read(source).map_err(|e| format!("读图片失败：{e}"))?;
    let (mime, ext) = text::sniff_image(&bytes)
        .ok_or_else(|| format!("{} 不是 jpg / png / webp / gif", source.display()))?;
    let dir = data_dir.join("assets").join(question_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建图片目录失败：{e}"))?;
    let file_name = format!("{}-{}.{}", kind.to_lowercase(), index + 1, ext);
    let target = dir.join(&file_name);
    std::fs::write(&target, &bytes).map_err(|e| format!("写入图片失败：{e}"))?;
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let relative = format!("{question_id}/{file_name}");
    let (width, height) = image::image_dimensions(&target)
        .map(|(w, h)| (Some(w as i64), Some(h as i64)))
        .unwrap_or((None, None));
    Ok((relative, mime.to_string(), sha, width, height))
}

/// 提交预览里勾选的条目。图片按需落盘，正文里的 Markdown 引用改写成 `nbasset:`。
pub fn commit(
    conn: &mut Connection,
    data_dir: &Path,
    preview: &Preview,
    rows: &[usize],
    import_duplicates: bool,
) -> Result<CommitResult, String> {
    let batch_id = uid();
    let now = text::now();
    conn.execute(
        "insert into import_batch (id, filename, source_format, total, status, created_at)
         values (?1, ?2, ?3, ?4, 'COMMITTED', ?5)",
        params![
            batch_id,
            preview.filename,
            preview.format,
            preview.total as i64,
            now
        ],
    )
    .map_err(|e| e.to_string())?;

    let mut imported = 0;
    let mut skipped = 0;
    let mut failed = 0;
    let mut ids = Vec::new();

    for row in rows {
        let Some(item) = preview.items.iter().find(|item| item.row_no == *row) else {
            continue;
        };
        if item.status == "INVALID" {
            failed += 1;
            record_error(conn, &batch_id, item, "INVALID", &item.problems.join("；"))?;
            continue;
        }
        if (item.status == "DUPLICATE" || item.status == "SIMILAR") && !import_duplicates {
            skipped += 1;
            continue;
        }

        let subject_id = resolve_subject(conn, item, preview.subject_hint.as_deref());
        let knowledge_ids = resolve_knowledge_all(conn, subject_id.as_deref(), item);

        let mut input = crate::models::QuestionInput {
            id: Some(item.draft.client_id.clone()),
            subject_id,
            kind: item.draft.kind.clone(),
            stem: item.draft.stem.clone(),
            options: item.draft.options.clone(),
            answer: item.draft.answer.clone(),
            analysis: item.draft.analysis.clone(),
            difficulty: item.draft.difficulty,
            tags: item.draft.tags.clone(),
            knowledge_ids,
            source: item.draft.source.clone(),
            year: item.draft.year,
            status: Some("DRAFT".to_string()),
            origin: Some("IMPORT".to_string()),
            needs_review: Some(item.draft.review || !item.warnings.is_empty()),
            review_note: item.draft.review_note.clone(),
        };

        // 图片：先把文件落到 assets/ 并占好附件 id，改写题干引用；
        // 附件行要等题目入库之后再写——question_asset.question_id 有外键指向 question，
        // 顺序反了会直接 FOREIGN KEY constraint failed（带图包导入曾整个失败）。
        let question_id = input.id.clone().unwrap_or_else(uid);
        input.id = Some(question_id.clone());
        // 包里带了 clientId 且库里已有这道题：这次导入是「覆盖」，先把旧附件
        // 清干净再重来，否则同一张图会被追加成两份。
        let overwrite: Option<String> = conn
            .query_row(
                "select id from question where id = ?1",
                [&question_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if overwrite.is_some() {
            conn.execute(
                "delete from question_asset where question_id = ?1",
                [&question_id],
            )
            .map_err(|e| e.to_string())?;
            let _ = std::fs::remove_dir_all(data_dir.join("assets").join(&question_id));
        }
        // (包里的文件路径, 附件 id, 替代文字)
        let mut replacements: Vec<(String, String, String)> = Vec::new();
        let mut pending_assets: Vec<PendingAsset> = Vec::new();
        for (index, image) in item.images.iter().enumerate() {
            let Some(source) = image.source.as_deref() else {
                continue;
            };
            match copy_asset(data_dir, &question_id, source, &image.kind, index) {
                Ok((relative, mime, sha, width, height)) => {
                    let asset_id = uid();
                    if !image.file.is_empty() {
                        replacements.push((
                            image.file.clone(),
                            asset_id.clone(),
                            image.alt.clone(),
                        ));
                    }
                    pending_assets.push(PendingAsset {
                        id: asset_id,
                        kind: image.kind.clone(),
                        relative,
                        content_type: mime,
                        sha256: sha,
                        width,
                        height,
                        sort: index as i64,
                    });
                }
                Err(error) => {
                    failed += 1;
                    record_error(conn, &batch_id, item, "IMAGE", &error)?;
                }
            }
        }

        if !replacements.is_empty() {
            // 改写时把替代文字带上：题干里没写 alt 就用包里 images[].alt，
            // 否则图注（「图1」）会丢，导出和再导入都找不回来。
            let relink = |source: &str| -> String {
                let mut out = source.to_string();
                for md in text::extract_md_images(source) {
                    // `images` 里写 `q1.png`、正文里写 `images/q1.png` 很常见，
                    // 完全按字符串匹配会漏掉，正文就留着包内路径，入库后看起来是图丢了。
                    let Some((_, asset_id, alt)) = replacements
                        .iter()
                        .find(|(file, _, _)| same_image_path(file, &md.path))
                    else {
                        continue;
                    };
                    let label = if md.alt.trim().is_empty() {
                        alt.as_str()
                    } else {
                        md.alt.as_str()
                    };
                    out = out.replace(&md.whole, &format!("![{label}](nbasset:{asset_id})"));
                }
                out
            };
            input.stem = relink(&input.stem);
            input.analysis = relink(&input.analysis);
            // 选项里的图也要换成 nbasset，否则导入后选项还指着包内的 images/ 路径，
            // 详情页只显示一段 `![](images/x.png)`，看起来就是图丢了。
            for option in &mut input.options {
                option.text = relink(&option.text);
            }
        }

        match crate::repo::save(conn, &input) {
            Ok(id) => {
                for asset in &pending_assets {
                    conn.execute(
                        "insert into question_asset (id, question_id, kind, relative_path, content_type, sha256, width, height, sort)
                         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![asset.id, id, asset.kind, asset.relative, asset.content_type, asset.sha256, asset.width, asset.height, asset.sort],
                    )
                    .map_err(|e| e.to_string())?;
                }
                imported += 1;
                conn.execute(
                    "insert or ignore into import_batch_question (batch_id, question_id) values (?1, ?2)",
                    params![batch_id, id],
                )
                .map_err(|e| e.to_string())?;
                ids.push(id);
            }
            Err(error) => {
                failed += 1;
                // 题没存进去，刚落的图就是垃圾文件，顺手清掉。
                if !pending_assets.is_empty() {
                    let _ = std::fs::remove_dir_all(data_dir.join("assets").join(&question_id));
                }
                record_error(conn, &batch_id, item, "SAVE", &error)?;
            }
        }
    }

    conn.execute(
        "update import_batch set imported = ?2, skipped = ?3, failed = ?4 where id = ?1",
        params![batch_id, imported as i64, skipped as i64, failed as i64],
    )
    .map_err(|e| e.to_string())?;

    Ok(CommitResult {
        batch_id,
        imported,
        skipped,
        failed,
        question_ids: ids,
    })
}

fn record_error(
    conn: &Connection,
    batch_id: &str,
    item: &PreviewItem,
    code: &str,
    message: &str,
) -> Result<(), String> {
    conn.execute(
        "insert into import_error (id, batch_id, row_no, code, message, raw_json)
         values (?1, ?2, ?3, ?4, ?5, null)",
        params![uid(), batch_id, item.row_no as i64, code, message],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 撤销一批：按 import_batch 批量软删除，不硬删。
pub fn undo(conn: &mut Connection, batch_id: &str) -> Result<usize, String> {
    let ids: Vec<String> = conn
        .prepare("select question_id from import_batch_question where batch_id = ?1")
        .map_err(|e| e.to_string())?
        .query_map([batch_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    let changed = crate::repo::soft_delete(conn, &ids)?;
    conn.execute(
        "update import_batch set status = 'UNDONE' where id = ?1",
        [batch_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(changed)
}

/// 知识点按路径匹配；只写了知识点名时逐科目再找一遍。
/// 一条题可能带多条知识点路径（导出的包会这么写），逐条解析后合并去重。
fn resolve_knowledge_all(
    conn: &Connection,
    subject_id: Option<&str>,
    item: &PreviewItem,
) -> Vec<String> {
    let mut out = resolve_knowledge(conn, subject_id, &item.draft.knowledge_path);
    for path in &item.draft.knowledge_paths {
        for id in resolve_knowledge(conn, subject_id, path) {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

fn resolve_knowledge(conn: &Connection, subject_id: Option<&str>, path: &[String]) -> Vec<String> {
    if path.is_empty() {
        return Vec::new();
    }
    if let Some(subject) = subject_id {
        if let Some(id) = crate::db::resolve_knowledge_path(conn, subject, path) {
            return vec![id];
        }
    }
    let mut stmt = match conn.prepare("select id from subject") {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let subjects: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default();
    for subject in subjects {
        if let Some(id) = crate::db::resolve_knowledge_path(conn, &subject, path) {
            return vec![id];
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_full_item() {
        let value = json!({
            "type": "单选题",
            "stem": "若 $a=2$，则 $2a$ 等于多少？",
            "options": [{"key": "A", "text": "4"}, {"key": "B", "text": "5"}],
            "answer": "A",
            "analysis": "$2a=2\\times2=4$。",
            "difficulty": 1,
            "examPoints": ["代数式求值"],
            "tags": ["七年级"],
            "knowledgePath": ["初中数学 · 七年级上册", "第二章 整式的加减", "单项式与多项式"]
        });
        let item = parse_item(1, &value, Some("数学"));
        assert!(item.problems.is_empty(), "{:?}", item.problems);
        assert_eq!(item.draft.kind, "SINGLE");
        assert_eq!(item.draft.options.len(), 2);
        assert_eq!(item.draft.difficulty, 1);
    }

    #[test]
    fn missing_required_fields_are_reported() {
        let value = json!({"type": "SINGLE", "stem": "", "answer": ""});
        let item = parse_item(1, &value, None);
        assert_eq!(item.status, "INVALID");
        assert!(item.problems.iter().any(|p| p.contains("题干")));
        assert!(item.problems.iter().any(|p| p.contains("答案")));
        assert!(item.problems.iter().any(|p| p.contains("知识点")));
    }

    #[test]
    fn unsafe_image_path_rejected() {
        let value = json!({
            "type": "FILL", "stem": "x", "answer": "1", "examPoints": ["a"],
            "images": [{"file": "../../etc/passwd"}]
        });
        let item = parse_item(1, &value, None);
        assert!(item.problems.iter().any(|p| p.contains("不安全")));
    }

    #[test]
    fn options_string_form() {
        let mut problems = Vec::new();
        let options = parse_options(Some(&json!("A|4;B|5")), &mut problems);
        assert_eq!(options.len(), 2);
        assert_eq!(options[1].text, "5");
    }
}
