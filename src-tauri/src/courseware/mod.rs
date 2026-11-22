//! 教师课件（P4）：讲义式课件库 + 讲义生成器。
//!
//! 「课件」在这里是**讲义**（教案/学案），不是 3D 交互仿真（那是 `courseware-lab` 的活）。
//! 见 `docs/product/local-first-teaching-plan.md` §8：讲义 = 知识点讲授 + 例题精讲 +
//! 变式拓展 + 拔高 + 思维训练 + 小结作业，题目尽量挂本地题库，模型只补题与解析。
//!
//! - 8.1a 库：导入已有单文件 HTML（讲义成品 / courseware-lab 3D 成品），填元信息、
//!   隔离预览、按班级布置；
//! - 8.1b 生成器：表单填 HandoutSpec，纯 Rust 渲染成单文件讲义 HTML（内联题干解析，
//!   A4 打印分页友好），不把 Node 工具链塞进安装包。
//!
//! 讲义文件落 `assets/courseware/<id>.html`，进程内用自定义协议 `nbasset://`
//! 提供给 iframe，避免给整个数据目录开放 asset 协议作用域。

use crate::state::AppState;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub mod ingest;
pub mod office;
pub mod prompt;
pub mod render;
pub mod source;

/// 单份讲义上限。courseware-lab 3D 成品约 1MB，讲义更小，留足第三方成品余量。
pub const MAX_BYTES: u64 = 16 * 1024 * 1024;
/// 低于这个体积基本可以判定产物被截断。
const MIN_OUTPUT_BYTES: usize = 4096;

// ---------------------------------------------------------------------------
// 数据形状
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignedClass {
    pub id: String,
    pub name: String,
    pub due_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Courseware {
    pub id: String,
    pub title: String,
    pub subject_id: Option<String>,
    pub subject_name: String,
    pub grade: String,
    pub description: String,
    pub knowledge: Vec<String>,
    pub source: String,
    pub version: i64,
    pub step_count: i64,
    pub quiz_count: i64,
    pub size_bytes: i64,
    pub warnings: Vec<String>,
    pub classes: Vec<AssignedClass>,
    pub has_spec: bool,
    pub question_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub path: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subject_id: Option<String>,
    #[serde(default)]
    pub grade: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub knowledge: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub id: String,
    pub title: String,
    pub warnings: Vec<String>,
    pub duplicated: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoursewarePatch {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subject_id: Option<String>,
    #[serde(default)]
    pub grade: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub knowledge: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignRequest {
    pub courseware_id: String,
    #[serde(default)]
    pub class_ids: Vec<String>,
    #[serde(default)]
    pub due_at: Option<String>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub id: String,
    pub title: String,
    pub url: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    pub path: String,
    pub exported: bool,
    pub needs_print: bool,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Handout Spec（表单 ↔ 编译器 ↔ 编辑器回填共用一份）
// ---------------------------------------------------------------------------

/// 一道挂题库或内联题干的题。`question_id` 优先：编译时把题库里的题干/答案/解析
/// 内联进讲义，同时在 `local_courseware_question` 留一份挂接（学情可回溯）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoutItem {
    #[serde(default)]
    pub question_id: String,
    #[serde(default)]
    pub stem: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub analysis: String,
    #[serde(default)]
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointSpec {
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub misconception: String,
    #[serde(default)]
    pub formula: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkSpec {
    pub prompt: String,
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub answer: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoutImage {
    /// 稳定编号，例如「图1」；提示词、正文和导出器都用它引用。
    pub id: String,
    /// 相对 assets/ 的受管路径，不依赖老师原文件还在原地。
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub source_path: String,
    #[serde(default)]
    pub caption: String,
    #[serde(default)]
    pub placement: String,
    #[serde(default)]
    pub required: bool,
    /// 仅渲染时填充，不写回课件草稿。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub data_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceTrace {
    #[serde(default)]
    pub original_path: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub locations: Vec<String>,
    /// FORMAT_ONLY 仅统一格式；OPTIMIZE 允许老师确认优化内容。
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoutSpec {
    pub title: String,
    #[serde(default)]
    pub subject_id: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub grade: String,
    #[serde(default)]
    pub chapter: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub knowledge: Vec<String>,
    #[serde(default)]
    pub objectives: Vec<String>,
    #[serde(default)]
    pub warmup: String,
    #[serde(default)]
    pub points: Vec<PointSpec>,
    #[serde(default)]
    pub examples: Vec<HandoutItem>,
    #[serde(default)]
    pub variants: Vec<HandoutItem>,
    #[serde(default)]
    pub challenge: Vec<HandoutItem>,
    #[serde(default)]
    pub thinking: Vec<ThinkSpec>,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub homework: Vec<String>,
    #[serde(default)]
    pub theme: String,
    /// REGULAR 常规课 / TOPIC 专题课，只影响提示词结构，不另建内容模型。
    #[serde(default)]
    pub content_mode: String,
    #[serde(default)]
    pub duration: String,
    #[serde(default)]
    pub audience: String,
    #[serde(default)]
    pub difficulty: String,
    #[serde(default)]
    pub teaching_focus: String,
    #[serde(default)]
    pub teaching_difficulty: String,
    #[serde(default)]
    pub teaching_style: String,
    #[serde(default)]
    pub custom_instruction: String,
    #[serde(default)]
    pub images: Vec<HandoutImage>,
    #[serde(default)]
    pub source: SourceTrace,
    /// 优化整理模式下由 AI 补写的内容说明，校对时单独提示。
    #[serde(default)]
    pub ai_added: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gate {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildOutcome {
    pub ok: bool,
    pub id: Option<String>,
    pub gates: Vec<Gate>,
    pub message: String,
    pub warnings: Vec<String>,
}

/// 讲义里一道题与题库的挂接角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Example,
    Variant,
    Challenge,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::Example => "EXAMPLE",
            Role::Variant => "VARIANT",
            Role::Challenge => "CHALLENGE",
        }
    }
}

// ---------------------------------------------------------------------------
// 导入
// ---------------------------------------------------------------------------

/// 静态扫描：外链脚本直接拒，其他外链给提示（断网会缺图/缺字体）。
pub fn inspect_html(html: &str) -> Result<Vec<String>, String> {
    let lower = html.to_ascii_lowercase();
    if !lower.contains("<html") && !lower.contains("<!doctype") {
        return Err("这不是一份 HTML 文件（找不到 <html> 开头）".into());
    }
    // 逐个 <script ...> 看有没有 src 指向网络地址
    let mut cursor = 0usize;
    while let Some(at) = lower[cursor..].find("<script") {
        let start = cursor + at;
        let tail = &lower[start..];
        let end = tail
            .find('>')
            .map(|value| start + value)
            .unwrap_or(lower.len());
        let tag = &lower[start..end];
        if tag.contains("src=")
            && (tag.contains("http://")
                || tag.contains("https://")
                || tag.contains("src=\"//")
                || tag.contains("src='//"))
        {
            return Err(
                "课件引用了外部脚本（<script src=\"http…\">），断网打不开；请先把脚本内联".into(),
            );
        }
        cursor = end.max(start + 7);
        if cursor >= lower.len() {
            break;
        }
    }
    let mut warnings = Vec::new();
    let remote = lower.matches("http://").count() + lower.matches("https://").count();
    let calls = lower.matches("fetch(").count() + lower.matches("xmlhttprequest").count();
    if remote > 0 {
        warnings.push(format!(
            "含 {remote} 处网络地址，断网时图片/字体/接口可能缺；能内联就内联"
        ));
    }
    if calls > 0 {
        warnings.push(format!("含 {calls} 处 fetch/XHR 调用，离线时会失败或超时"));
    }
    if lower.contains("localstorage") {
        warnings.push("用到了 localStorage：在预览里可用，换机器不带走".into());
    }
    Ok(warnings)
}

fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now() -> String {
    chrono::Local::now().to_rfc3339()
}

fn json_list(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".into())
}

fn parse_list(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn courseware_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("assets").join("courseware")
}

fn safe_component(raw: &str, fallback: &str) -> String {
    let cleaned = raw
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let cleaned = cleaned.trim_matches(['.', '_']).to_string();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned.chars().take(80).collect()
    }
}

fn managed_path(assets_root: &Path, raw: &str) -> Result<PathBuf, String> {
    let relative = Path::new(raw.trim());
    if raw.trim().is_empty()
        || relative.is_absolute()
        || relative.components().any(|part| {
            !matches!(
                part,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return Err("图片路径必须是 assets/ 内的相对路径".into());
    }
    Ok(assets_root.join(relative))
}

/// 读取受管图片；受管副本不存在时才退回用户最初选择的本机文件。
pub fn read_image(assets_root: &Path, image: &HandoutImage) -> Result<(Vec<u8>, String), String> {
    let managed = managed_path(assets_root, &image.path).ok();
    let source = managed
        .filter(|path| path.is_file())
        .or_else(|| {
            let original = PathBuf::from(image.source_path.trim());
            original.is_file().then_some(original)
        })
        .ok_or_else(|| format!("读不到图片「{}」", image.id))?;
    let bytes = std::fs::read(&source).map_err(|e| format!("读取图片失败：{e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("单张图片不能超过 {} MB", MAX_BYTES / 1024 / 1024));
    }
    let mime = match image::guess_format(&bytes).map_err(|_| "无法识别图片格式".to_string())?
    {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        _ => return Err("仅支持 PNG、JPEG、GIF、WebP 图片".into()),
    };
    Ok((bytes, mime.to_string()))
}

/// 给外部智能体准备素材时，直接打开这份草稿的受管图片目录。
pub fn draft_image_dir(data_dir: &Path, draft_id: &str) -> Result<PathBuf, String> {
    let draft = safe_component(draft_id, "draft");
    let dir = data_dir
        .join("assets")
        .join("courseware")
        .join("drafts")
        .join(draft);
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err("这份草稿还没有本地图片".into())
    }
}

/// 添加图片时立即复制到草稿目录，之后不再依赖老师原文件的位置。
pub fn add_image(
    data_dir: &Path,
    draft_id: &str,
    image_id: &str,
    source_path: &str,
) -> Result<HandoutImage, String> {
    let source = Path::new(source_path.trim());
    if !source.is_file() {
        return Err("请选择一张本机图片".into());
    }
    let bytes = std::fs::read(source).map_err(|e| format!("读取图片失败：{e}"))?;
    let extension = match image::guess_format(&bytes).map_err(|_| "无法识别图片格式".to_string())?
    {
        image::ImageFormat::Png => "png",
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Gif => "gif",
        image::ImageFormat::WebP => "webp",
        _ => return Err("仅支持 PNG、JPEG、GIF、WebP 图片".into()),
    };
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("单张图片不能超过 {} MB", MAX_BYTES / 1024 / 1024));
    }
    let draft = safe_component(draft_id, "draft");
    let id = safe_component(image_id, "图");
    let relative = format!("courseware/drafts/{draft}/{id}.{extension}");
    let target = data_dir.join("assets").join(&relative);
    let parent = target
        .parent()
        .ok_or_else(|| "图片保存位置不正确".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("创建图片目录失败：{e}"))?;
    std::fs::write(&target, bytes).map_err(|e| format!("保存图片副本失败：{e}"))?;
    Ok(HandoutImage {
        id: image_id.trim().to_string(),
        path: relative,
        name: source
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default(),
        source_path: source.to_string_lossy().to_string(),
        ..Default::default()
    })
}

pub fn import(
    conn: &Connection,
    data_dir: &Path,
    req: &ImportRequest,
) -> Result<ImportResult, String> {
    let path = Path::new(&req.path);
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext != "html" && ext != "htm" {
        return Err("只支持 .html / .htm 单文件课件".into());
    }
    let size = std::fs::metadata(path)
        .map_err(|e| format!("读不到这个文件：{e}"))?
        .len();
    if size == 0 {
        return Err("这个文件是空的".into());
    }
    if size > MAX_BYTES {
        return Err(format!(
            "课件超过 {} MB，先精简一下再导入",
            MAX_BYTES / 1024 / 1024
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("读文件失败：{e}"))?;
    let html = String::from_utf8_lossy(&bytes).to_string();
    let warnings = inspect_html(&html)?;
    let sha = hex_sha256(&bytes);

    let existing: Option<(String, String)> = conn
        .query_row(
            "select id, title from local_courseware where file_sha256 = ?1 and deleted_at is null",
            [&sha],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((id, title)) = existing {
        return Ok(ImportResult {
            id,
            title,
            warnings,
            duplicated: true,
        });
    }

    let id = uuid::Uuid::new_v4().to_string();
    let dir = courseware_dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建课件目录失败：{e}"))?;
    let target = dir.join(format!("{id}.html"));
    std::fs::write(&target, &bytes).map_err(|e| format!("保存课件失败：{e}"))?;

    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            path.file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "未命名课件".into());
    // 从 <title> 里捡一个更像课件名的（文件名常常是 slug）
    let title = match extract_title(&html) {
        Some(from_html) if req.title.as_deref().unwrap_or("").trim().is_empty() => from_html,
        _ => title,
    };
    let stamp = now();
    conn.execute(
        "insert into local_courseware
           (id, title, subject_id, grade, description, knowledge, file_path, file_sha256,
            lesson_spec_json, source, version, step_count, quiz_count, size_bytes, warnings,
            created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, null, 'IMPORT', 1, 0, 0, ?9, ?10, ?11, ?11)",
        params![
            id,
            title,
            req.subject_id,
            req.grade,
            req.description,
            json_list(&req.knowledge),
            format!("courseware/{id}.html"),
            sha,
            size as i64,
            json_list(&warnings),
            stamp,
        ],
    )
    .map_err(|e| format!("写入课件库失败：{e}"))?;

    Ok(ImportResult {
        id,
        title,
        warnings,
        duplicated: false,
    })
}

fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let close = lower[open_end..].find("</title>")? + open_end;
    let title = html[open_end..close].trim();
    if title.is_empty() || title.chars().count() > 40 {
        None
    } else {
        Some(title.to_string())
    }
}

// ---------------------------------------------------------------------------
// 查询 / 维护
// ---------------------------------------------------------------------------

pub fn list(
    conn: &Connection,
    keyword: Option<&str>,
    subject_id: Option<&str>,
    source: Option<&str>,
) -> Result<Vec<Courseware>, String> {
    let mut stmt = conn
        .prepare(
            "select c.id, c.title, c.subject_id, coalesce(s.name, ''), c.grade, c.description,
                    c.knowledge, c.source, c.version, c.step_count, c.quiz_count, c.size_bytes,
                    c.warnings, c.lesson_spec_json is not null, c.created_at, c.updated_at,
                    (select count(*) from local_courseware_question q where q.courseware_id = c.id)
               from local_courseware c
               left join subject s on s.id = c.subject_id
              where c.deleted_at is null
                and (?1 is null or c.title like '%' || ?1 || '%'
                     or c.description like '%' || ?1 || '%'
                     or c.knowledge like '%' || ?1 || '%')
                and (?2 is null or c.subject_id = ?2)
                and (?3 is null or c.source = ?3)
              order by c.updated_at desc",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![keyword, subject_id, source], |row| {
            Ok(Courseware {
                id: row.get(0)?,
                title: row.get(1)?,
                subject_id: row.get(2)?,
                subject_name: row.get(3)?,
                grade: row.get(4)?,
                description: row.get(5)?,
                knowledge: parse_list(&row.get::<_, String>(6)?),
                source: row.get(7)?,
                version: row.get(8)?,
                step_count: row.get(9)?,
                quiz_count: row.get(10)?,
                size_bytes: row.get(11)?,
                warnings: parse_list(&row.get::<_, String>(12)?),
                has_spec: row.get(13)?,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
                question_count: row.get(16)?,
                classes: Vec::new(),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut assigned: HashMap<String, Vec<AssignedClass>> = HashMap::new();
    let mut stmt = conn
        .prepare(
            "select cc.courseware_id, c.id, c.name, cc.due_at
               from local_class_courseware cc
               join local_class c on c.id = cc.class_id
              where c.deleted_at is null
              order by cc.assigned_at desc",
        )
        .map_err(|e| e.to_string())?;
    let links = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                AssignedClass {
                    id: row.get(1)?,
                    name: row.get(2)?,
                    due_at: row.get(3)?,
                },
            ))
        })
        .map_err(|e| e.to_string())?;
    for link in links {
        let (courseware_id, class) = link.map_err(|e| e.to_string())?;
        assigned.entry(courseware_id).or_default().push(class);
    }

    Ok(rows
        .into_iter()
        .map(|mut row| {
            row.classes = assigned.remove(&row.id).unwrap_or_default();
            row
        })
        .collect())
}

pub fn file_of(conn: &Connection, id: &str) -> Result<Option<PathBuf>, String> {
    conn.query_row(
        "select file_path from local_courseware where id = ?1 and deleted_at is null",
        [id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| e.to_string())
    .map(|value| value.map(PathBuf::from))
}

pub fn title_of(conn: &Connection, id: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "select title from local_courseware where id = ?1 and deleted_at is null",
        [id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

pub fn warnings_of(conn: &Connection, id: &str) -> Vec<String> {
    conn.query_row(
        "select warnings from local_courseware where id = ?1",
        [id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .map(|raw| parse_list(&raw))
    .unwrap_or_default()
}

pub fn update_meta(conn: &Connection, patch: &CoursewarePatch) -> Result<(), String> {
    let title = patch.title.trim();
    if title.is_empty() {
        return Err("课件名不能为空".into());
    }
    let changed = conn
        .execute(
            "update local_courseware
                set title = ?2, subject_id = ?3, grade = ?4, description = ?5,
                    knowledge = ?6, updated_at = ?7
              where id = ?1 and deleted_at is null",
            params![
                patch.id,
                title,
                patch.subject_id,
                patch.grade,
                patch.description,
                json_list(&patch.knowledge),
                now()
            ],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("课件不存在或已删除".into());
    }
    Ok(())
}

/// 软删除；文件留在磁盘上，误删还能从备份/回收站恢复。
pub fn remove(conn: &Connection, id: &str) -> Result<(), String> {
    let stamp = now();
    conn.execute(
        "update local_courseware set deleted_at = ?2, updated_at = ?2
          where id = ?1 and deleted_at is null",
        params![id, stamp],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "delete from local_class_courseware where courseware_id = ?1",
        [id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "delete from local_courseware_question where courseware_id = ?1",
        [id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn assign(conn: &Connection, req: &AssignRequest) -> Result<(), String> {
    if req.class_ids.is_empty() {
        return Err("先选至少一个班级".into());
    }
    let exists: Option<i64> = conn
        .query_row(
            "select 1 from local_courseware where id = ?1 and deleted_at is null",
            [&req.courseware_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err("课件不在了".into());
    }
    let stamp = now();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for class_id in &req.class_ids {
        tx.execute(
            "insert into local_class_courseware (class_id, courseware_id, assigned_at, due_at, note)
             values (?1, ?2, ?3, ?4, ?5)
             on conflict (class_id, courseware_id)
             do update set due_at = excluded.due_at, note = excluded.note",
            params![
                class_id,
                req.courseware_id,
                stamp,
                req.due_at,
                req.note.trim()
            ],
        )
        .map_err(|e| format!("布置到班级失败：{e}"))?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn unassign(conn: &Connection, courseware_id: &str, class_id: &str) -> Result<(), String> {
    conn.execute(
        "delete from local_class_courseware where courseware_id = ?1 and class_id = ?2",
        params![courseware_id, class_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn spec_of(conn: &Connection, id: &str) -> Result<Option<serde_json::Value>, String> {
    let raw: Option<String> = conn
        .query_row(
            "select lesson_spec_json from local_courseware where id = ?1 and deleted_at is null",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(raw.and_then(|text| serde_json::from_str(&text).ok()))
}

pub fn export(
    conn: &Connection,
    data_dir: &Path,
    id: &str,
    kind: &str,
    target: &Path,
) -> Result<ExportOutcome, String> {
    if target.exists() {
        return Err("这个位置已有同名文件，请换一个文件名，避免覆盖".into());
    }
    if target
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty() && !parent.is_dir())
    {
        return Err("导出目录不存在，请重新选择保存位置".into());
    }
    let relative = file_of(conn, id)?.ok_or("课件不在了")?;
    let html_path = data_dir.join("assets").join(relative);
    if !html_path.is_file() {
        return Err("课件 HTML 文件不在了，请重新编译后再导出".into());
    }
    let kind = kind.trim().to_ascii_lowercase();
    let spec_value = || {
        spec_of(conn, id)?
            .ok_or_else(|| "这份导入课件没有讲义草稿，只能导出 HTML 或 PDF".to_string())
    };
    let handout = || -> Result<HandoutSpec, String> {
        serde_json::from_value(spec_value()?).map_err(|e| format!("讲义草稿读取失败：{e}"))
    };
    let assets = data_dir.join("assets");
    let needs_print = match kind.as_str() {
        "html" => {
            std::fs::copy(&html_path, target).map_err(|e| format!("导出 HTML 失败：{e}"))?;
            false
        }
        "json" => {
            let text = serde_json::to_string_pretty(&spec_value()?).map_err(|e| e.to_string())?;
            std::fs::write(target, text).map_err(|e| format!("导出 JSON 失败：{e}"))?;
            false
        }
        "docx" => {
            office::export_docx(&handout()?, &assets, target)?;
            false
        }
        "pptx" => {
            office::export_pptx(&handout()?, &assets, target)?;
            false
        }
        "pdf" => !office::export_pdf(&html_path, target)?,
        _ => return Err("支持 HTML、DOCX、PPTX、PDF 和 JSON 导出".into()),
    };
    Ok(ExportOutcome {
        path: target.to_string_lossy().to_string(),
        exported: !needs_print,
        needs_print,
        message: if needs_print {
            "没有找到可用的 Chromium 浏览器，已改为系统打印；请在打印面板选择“存储为 PDF”".into()
        } else {
            format!("已导出 {}", target.display())
        },
    })
}

// ---------------------------------------------------------------------------
// 五道闸
// ---------------------------------------------------------------------------

fn problems_gate(name: &str, passed: usize, problems: &[String]) -> Gate {
    Gate {
        name: name.to_string(),
        ok: problems.is_empty(),
        detail: if problems.is_empty() {
            format!("通过（{passed} 项）")
        } else {
            problems.join("；")
        },
    }
}

/// 前四道：写盘之前就能判的内容闸。
pub fn grade(spec: &HandoutSpec) -> Vec<Gate> {
    // 1 结构与必填
    let mut problems = Vec::new();
    if spec.title.trim().is_empty() {
        problems.push("缺课题".to_string());
    }
    if spec.knowledge.is_empty() {
        problems.push("至少挂一个知识点".to_string());
    }
    if spec.points.is_empty() && spec.examples.is_empty() {
        problems.push("讲义既没有知识清单也没有例题，空讲".to_string());
    }
    if spec.points.len() > 8 {
        problems.push(format!("知识清单最多 8 条（当前 {}）", spec.points.len()));
    }
    for (index, point) in spec.points.iter().enumerate() {
        if point.name.trim().is_empty() {
            problems.push(format!("第 {} 条知识缺名称", index + 1));
        }
    }
    let g1 = problems_gate("结构与必填", spec.points.len() + 1, &problems);

    // 2 题目引用与题干
    let mut problems = Vec::new();
    let mut item_count = 0;
    let mut check_item = |item: &HandoutItem, label: &str, problems: &mut Vec<String>| {
        item_count += 1;
        let has_ref = !item.question_id.trim().is_empty();
        let has_inline = !item.stem.trim().is_empty();
        if !has_ref && !has_inline {
            problems.push(format!("{label}既没挂题库题也没写题干"));
        }
        if has_inline && item.answer.trim().is_empty() {
            problems.push(format!("{label}缺答案"));
        }
        if has_inline && item.analysis.trim().is_empty() {
            problems.push(format!("{label}缺解析"));
        }
    };
    for (index, item) in spec.examples.iter().enumerate() {
        check_item(item, &format!("例题 {}", index + 1), &mut problems);
    }
    for (index, item) in spec.variants.iter().enumerate() {
        check_item(item, &format!("变式 {}", index + 1), &mut problems);
    }
    for (index, item) in spec.challenge.iter().enumerate() {
        check_item(item, &format!("拔高 {}", index + 1), &mut problems);
    }
    for (index, item) in spec.thinking.iter().enumerate() {
        if item.prompt.trim().is_empty() {
            problems.push(format!("思维训练 {} 缺题目", index + 1));
        }
    }
    let g2 = problems_gate("题目引用与题干", item_count, &problems);

    // 3 篇幅与体量（一节课 6–10 段为宜，这里给软约束）
    let mut problems = Vec::new();
    let total = spec.points.len()
        + spec.examples.len()
        + spec.variants.len()
        + spec.challenge.len()
        + spec.thinking.len();
    if total > 20 {
        problems.push(format!("讲义段落 {total} 段偏多（建议 ≤20）"));
    }
    if spec.title.chars().count() > 40 {
        problems.push("课题超过 40 字".into());
    }
    for (index, item) in spec
        .examples
        .iter()
        .chain(spec.variants.iter())
        .chain(spec.challenge.iter())
        .enumerate()
    {
        if item.stem.chars().count() > 600 {
            problems.push(format!("第 {} 题题干超过 600 字", index + 1));
        }
        if item.steps.len() > 8 {
            problems.push(format!("第 {} 题解题步骤超过 8 步", index + 1));
        }
    }
    let g3 = problems_gate("篇幅与体量", total, &problems);

    // 4 打印可用（内容层面的判据；CSS 层面在产物闸）
    let mut problems = Vec::new();
    if spec
        .points
        .iter()
        .any(|point| point.detail.chars().count() > 1500 || point.summary.chars().count() > 200)
    {
        problems.push("有知识点讲解过长（单条摘要 ≤200 字、详解 ≤1500 字）".into());
    }
    if spec.homework.len() > 10 {
        problems.push(format!("作业最多 10 条（当前 {}）", spec.homework.len()));
    }
    if spec.objectives.len() > 6 {
        problems.push(format!(
            "教学目标最多 6 条（当前 {}）",
            spec.objectives.len()
        ));
    }
    let g4 = problems_gate("打印可用", 3, &problems);

    vec![g1, g2, g3, g4]
}

/// 第五道：产物自检（体积、无外链、运行时锚点、A4 打印样式齐全）。
pub fn gate_artifact(html: &str) -> Gate {
    let mut problems = Vec::new();
    if html.len() < MIN_OUTPUT_BYTES {
        problems.push(format!("产物只有 {} 字节，疑似被截断", html.len()));
    }
    if html.contains("http://") || html.contains("https://") {
        problems.push("产物里出现外链，断网会缺资源".into());
    }
    if !html.contains(render::MARKER) {
        problems.push("找不到运行时数据（模板没写进去）".into());
    }
    if !html.contains("<style>") || !html.contains("</script>") {
        problems.push("产物结构不完整".into());
    }
    if !html.contains("@media print") {
        problems.push("缺少打印样式，导出 PDF 会乱页".into());
    }
    Gate {
        name: "产物自检".into(),
        ok: problems.is_empty(),
        detail: if problems.is_empty() {
            format!("通过（{} KB，可离线打开 / 打印）", html.len() / 1024)
        } else {
            problems.join("；")
        },
    }
}

// ---------------------------------------------------------------------------
// 编译落库
// ---------------------------------------------------------------------------

/// 从题库把题干/答案/解析补进讲义条目（条目没写全时），并登记挂接关系。
fn hydrate_and_link(
    conn: &Connection,
    spec: &mut HandoutSpec,
) -> Result<Vec<(String, Role)>, String> {
    let mut links: Vec<(String, Role)> = Vec::new();
    let hydrate = |item: &mut HandoutItem, role: Role, links: &mut Vec<(String, Role)>| {
        let id = item.question_id.trim().to_string();
        if id.is_empty() {
            return Ok(());
        }
        links.push((id.clone(), role));
        let row: Option<(String, String, String)> = conn
            .query_row(
                "select stem, coalesce(answer, ''), coalesce(analysis, '')
                   from question where id = ?1 and deleted_at is null",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((stem, answer, analysis)) = row {
            if item.stem.trim().is_empty() {
                item.stem = stem;
            }
            if item.answer.trim().is_empty() {
                item.answer = answer;
            }
            if item.analysis.trim().is_empty() {
                item.analysis = analysis;
            }
        }
        Ok::<(), String>(())
    };
    for item in spec.examples.iter_mut() {
        hydrate(item, Role::Example, &mut links)?;
    }
    for item in spec.variants.iter_mut() {
        hydrate(item, Role::Variant, &mut links)?;
    }
    for item in spec.challenge.iter_mut() {
        hydrate(item, Role::Challenge, &mut links)?;
    }
    Ok(links)
}

fn write_links(conn: &Connection, id: &str, links: &[(String, Role)]) -> Result<(), String> {
    conn.execute(
        "delete from local_courseware_question where courseware_id = ?1",
        [id],
    )
    .map_err(|e| e.to_string())?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut seen: HashMap<(String, &str), ()> = HashMap::new();
    for (index, (question_id, role)) in links.iter().enumerate() {
        let key = (question_id.clone(), role.as_str());
        if seen.insert(key, ()).is_some() {
            continue;
        }
        tx.execute(
            "insert or ignore into local_courseware_question
               (courseware_id, question_id, role, ord) values (?1, ?2, ?3, ?4)",
            params![id, question_id, role.as_str(), index as i64],
        )
        .map_err(|e| format!("登记题目挂接失败：{e}"))?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// 为渲染准备最终图片路径与 data URL；只有全部闸通过后才把字节写入正式目录。
fn prepare_images(
    spec: &HandoutSpec,
    data_dir: &Path,
    courseware_id: &str,
) -> Result<(HandoutSpec, Vec<(PathBuf, Vec<u8>)>, Vec<String>), String> {
    use base64::Engine;

    let assets_root = data_dir.join("assets");
    let mut prepared = spec.clone();
    let mut copies = Vec::new();
    let mut warnings = Vec::new();
    for (index, image) in prepared.images.iter_mut().enumerate() {
        match read_image(&assets_root, image) {
            Ok((bytes, mime)) => {
                let extension = match mime.as_str() {
                    "image/jpeg" => "jpg",
                    "image/png" => "png",
                    "image/gif" => "gif",
                    "image/webp" => "webp",
                    _ => "png",
                };
                let filename = format!(
                    "{:02}-{}.{}",
                    index + 1,
                    safe_component(&image.id, "图"),
                    extension
                );
                let relative = format!("courseware/{courseware_id}/images/{filename}");
                let target = assets_root.join(&relative);
                image.path = relative;
                image.data_url = format!(
                    "data:{};base64,{}",
                    mime,
                    base64::engine::general_purpose::STANDARD.encode(&bytes)
                );
                copies.push((target, bytes));
            }
            Err(error) if image.required => {
                return Err(format!("必需图片「{}」无法使用：{error}", image.id));
            }
            Err(error) => {
                warnings.push(format!("可选图片「{}」未嵌入：{error}", image.id));
                image.data_url.clear();
            }
        }
    }
    Ok((prepared, copies, warnings))
}

pub fn build(
    conn: &Connection,
    data_dir: &Path,
    spec: &mut HandoutSpec,
    existing_id: Option<&str>,
) -> Result<BuildOutcome, String> {
    let links = hydrate_and_link(conn, spec)?;
    let id = existing_id
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err("课件 id 不合法".into());
    }
    let (prepared, copies, mut preparation_warnings) = match prepare_images(spec, data_dir, &id) {
        Ok((prepared, copies, warnings)) => (prepared, copies, warnings),
        Err(error) => {
            let mut gates = grade(spec);
            gates.push(Gate {
                name: "产物自检".into(),
                ok: false,
                detail: error.clone(),
            });
            return Ok(BuildOutcome {
                ok: false,
                id: None,
                gates,
                message: error,
                warnings: Vec::new(),
            });
        }
    };
    let mut gates = grade(&prepared);
    let html = match render::render(&prepared) {
        Ok(html) => html,
        Err(error) => {
            gates.push(Gate {
                name: "产物自检".into(),
                ok: false,
                detail: error.clone(),
            });
            return Ok(BuildOutcome {
                ok: false,
                id: None,
                gates,
                message: error,
                warnings: Vec::new(),
            });
        }
    };
    gates.push(gate_artifact(&html));
    let ok = gates.iter().all(|gate| gate.ok);
    if !ok {
        let failed = gates
            .iter()
            .filter(|gate| !gate.ok)
            .map(|gate| format!("{}：{}", gate.name, gate.detail))
            .collect::<Vec<_>>()
            .join("；");
        return Ok(BuildOutcome {
            ok: false,
            id: None,
            gates,
            message: failed,
            warnings: Vec::new(),
        });
    }

    let bytes = html.as_bytes();
    let sha = hex_sha256(bytes);
    let dir = courseware_dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建课件目录失败：{e}"))?;
    for (target, bytes) in copies {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("建图片目录失败：{e}"))?;
        }
        std::fs::write(&target, bytes).map_err(|e| format!("保存课件图片失败：{e}"))?;
    }
    std::fs::write(dir.join(format!("{id}.html")), bytes)
        .map_err(|e| format!("保存课件失败：{e}"))?;

    // data URL 只用于 HTML 运行时；数据库里的草稿保留可迁移的相对路径。
    let mut final_spec = prepared;
    for image in &mut final_spec.images {
        image.data_url.clear();
    }
    *spec = final_spec;
    let subject_id = if spec.subject_id.trim().is_empty() {
        None
    } else {
        Some(spec.subject_id.trim().to_string())
    };
    let title = spec.title.trim();
    // step_count 存「讲义段落数」，quiz_count 存「题目数」，展示层按讲义口径读。
    let step_count = (spec.points.len()
        + spec.examples.len()
        + spec.variants.len()
        + spec.challenge.len()
        + spec.thinking.len()) as i64;
    let quiz_count = (spec.examples.len() + spec.variants.len() + spec.challenge.len()) as i64;
    let spec_json = serde_json::to_string(spec).map_err(|e| e.to_string())?;
    let mut warning_items = spec.source.warnings.clone();
    warning_items.append(&mut preparation_warnings);
    if spec.knowledge.is_empty() {
        warning_items.push("还没填知识点，学情里归不了档".to_string());
    }
    let warnings = json_list(&warning_items);
    let stamp = now();
    if existing_id.is_some() {
        let changed = conn
            .execute(
                "update local_courseware
                    set title = ?2, subject_id = ?3, grade = ?4, description = ?5, knowledge = ?6,
                        file_path = ?7, file_sha256 = ?8, lesson_spec_json = ?9, source = 'LOCAL_BUILD',
                        version = version + 1, step_count = ?10, quiz_count = ?11, size_bytes = ?12,
                        warnings = ?13, updated_at = ?14, deleted_at = null
                  where id = ?1",
                params![
                    id,
                    title,
                    subject_id,
                    spec.grade,
                    spec.description,
                    json_list(&spec.knowledge),
                    format!("courseware/{id}.html"),
                    sha,
                    spec_json,
                    step_count,
                    quiz_count,
                    bytes.len() as i64,
                    warnings,
                    stamp,
                ],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("要覆盖的讲义不存在".into());
        }
    } else {
        conn.execute(
            "insert into local_courseware
               (id, title, subject_id, grade, description, knowledge, file_path, file_sha256,
                lesson_spec_json, source, version, step_count, quiz_count, size_bytes, warnings,
                created_at, updated_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'LOCAL_BUILD', 1, ?10, ?11, ?12, ?13, ?14, ?14)",
            params![
                id,
                title,
                subject_id,
                spec.grade,
                spec.description,
                json_list(&spec.knowledge),
                format!("courseware/{id}.html"),
                sha,
                spec_json,
                step_count,
                quiz_count,
                bytes.len() as i64,
                warnings,
                stamp,
            ],
        )
        .map_err(|e| format!("写入讲义库失败：{e}"))?;
    }
    write_links(conn, &id, &links)?;

    Ok(BuildOutcome {
        ok: true,
        id: Some(id),
        gates,
        message: format!(
            "已生成讲义：{} 段、{} 道题、产物 {} KB",
            step_count,
            quiz_count,
            bytes.len() / 1024
        ),
        warnings: warning_items,
    })
}

// ---------------------------------------------------------------------------
// 预览：自定义协议按 id 提供 HTML，不开放整个数据目录
// ---------------------------------------------------------------------------

pub fn asset_url(id: &str) -> String {
    if cfg!(windows) {
        format!("http://nbasset.localhost/courseware/{id}")
    } else {
        format!("nbasset://localhost/courseware/{id}")
    }
}

pub fn register_preview(state: &AppState, id: &str, path: &Path) {
    if let Ok(mut map) = state.courseware_files.lock() {
        map.insert(id.to_string(), path.to_path_buf());
    }
}

/// 处理 `nbasset://localhost/courseware/<id>`：只回已登记的课件，其他一律 404。
pub fn serve(
    app: &tauri::AppHandle,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<std::borrow::Cow<'static, [u8]>> {
    use std::borrow::Cow;
    use tauri::Manager;

    let not_found = || {
        tauri::http::Response::builder()
            .status(404)
            .header(
                tauri::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )
            .body(Cow::Borrowed(&b"courseware not found"[..]))
            .expect("构造 404")
    };

    let path = request.uri().path();
    let id = match path.strip_prefix("/courseware/") {
        Some(value) if !value.is_empty() => value,
        _ => return not_found(),
    };
    let state = app.state::<AppState>();
    let file = state
        .courseware_files
        .lock()
        .ok()
        .and_then(|map| map.get(id).cloned());
    let Some(file) = file else {
        return not_found();
    };
    let Ok(bytes) = std::fs::read(&file) else {
        return not_found();
    };
    tauri::http::Response::builder()
        .status(200)
        .header(
            tauri::http::header::CONTENT_TYPE,
            "text/html; charset=utf-8",
        )
        .header("Cache-Control", "no-store")
        .body(Cow::Owned(bytes))
        .expect("构造响应")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn fresh() -> (Connection, PathBuf) {
        let dir = std::env::temp_dir().join(format!("nb-cw-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        let mut conn = db::open(&dir.join("neuralkb.db")).unwrap();
        db::migrate(&mut conn).unwrap();
        db::seed_curriculum(&mut conn).unwrap();
        (conn, dir)
    }

    fn subject_id(conn: &Connection) -> String {
        conn.query_row("select id from subject order by sort limit 1", [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn demo_spec() -> HandoutSpec {
        HandoutSpec {
            title: "热力环流与海陆风".into(),
            subject: "高中地理".into(),
            grade: "高一".into(),
            chapter: "第二章 地球上的大气".into(),
            knowledge: vec!["热力环流".into(), "海陆风".into()],
            objectives: vec![
                "说出热力环流的形成过程".into(),
                "会判断海陆风的昼夜方向".into(),
            ],
            warmup: "海边白天风从海上吹来，傍晚却反了方向——这是为什么？".into(),
            points: vec![PointSpec {
                name: "热力环流".into(),
                summary: "冷热不均 → 空气升降 → 同一水平面气压差 → 环流".into(),
                detail: "受热处空气膨胀上升，冷却处收缩下沉。".into(),
                misconception: "近地面与高空环流方向相同".into(),
                formula: "温差 → 密度差 → 气压差 → 环流".into(),
            }],
            examples: vec![HandoutItem {
                question_id: String::new(),
                stem: "白天滨海地区近地面风向是？".into(),
                answer: "从海洋吹向陆地（海风）".into(),
                analysis: "白天陆地升温快、气压低，海洋相对高压。".into(),
                steps: vec!["比较海陆升温".into(), "定气压高低".into()],
            }],
            variants: vec![HandoutItem {
                question_id: String::new(),
                stem: "夜晚滨海地区近地面风向是？".into(),
                answer: "陆风".into(),
                analysis: "夜晚陆地降温快，气压高于海面。".into(),
                steps: vec![],
            }],
            challenge: vec![],
            thinking: vec![ThinkSpec {
                prompt: "城市热岛环流会给郊区带来什么？".into(),
                hint: "想想污染物随风走".into(),
                answer: "郊区气流流向城市，可能把污染物带回城区。".into(),
            }],
            summary: "温差是环流的发动机，气压差是直接推手。".into(),
            homework: vec!["教材 P38 活动 2".into()],
            theme: "paper".into(),
            ..Default::default()
        }
    }

    #[test]
    fn spec_gates_catch_missing_fields_and_long_sections() {
        let mut spec = demo_spec();
        spec.knowledge.clear();
        spec.points.clear();
        spec.examples.clear();
        let gates = grade(&spec);
        assert!(!gates[0].ok, "空讲要拦");
        assert!(gates[0].detail.contains("知识点"));

        let mut spec = demo_spec();
        spec.examples[0].stem = String::new();
        spec.examples[0].answer = String::new();
        let gates = grade(&spec);
        assert!(!gates[1].ok);
        assert!(gates[1].detail.contains("例题 1"));

        let mut spec = demo_spec();
        spec.examples[0].analysis.clear();
        let gates = grade(&spec);
        assert!(!gates[1].ok);
        assert!(gates[1].detail.contains("缺解析"));

        let mut spec = demo_spec();
        for index in 0..25 {
            spec.thinking.push(ThinkSpec {
                prompt: format!("思考 {index}"),
                ..Default::default()
            });
        }
        let gates = grade(&spec);
        assert!(!gates[2].ok, "段落过多要拦");
    }

    #[test]
    fn build_writes_self_contained_html_and_links_questions() {
        let (conn, dir) = fresh();
        // 先在题库里放一道题，讲义用 questionId 挂上
        let subject = subject_id(&conn);
        let point: String = conn
            .query_row(
                "select id from knowledge_point where subject_id = ?1 limit 1",
                [&subject],
                |row| row.get(0),
            )
            .unwrap();
        let qid = "q-handout-1".to_string();
        conn.execute(
            "insert into question (id, subject_id, type, difficulty, stem, answer, analysis, status, created_at, updated_at)
             values (?1, ?2, 'SINGLE', 3, '白天风从哪来？', '海', '陆地热低压。', 'PUBLISHED', '2026-01-01T00:00:00+08:00', '2026-01-01T00:00:00+08:00')",
            params![qid, subject],
        )
        .unwrap();
        conn.execute(
            "insert into question_knowledge (question_id, knowledge_point_id) values (?1, ?2)",
            params![qid, point],
        )
        .unwrap();

        let mut spec = demo_spec();
        spec.examples[0] = HandoutItem {
            question_id: qid.clone(),
            ..Default::default()
        };
        let first = build(&conn, &dir, &mut spec, None).unwrap();
        assert!(first.ok, "{}", first.message);
        assert_eq!(first.gates.len(), 5);
        assert!(first.gates.iter().all(|gate| gate.ok), "{:?}", first.gates);
        let id = first.id.unwrap();

        let file = dir
            .join("assets")
            .join("courseware")
            .join(format!("{id}.html"));
        let html = std::fs::read_to_string(&file).unwrap();
        assert!(html.contains(render::MARKER));
        assert!(!html.contains("http://") && !html.contains("https://"));
        assert!(html.contains("热力环流"));
        assert!(html.contains("白天风从哪来？"), "题库题干要内联进讲义");
        assert!(html.contains("陆地热低压。"), "解析也要内联");
        assert!(html.contains("@media print"));
        assert!(html.len() > MIN_OUTPUT_BYTES);

        let row: (i64, i64, i64, String) = conn
            .query_row(
                "select step_count, quiz_count, version, source from local_courseware where id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row.0, 4, "1 知识 + 1 例 + 1 变式 + 1 思维");
        assert_eq!(row.1, 2, "例题 + 变式");
        assert_eq!(row.2, 1);
        assert_eq!(row.3, "LOCAL_BUILD");

        let linked: (String, String) = conn
            .query_row(
                "select question_id, role from local_courseware_question where courseware_id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(linked.0, qid);
        assert_eq!(linked.1, "EXAMPLE");

        let mut spec2 = spec.clone();
        spec2.title = "热力环流与海陆风（修订）".into();
        let second = build(&conn, &dir, &mut spec2, Some(&id)).unwrap();
        assert!(second.ok);
        assert_eq!(second.id.as_deref(), Some(id.as_str()));
        let (title, version): (String, i64) = conn
            .query_row(
                "select title, version from local_courseware where id = ?1",
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(title, "热力环流与海陆风（修订）");
        assert_eq!(version, 2);
    }

    /// 主路径真实文件冒烟：从同一份已编译草稿导出所有格式，并实际读取产物。
    #[test]
    #[ignore = "需要本机 Chromium，做主路径真实文件冒烟时单独运行"]
    fn export_all_formats_from_same_built_spec_with_image() {
        use std::io::Read as _;

        let (conn, dir) = fresh();
        let source = dir.join("原图.png");
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            12,
            8,
            image::Rgb([30, 120, 200]),
        ))
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
        std::fs::write(&source, encoded.into_inner()).unwrap();

        let mut image = add_image(&dir, "draft-smoke", "图1", &source.to_string_lossy()).unwrap();
        image.caption = "海陆风示意图".into();
        image.placement = "新知讲解后".into();
        image.required = true;
        assert_eq!(
            draft_image_dir(&dir, "draft-smoke").unwrap(),
            dir.join("assets/courseware/drafts/draft-smoke")
        );
        let mut spec = demo_spec();
        spec.images = vec![image];
        let built = build(&conn, &dir, &mut spec, None).unwrap();
        assert!(built.ok, "{}", built.message);
        let id = built.id.unwrap();
        let exports = dir.join("exports");
        std::fs::create_dir_all(&exports).unwrap();

        let html_path = exports.join("讲义.html");
        let html_outcome = export(&conn, &dir, &id, "html", &html_path).unwrap();
        assert!(html_outcome.exported);
        let html = std::fs::read_to_string(&html_path).unwrap();
        assert!(html.contains("热力环流与海陆风"));
        assert!(html.contains("白天滨海地区近地面风向是？"));
        assert!(html.contains("data:image/png;base64,"));
        assert!(html.contains("海陆风示意图"));
        assert!(!html.contains(source.to_string_lossy().as_ref()));
        let overwrite = export(&conn, &dir, &id, "html", &html_path).unwrap_err();
        assert!(overwrite.contains("同名文件"), "{overwrite}");

        let json_path = exports.join("讲义.json");
        export(&conn, &dir, &id, "json", &json_path).unwrap();
        let exported_spec: HandoutSpec =
            serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
        assert_eq!(exported_spec.title, spec.title);
        assert_eq!(exported_spec.images.len(), 1);
        assert_eq!(exported_spec.images[0].caption, "海陆风示意图");

        let docx_path = exports.join("讲义.docx");
        export(&conn, &dir, &id, "docx", &docx_path).unwrap();
        let mut docx = zip::ZipArchive::new(std::fs::File::open(&docx_path).unwrap()).unwrap();
        let mut document = String::new();
        docx.by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut document)
            .unwrap();
        assert!(document.contains("热力环流与海陆风"));
        assert!(document.contains("白天滨海地区近地面风向是？"));
        assert!(document.contains("海陆风示意图"));
        assert!(docx.by_name("word/media/image1.png").is_ok());

        let pptx_path = exports.join("讲义.pptx");
        export(&conn, &dir, &id, "pptx", &pptx_path).unwrap();
        let mut pptx = zip::ZipArchive::new(std::fs::File::open(&pptx_path).unwrap()).unwrap();
        let mut slides = String::new();
        for index in 1..=pptx.len() {
            let name = format!("ppt/slides/slide{index}.xml");
            let Ok(mut entry) = pptx.by_name(&name) else {
                continue;
            };
            entry.read_to_string(&mut slides).unwrap();
        }
        assert!(slides.contains("热力环流与海陆风"));
        assert!(slides.contains("白天滨海地区近地面风向是？"));
        assert!(slides.contains("海陆风示意图"));
        assert!(pptx.by_name("ppt/media/image1.png").is_ok());

        let pdf_path = exports.join("讲义.pdf");
        let pdf = export(&conn, &dir, &id, "pdf", &pdf_path).unwrap();
        assert!(
            pdf.exported,
            "真实文件冒烟需要本机 Chromium：{}",
            pdf.message
        );
        let bytes = std::fs::read(&pdf_path).unwrap();
        assert!(bytes.len() > 500);
        assert_eq!(bytes.get(..5), Some(&b"%PDF-"[..]));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_refuses_invalid_spec_and_writes_nothing() {
        let (conn, dir) = fresh();
        let mut spec = demo_spec();
        spec.title = "  ".into();
        let outcome = build(&conn, &dir, &mut spec, None).unwrap();
        assert!(!outcome.ok);
        assert!(outcome.id.is_none());
        assert!(outcome.message.contains("结构与必填"));
        let count: i64 = conn
            .query_row("select count(*) from local_courseware", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
        assert!(!dir.join("assets/courseware").exists());
    }

    #[test]
    fn import_rejects_external_script_and_dedupes_by_hash() {
        let (conn, dir) = fresh();
        let bad = dir.join("bad.html");
        std::fs::write(
            &bad,
            "<!doctype html><html><head><script src=\"https://cdn.example/x.js\"></script></head><body>hi</body></html>",
        )
        .unwrap();
        let req = ImportRequest {
            path: bad.to_string_lossy().to_string(),
            title: None,
            subject_id: None,
            grade: String::new(),
            description: String::new(),
            knowledge: vec![],
        };
        let error = import(&conn, &dir, &req).unwrap_err();
        assert!(error.contains("外部脚本"), "{error}");

        let good = dir.join("good.html");
        std::fs::write(
            &good,
            "<!doctype html><html><head><title>抛体运动</title></head><body><h1>hi</h1></body></html>",
        )
        .unwrap();
        let req = ImportRequest {
            path: good.to_string_lossy().to_string(),
            ..req
        };
        let first = import(&conn, &dir, &req).unwrap();
        assert!(!first.duplicated);
        assert_eq!(first.title, "抛体运动", "优先用 <title> 当课件名");
        let again = import(&conn, &dir, &req).unwrap();
        assert!(again.duplicated);
        assert_eq!(again.id, first.id);

        let id = first.id;
        let rel = file_of(&conn, &id).unwrap().unwrap();
        let path = dir.join("assets").join(rel);
        assert!(path.exists());
        assert_eq!(title_of(&conn, &id).unwrap().as_deref(), Some("抛体运动"));
    }

    #[test]
    fn assign_and_unassign_show_up_in_list() {
        let (conn, dir) = fresh();
        let subject = subject_id(&conn);
        // 讲义挂上真实学科，列表按学科筛才筛得到
        let mut spec = demo_spec();
        spec.subject_id = subject.clone();
        let id = build(&conn, &dir, &mut spec, None).unwrap().id.unwrap();
        conn.execute(
            "insert into local_class (id, name, subject_id, term, join_code, created_at, updated_at)
             values ('c1', '高一(3)班', ?1, '2026 秋', 'ABC234', '2026-09-01T00:00:00+08:00', '2026-09-01T00:00:00+08:00')",
            [&subject],
        )
        .unwrap();

        assign(
            &conn,
            &AssignRequest {
                courseware_id: id.clone(),
                class_ids: vec!["c1".into()],
                due_at: Some("2026-11-20T18:00".into()),
                note: "课前看".into(),
            },
        )
        .unwrap();

        let rows = list(&conn, Some("热力环流"), Some(&subject), Some("LOCAL_BUILD")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].classes.len(), 1);
        assert_eq!(rows[0].classes[0].name, "高一(3)班");
        assert_eq!(
            rows[0].classes[0].due_at.as_deref(),
            Some("2026-11-20T18:00")
        );
        assert_eq!(rows[0].step_count, 4);
        assert!(rows[0].has_spec);

        assert!(list(&conn, Some("热力学"), None, None).unwrap().is_empty());

        unassign(&conn, &id, "c1").unwrap();
        let rows = list(&conn, None, None, None).unwrap();
        assert!(rows[0].classes.is_empty());

        assign(
            &conn,
            &AssignRequest {
                courseware_id: id.clone(),
                class_ids: vec!["c1".into()],
                due_at: None,
                note: String::new(),
            },
        )
        .unwrap();
        remove(&conn, &id).unwrap();
        assert!(list(&conn, None, None, None).unwrap().is_empty());
        let left: i64 = conn
            .query_row("select count(*) from local_class_courseware", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn update_meta_requires_title_and_keeps_file() {
        let (conn, dir) = fresh();
        let id = build(&conn, &dir, &mut demo_spec(), None)
            .unwrap()
            .id
            .unwrap();
        update_meta(
            &conn,
            &CoursewarePatch {
                id: id.clone(),
                title: "热力环流（课堂版）".into(),
                subject_id: None,
                grade: "高一".into(),
                description: "改了简介".into(),
                knowledge: vec!["热力环流".into(), "大气受热".into()],
            },
        )
        .unwrap();
        let row = &list(&conn, None, None, None).unwrap()[0];
        assert_eq!(row.title, "热力环流（课堂版）");
        assert_eq!(row.knowledge.len(), 2);
        assert!(file_of(&conn, &id).unwrap().is_some());

        let error = update_meta(
            &conn,
            &CoursewarePatch {
                id: id.clone(),
                title: "   ".into(),
                subject_id: None,
                grade: String::new(),
                description: String::new(),
                knowledge: vec![],
            },
        )
        .unwrap_err();
        assert!(error.contains("不能为空"));
    }

    #[test]
    fn inspect_flags_remote_assets_as_warning_only() {
        let html = "<!doctype html><html><body><img src=\"https://x/y.png\"><script>fetch('/a')</script></body></html>";
        let warnings = inspect_html(html).unwrap();
        assert!(warnings.iter().any(|item| item.contains("网络地址")));
        assert!(warnings.iter().any(|item| item.contains("fetch")));
        assert!(inspect_html("not html").is_err());
        assert!(inspect_html("<!doctype html><html></html>")
            .unwrap()
            .is_empty());
    }
    /// 只为人工看效果：跑 `cargo test dump_handout_preview` 会把演示讲义写到 /tmp。
    #[test]
    fn dump_handout_preview() {
        let html = render::render(&demo_spec()).unwrap();
        std::fs::write("/tmp/nb-handout-preview.html", html).unwrap();
    }
}
