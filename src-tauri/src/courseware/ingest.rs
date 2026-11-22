//! 已有课件的批量导入任务：受管原稿 → 本地抽取 → AI/本地草稿 → 教师校对 → 统一入库。

use super::{source, BuildOutcome, HandoutSpec, PointSpec, SourceTrace};
use crate::{db, llm};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

const ITEM_COLUMNS: &str = "id, batch_id, source_path, managed_path, sha256, title, status, mode,
    use_ai, subject_id, subject, grade, extracted_text, draft_json, raw_ai_json,
    warnings_json, locations_json, error, courseware_id, attempts, created_at, updated_at";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStartInput {
    pub paths: Vec<String>,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub subject_id: Option<String>,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub grade: String,
    #[serde(default)]
    pub use_ai: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItem {
    pub id: String,
    pub batch_id: String,
    pub source_path: String,
    pub managed_path: String,
    pub sha256: String,
    pub title: String,
    pub status: String,
    pub mode: String,
    pub use_ai: bool,
    pub subject_id: Option<String>,
    pub subject: String,
    pub grade: String,
    pub extracted_text: String,
    pub draft: Option<HandoutSpec>,
    pub raw_ai_json: Option<String>,
    pub warnings: Vec<String>,
    pub locations: Vec<String>,
    pub error: String,
    pub courseware_id: Option<String>,
    pub attempts: i64,
    pub created_at: String,
    pub updated_at: String,
}

fn now() -> String {
    chrono::Local::now().to_rfc3339()
}

fn strings(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn json_strings(values: &[String]) -> Result<String, String> {
    serde_json::to_string(values).map_err(|e| e.to_string())
}

fn normalize_mode(raw: &str) -> Result<String, String> {
    let mode = raw.trim().to_ascii_uppercase();
    match mode.as_str() {
        "" | "FORMAT_ONLY" => Ok("FORMAT_ONLY".into()),
        "OPTIMIZE" => Ok("OPTIMIZE".into()),
        _ => Err("整理模式只支持 FORMAT_ONLY 或 OPTIMIZE".into()),
    }
}

fn file_title(path: &Path) -> String {
    path.file_stem()
        .map(|value| value.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "未命名课件".into())
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<ImportItem> {
    let draft_json = row.get::<_, Option<String>>(13)?;
    Ok(ImportItem {
        id: row.get(0)?,
        batch_id: row.get(1)?,
        source_path: row.get(2)?,
        managed_path: row.get(3)?,
        sha256: row.get(4)?,
        title: row.get(5)?,
        status: row.get(6)?,
        mode: row.get(7)?,
        use_ai: row.get::<_, i64>(8)? != 0,
        subject_id: row.get(9)?,
        subject: row.get(10)?,
        grade: row.get(11)?,
        extracted_text: row.get(12)?,
        draft: draft_json.and_then(|raw| serde_json::from_str(&raw).ok()),
        raw_ai_json: row.get(14)?,
        warnings: strings(&row.get::<_, String>(15)?),
        locations: strings(&row.get::<_, String>(16)?),
        error: row.get(17)?,
        courseware_id: row.get(18)?,
        attempts: row.get(19)?,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}

fn get(conn: &Connection, id: &str) -> Result<ImportItem, String> {
    conn.query_row(
        &format!("select {ITEM_COLUMNS} from local_courseware_import where id = ?1"),
        [id],
        from_row,
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "课件导入任务不存在".into())
}

/// 返回受管原稿的绝对路径，仅供本机打开；数据库仍只保存相对路径。
pub fn original_path(conn: &Connection, data_dir: &Path, id: &str) -> Result<String, String> {
    let item = get(conn, id)?;
    if item.managed_path.is_empty() {
        return Err("原稿尚未保存，请先重试这个导入任务".into());
    }
    let path = managed_absolute(data_dir, &item.managed_path)?;
    if !path.is_file() {
        return Err("受管原稿不在了，请重新导入原文件".into());
    }
    Ok(path.to_string_lossy().to_string())
}

/// 只建立批次与逐文件任务，不在 UI 调用线程里读取或抽取文件。
pub fn start(
    conn: &Connection,
    _data_dir: &Path,
    input: &ImportStartInput,
) -> Result<Vec<ImportItem>, String> {
    if input.paths.is_empty() {
        return Err("请先选择至少一个讲义文件".into());
    }
    if input.paths.iter().any(|path| path.trim().is_empty()) {
        return Err("选中的文件里有空路径，请重新选择".into());
    }
    let mode = normalize_mode(&input.mode)?;
    let batch_id = uuid::Uuid::new_v4().to_string();
    let stamp = now();
    let subject_id = input
        .subject_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for source_path in &input.paths {
        let id = uuid::Uuid::new_v4().to_string();
        let source_path = source_path.trim();
        tx.execute(
            "insert into local_courseware_import
               (id, batch_id, source_path, title, status, mode, use_ai,
                subject_id, subject, grade, created_at, updated_at)
             values (?1, ?2, ?3, ?4, 'QUEUED', ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                id,
                batch_id,
                source_path,
                file_title(Path::new(source_path)),
                mode,
                input.use_ai as i64,
                subject_id,
                input.subject.trim(),
                input.grade.trim(),
                stamp,
            ],
        )
        .map_err(|e| format!("创建课件导入任务失败：{e}"))?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    list(conn, Some(&batch_id))
}

/// 批次内按插入顺序返回；不传批次时返回全部任务。
pub fn list(conn: &Connection, batch_id: Option<&str>) -> Result<Vec<ImportItem>, String> {
    let (where_sql, value) = match batch_id {
        Some(id) => (" where batch_id = ?1", Some(id)),
        None => ("", None),
    };
    let mut stmt = conn
        .prepare(&format!(
            "select {ITEM_COLUMNS} from local_courseware_import{where_sql} order by created_at, rowid"
        ))
        .map_err(|e| e.to_string())?;
    let rows = if let Some(value) = value {
        stmt.query_map([value], from_row)
    } else {
        stmt.query_map([], from_row)
    }
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// 把失败、重复或待校对项重新排队。抽取文字和受管原稿均保留。
pub fn retry(
    conn: &Connection,
    id: &str,
    use_ai: bool,
    allow_duplicate: bool,
) -> Result<ImportItem, String> {
    let item = get(conn, id)?;
    if item.status == "DUPLICATE" && !allow_duplicate {
        return Err("这是重复文件；确认仍需导入时，请允许重复后再重试".into());
    }
    if !matches!(item.status.as_str(), "FAILED" | "DUPLICATE" | "REVIEW") {
        return Err("只有失败、重复或待校对的课件可以重试".into());
    }
    conn.execute(
        "update local_courseware_import
            set status = 'QUEUED', use_ai = ?2, error = '', updated_at = ?3
          where id = ?1",
        params![id, use_ai as i64, now()],
    )
    .map_err(|e| e.to_string())?;
    get(conn, id)
}

fn safe_file_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "source".into());
    let cleaned = raw
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let cleaned = cleaned.trim_matches(|ch| matches!(ch, '.' | '-' | '_'));
    if cleaned.is_empty() {
        "source".into()
    } else {
        cleaned.into()
    }
}

fn managed_absolute(data_dir: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("受管原稿路径不安全，请重新导入".into());
    }
    Ok(data_dir.join("assets").join(path))
}

fn read_source(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("读不到这个讲义文件：{e}"))?;
    if !metadata.is_file() {
        return Err("请选择讲义文件，不要选择文件夹".into());
    }
    if metadata.len() == 0 {
        return Err("这个讲义文件是空的".into());
    }
    if metadata.len() > super::MAX_BYTES {
        return Err(format!(
            "讲义文件超过 {} MB，请先精简或拆分后再导入",
            super::MAX_BYTES / 1024 / 1024
        ));
    }
    std::fs::read(path).map_err(|e| format!("读讲义文件失败：{e}"))
}

fn preserve_original(
    conn: &Connection,
    data_dir: &Path,
    item: &ImportItem,
) -> Result<(String, String, PathBuf), String> {
    if !item.managed_path.is_empty() {
        let absolute = managed_absolute(data_dir, &item.managed_path)?;
        let bytes = read_source(&absolute)?;
        let sha = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        return Ok((item.managed_path.clone(), sha, absolute));
    }

    let original = Path::new(&item.source_path);
    let bytes = read_source(original)?;
    let sha = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let relative = format!(
        "courseware/imports/{}/{}",
        item.id,
        safe_file_name(original)
    );
    let absolute = managed_absolute(data_dir, &relative)?;
    let parent = absolute
        .parent()
        .ok_or_else(|| "受管原稿目录不可用".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| format!("建原稿目录失败：{e}"))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&absolute)
        .map_err(|e| format!("保存受管原稿失败：{e}"))?;
    if let Err(error) = file.write_all(&bytes) {
        drop(file);
        let _ = std::fs::remove_file(&absolute);
        return Err(format!("保存受管原稿失败：{error}"));
    }
    conn.execute(
        "update local_courseware_import
            set managed_path = ?2, sha256 = ?3, updated_at = ?4 where id = ?1",
        params![item.id, relative, sha, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok((relative, sha, absolute))
}

fn local_draft(
    item: &ImportItem,
    managed_path: &str,
    sha: &str,
    extracted: &source::ExtractedSource,
) -> HandoutSpec {
    let title = file_title(Path::new(&item.source_path));
    HandoutSpec {
        title: title.clone(),
        subject_id: item.subject_id.clone().unwrap_or_default(),
        subject: item.subject.clone(),
        grade: item.grade.clone(),
        points: vec![PointSpec {
            name: title,
            detail: extracted.text.clone(),
            ..Default::default()
        }],
        source: SourceTrace {
            original_path: managed_path.to_string(),
            sha256: sha.to_string(),
            locations: extracted.locations.clone(),
            mode: item.mode.clone(),
            warnings: extracted.warnings.clone(),
        },
        ..Default::default()
    }
}

fn save_review(
    conn: &Connection,
    item: &ImportItem,
    source: &source::ExtractedSource,
    spec: &HandoutSpec,
    raw_ai: Option<&str>,
    error: String,
) -> Result<ImportItem, String> {
    let draft = serde_json::to_string(spec).map_err(|e| e.to_string())?;
    conn.execute(
        "update local_courseware_import
            set status = 'REVIEW', extracted_text = ?2, draft_json = ?3,
                raw_ai_json = coalesce(?4, raw_ai_json), warnings_json = ?5,
                locations_json = ?6, error = ?7, updated_at = ?8
          where id = ?1",
        params![
            item.id,
            source.text,
            draft,
            raw_ai,
            json_strings(&source.warnings)?,
            json_strings(&source.locations)?,
            error,
            now(),
        ],
    )
    .map_err(|e| e.to_string())?;
    get(conn, &item.id)
}

fn process_current(
    conn: &Connection,
    data_dir: &Path,
    http: &reqwest::blocking::Client,
    item: &ImportItem,
    allow_duplicate: bool,
    cooldowns: &mut HashMap<String, Instant>,
) -> Result<ImportItem, String> {
    let (managed_path, sha, absolute) = preserve_original(conn, data_dir, item)?;
    if !allow_duplicate {
        let duplicate: Option<String> = conn
            .query_row(
                "select id from local_courseware_import
                  where sha256 = ?1 and id <> ?2 limit 1",
                params![sha, item.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if duplicate.is_some() {
            conn.execute(
                "update local_courseware_import
                    set status = 'DUPLICATE',
                        error = '发现相同内容的导入记录，已跳过；确认仍需导入时请允许重复',
                        updated_at = ?2 where id = ?1",
                params![item.id, now()],
            )
            .map_err(|e| e.to_string())?;
            return get(conn, &item.id);
        }
    }

    // 首次抽取的文字、位置与警告保留下来；AI 重试不再读取老师可能已移动的原文件。
    let extracted = if item.extracted_text.trim().is_empty() {
        source::extract(&absolute, data_dir)?
    } else {
        source::ExtractedSource {
            text: item.extracted_text.clone(),
            warnings: item.warnings.clone(),
            locations: item.locations.clone(),
        }
    };
    let base = local_draft(item, &managed_path, &sha, &extracted);
    if !item.use_ai {
        return save_review(conn, item, &extracted, &base, None, String::new());
    }

    let messages = super::prompt::import_messages(&base, &extracted);
    match llm::complete(
        conn,
        http,
        "courseware_import",
        &messages,
        None,
        true,
        8192,
        cooldowns,
    ) {
        Ok(raw) => match super::prompt::parse_draft(&raw, &base, "课件导入模型") {
            Ok(spec) => save_review(conn, item, &extracted, &spec, Some(&raw), String::new()),
            Err(error) => save_review(
                conn,
                item,
                &extracted,
                &base,
                Some(&raw),
                format!("AI 返回的草稿不可用，已保留本地草稿：{error}"),
            ),
        },
        Err(error) => save_review(
            conn,
            item,
            &extracted,
            &base,
            None,
            format!("AI 整理失败，已保留本地草稿：{error}"),
        ),
    }
}

fn process_with_conn(
    conn: &Connection,
    data_dir: &Path,
    http: &reqwest::blocking::Client,
    id: &str,
    allow_duplicate: bool,
    cooldowns: &mut HashMap<String, Instant>,
) -> Result<ImportItem, String> {
    let item = get(conn, id)?;
    if item.status == "DUPLICATE" && !allow_duplicate {
        return Ok(item);
    }
    if item.status != "QUEUED" && !(item.status == "DUPLICATE" && allow_duplicate) {
        return Err("这项不在待处理队列；失败或待校对项请先重试".into());
    }
    conn.execute(
        "update local_courseware_import
            set status = 'EXTRACTING', attempts = attempts + 1, error = '', updated_at = ?2
          where id = ?1",
        params![id, now()],
    )
    .map_err(|e| e.to_string())?;

    match process_current(conn, data_dir, http, &item, allow_duplicate, cooldowns) {
        Ok(item) => Ok(item),
        Err(error) => {
            conn.execute(
                "update local_courseware_import
                    set status = 'FAILED', error = ?2, updated_at = ?3 where id = ?1",
                params![id, error, now()],
            )
            .map_err(|e| e.to_string())?;
            get(conn, id)
        }
    }
}

/// 后台线程处理单项；数据库连接在本函数内打开，不持有 Tauri State。
pub fn process_one(
    db_path: &Path,
    data_dir: &Path,
    http: &reqwest::blocking::Client,
    id: &str,
    allow_duplicate: bool,
) -> Result<ImportItem, String> {
    let conn = db::open(db_path)?;
    let mut cooldowns = HashMap::new();
    process_with_conn(&conn, data_dir, http, id, allow_duplicate, &mut cooldowns)
}

/// 后台线程顺序处理一个批次，单项失败只落在该项上，模型冷却时间在整批共享。
pub fn process_batch(
    db_path: &Path,
    data_dir: &Path,
    http: &reqwest::blocking::Client,
    batch_id: &str,
) -> Result<Vec<ImportItem>, String> {
    let conn = db::open(db_path)?;
    let mut stmt = conn
        .prepare(
            "select id from local_courseware_import
              where batch_id = ?1 and status = 'QUEUED' order by created_at, rowid",
        )
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([batch_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    let mut cooldowns = HashMap::new();
    let mut errors = Vec::new();
    for id in ids {
        if let Err(error) = process_with_conn(&conn, data_dir, http, &id, false, &mut cooldowns) {
            errors.push(format!("{id}：{error}"));
        }
    }
    let items = list(&conn, Some(batch_id))?;
    if errors.is_empty() {
        Ok(items)
    } else {
        Err(errors.join("\n"))
    }
}

fn restore_trace(spec: &mut HandoutSpec, item: &ImportItem) -> Result<(), String> {
    if item.managed_path.is_empty() || item.sha256.is_empty() {
        return Err("原稿尚未保存，不能校对这个课件".into());
    }
    // 防止校对请求把来源改成绝对路径或 assets 外路径。
    managed_absolute(Path::new(""), &item.managed_path)?;
    spec.source = SourceTrace {
        original_path: item.managed_path.clone(),
        sha256: item.sha256.clone(),
        locations: item.locations.clone(),
        mode: item.mode.clone(),
        warnings: item.warnings.clone(),
    };
    Ok(())
}

fn store_teacher_draft(
    conn: &Connection,
    item: &ImportItem,
    spec: &HandoutSpec,
) -> Result<(), String> {
    let draft = serde_json::to_string(spec).map_err(|e| e.to_string())?;
    conn.execute(
        "update local_courseware_import
            set status = 'REVIEW', title = ?2, draft_json = ?3, error = '', updated_at = ?4
          where id = ?1",
        params![item.id, spec.title.trim(), draft, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 保存老师校对，不触发入库。
pub fn save_draft(conn: &Connection, id: &str, spec: &HandoutSpec) -> Result<ImportItem, String> {
    let item = get(conn, id)?;
    if item.status != "REVIEW" {
        return Err("只有待校对的课件可以保存草稿".into());
    }
    let mut spec = spec.clone();
    restore_trace(&mut spec, &item)?;
    store_teacher_draft(conn, &item, &spec)?;
    get(conn, id)
}

/// 校对后统一走 courseware::build；闸未通过或写盘失败时都保留 REVIEW 与草稿。
pub fn commit(
    conn: &Connection,
    data_dir: &Path,
    id: &str,
    spec: &HandoutSpec,
) -> Result<BuildOutcome, String> {
    let item = get(conn, id)?;
    if item.status != "REVIEW" {
        return Err("只有待校对的课件可以入库".into());
    }
    let mut spec = spec.clone();
    restore_trace(&mut spec, &item)?;
    store_teacher_draft(conn, &item, &spec)?;
    let outcome = match super::build(conn, data_dir, &mut spec, None) {
        Ok(outcome) => outcome,
        Err(error) => {
            conn.execute(
                "update local_courseware_import
                    set status = 'REVIEW', error = ?2, updated_at = ?3 where id = ?1",
                params![id, format!("课件入库失败：{error}"), now()],
            )
            .map_err(|e| e.to_string())?;
            return Err(error);
        }
    };
    let draft = serde_json::to_string(&spec).map_err(|e| e.to_string())?;
    if outcome.ok {
        let courseware_id = outcome
            .id
            .clone()
            .ok_or_else(|| "课件已生成但没有返回课件 id".to_string())?;
        conn.execute(
            "update local_courseware_import
                set status = 'COMMITTED', title = ?2, draft_json = ?3, courseware_id = ?4,
                    error = '', updated_at = ?5 where id = ?1",
            params![id, spec.title.trim(), draft, courseware_id, now()],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "update local_courseware_import
                set status = 'REVIEW', title = ?2, draft_json = ?3, error = ?4, updated_at = ?5
              where id = ?1",
            params![id, spec.title.trim(), draft, outcome.message, now()],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_draft_preserves_text_and_managed_trace_without_ai() {
        let item = ImportItem {
            id: "task".into(),
            batch_id: "batch".into(),
            source_path: "/tmp/牛顿定律.md".into(),
            managed_path: "courseware/imports/task/牛顿定律.md".into(),
            sha256: "abc".into(),
            title: "旧标题".into(),
            status: "EXTRACTING".into(),
            mode: "FORMAT_ONLY".into(),
            use_ai: false,
            subject_id: Some("subj-PHYS".into()),
            subject: "物理".into(),
            grade: "高一".into(),
            extracted_text: String::new(),
            draft: None,
            raw_ai_json: None,
            warnings: vec!["抽取警告".into()],
            locations: vec!["Markdown 正文".into()],
            error: String::new(),
            courseware_id: None,
            attempts: 1,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let text = "第一段\n\n第二段，原文完整保留。";
        let extracted = source::ExtractedSource {
            text: text.into(),
            warnings: item.warnings.clone(),
            locations: item.locations.clone(),
        };
        let spec = local_draft(
            &item,
            "courseware/imports/task/牛顿定律.md",
            "abc",
            &extracted,
        );
        assert_eq!(spec.title, "牛顿定律");
        assert_eq!(spec.points.len(), 1);
        assert_eq!(spec.points[0].detail, text);
        assert!(spec.examples.is_empty());
        assert!(spec.knowledge.is_empty());
        assert_eq!(spec.source.original_path, item.managed_path);
        assert_eq!(spec.source.locations, item.locations);
        assert_eq!(spec.source.warnings, item.warnings);
        assert!(!Path::new(&spec.source.original_path).is_absolute());
    }

    #[test]
    fn managed_names_cannot_escape_task_directory() {
        let path = Path::new("..\\..\\evil/常规 讲义.md");
        let name = safe_file_name(path);
        assert!(!name.contains('/'));
        assert!(!name.contains('\\'));
        assert!(!name.contains(".."));
        assert!(managed_absolute(Path::new("/tmp/data"), "../outside").is_err());
        assert!(managed_absolute(Path::new("/tmp/data"), "/tmp/outside").is_err());
    }
}
