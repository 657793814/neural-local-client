//! 异步任务：AI 出题、截图识别、AI 归类。
//!
//! 任务表存在 `generation_job`，线程池里的 worker 自己开 SQLite 连接；
//! 前端轮询 `job_get`，确认后才 `job_commit` 入库——半成品不污染题库。

use crate::db;
use crate::llm::{self, prompts, ImageInput};
use crate::models::{Job, OptionItem, QuestionInput};
use crate::state::JobContext;
use crate::text;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn now() -> String {
    text::now()
}

fn load_job(conn: &Connection, id: &str) -> Result<Job, String> {
    conn.query_row(
        "select id, kind, status, progress, error, input_json, result_json, created_at, updated_at
         from generation_job where id = ?1",
        [id],
        |row| {
            let input: String = row.get(5)?;
            let result: Option<String> = row.get(6)?;
            Ok(Job {
                id: row.get(0)?,
                kind: row.get(1)?,
                status: row.get(2)?,
                progress: row.get(3)?,
                error: row.get(4)?,
                input: serde_json::from_str(&input).unwrap_or(json!({})),
                result: result
                    .as_deref()
                    .and_then(|raw| serde_json::from_str(raw).ok()),
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

pub fn get(conn: &Connection, id: &str) -> Result<Job, String> {
    load_job(conn, id)
}

pub fn recent(conn: &Connection) -> Result<Vec<Job>, String> {
    let mut stmt = conn
        .prepare("select id from generation_job order by created_at desc limit 20")
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(ids
        .iter()
        .filter_map(|id| load_job(conn, id).ok())
        .collect())
}

pub fn cancel(conn: &Connection, id: &str) {
    let _ = conn.execute(
        "update generation_job set status = 'CANCELLED', updated_at = ?2
         where id = ?1 and status = 'RUNNING'",
        params![id, now()],
    );
}

fn set_progress(conn: &Connection, id: &str, progress: i64) {
    let _ = conn.execute(
        "update generation_job set progress = ?2, updated_at = ?3 where id = ?1",
        params![id, progress, now()],
    );
}

fn cancelled(conn: &Connection, id: &str) -> bool {
    conn.query_row(
        "select status from generation_job where id = ?1",
        [id],
        |row| row.get::<_, String>(0),
    )
    .map(|status| status == "CANCELLED")
    .unwrap_or(false)
}

fn subject_name(conn: &Connection, subject_id: &str) -> String {
    conn.query_row(
        "select name from subject where id = ?1",
        [subject_id],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or_else(|| "综合".to_string())
}

fn knowledge_names(conn: &Connection, subject_id: &str) -> Vec<String> {
    let mut stmt = match conn
        .prepare("select name from knowledge_point where subject_id = ?1 order by sort, name")
    {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([subject_id], |row| row.get(0))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// 知识点名字的归一化：模型常把标题写得和本地目录略有出入（多了空格、书名号、
/// 括号注释、全角标点、大小写），精确匹配会漏。这里做保守归一，只用于兜底匹配，
/// 不改库里的原名。全角转半角 + 去掉空白/《》/（）及其中注释 + 统一大小写。
fn normalize_kp_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        // 全角 ASCII（！～）映射到半角，标点本身在下一步按情况丢弃。
        let half = if ('\u{FF01}'..='\u{FF5E}').contains(&ch) {
            char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch)
        } else {
            ch
        };
        out.push(half);
    }
    // 去掉括号及其内容：模型爱写「勾股定理（八年级）」这种后缀。
    let mut stripped = String::with_capacity(out.len());
    let mut depth = 0usize;
    for ch in out.chars() {
        match ch {
            '(' | '[' | '【' => depth += 1,
            ')' | ']' | '】' if depth > 0 => depth -= 1,
            _ if depth == 0 => stripped.push(ch),
            _ => {}
        }
    }
    stripped
        .chars()
        .filter(|c| !c.is_whitespace() && !"《》〈〉\"'`，,。.、:：;；".contains(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn resolve_point(conn: &Connection, subject_id: &str, name: &str) -> Option<String> {
    let name = name.trim();
    // 1) 原样精确匹配：本学科优先，其次全局。这是最可信的一档，绝不让位给归一匹配。
    if let Some(id) = conn
        .query_row(
            "select id from knowledge_point where subject_id = ?1 and name = ?2 limit 1",
            params![subject_id, name],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
    {
        return Some(id);
    }
    if let Some(id) = conn
        .query_row(
            "select id from knowledge_point where name = ?1 limit 1",
            [name],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
    {
        return Some(id);
    }

    // 2) 归一化后相等：拆掉空格/书名号/括号注释等写法差异。
    let wanted = normalize_kp_name(name);
    if wanted.is_empty() {
        return None;
    }
    // 一次读出全部 (id, subject_id, name)，本学科排在前面，取第一个归一相等的。
    let mut stmt = conn
        .prepare("select id, subject_id, name from knowledge_point")
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .ok()?;
    let mut same_subject: Option<String> = None;
    let mut any_subject: Option<String> = None;
    for row in rows.flatten() {
        if normalize_kp_name(&row.2) != wanted {
            continue;
        }
        if any_subject.is_none() {
            any_subject = Some(row.0.clone());
        }
        if row.1 == subject_id {
            same_subject = Some(row.0);
            break;
        }
    }
    if let Some(id) = same_subject.or(any_subject) {
        return Some(id);
    }
    // 词表里确实没有：补一条自建知识点。词表再全也有覆盖不到的时候，
    // 与其把这道题的归类信息丢掉，不如把新词收进来（origin='ai'，可在复核时清理）。
    crate::db::ensure_ai_point(conn, subject_id, name)
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(|text| text.trim().to_string()))
            .filter(|item| !item.is_empty())
            .collect(),
        Some(Value::String(text)) => text
            .split([';', '；', ',', '，'])
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn options_of(value: Option<&Value>) -> Vec<OptionItem> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let map = item.as_object();
            let key = map
                .and_then(|map| map.get("key").or_else(|| map.get("label")))
                .and_then(|node| node.as_str())
                .map(|text| text.trim().to_string())
                .unwrap_or_else(|| ((b'A' + index as u8) as char).to_string());
            // 网页抓取与本地 OCR 都可能给纯字符串数组；模型则给 {key,text}。
            // 两种都收，别让整道题的选项在归一化时被清空。
            let text = item
                .as_str()
                .or_else(|| {
                    map.and_then(|map| map.get("text"))
                        .and_then(|node| node.as_str())
                })
                .unwrap_or("")
                .to_string();
            OptionItem { key, text }
        })
        .collect()
}

/// 难度可能是数字，也可能是「易 / 中等 / 较难」或「3」这种字符串。
/// 网页 DOM 与本地 OCR 都不是模型，不能指望它们一定给整数。
fn difficulty_of(value: Option<&Value>) -> i64 {
    let Some(node) = value else { return 3 };
    if let Some(number) = node.as_i64() {
        return number.clamp(1, 5);
    }
    if let Some(number) = node.as_f64() {
        return (number.round() as i64).clamp(1, 5);
    }
    let Some(text) = node.as_str() else { return 3 };
    let text = text.trim();
    if let Ok(number) = text.parse::<i64>() {
        return number.clamp(1, 5);
    }
    // 「很容易 / 很难」要先于「容易 / 难」判断，否则会被前半截吞掉（5 变 4）。
    if text.contains("很容易") {
        1
    } else if text.contains("很难") {
        5
    } else if text.contains('易') || text.contains("简单") || text.contains("基础") {
        2
    } else if text.contains("较难") || text.contains('难') || text.contains('高') {
        4
    } else {
        3
    }
}

/// 真正拦住、不让进草稿箱的只有一种情况：连题干都没认出来。
/// 草稿箱本来就是给「还没补完」的题待的地方，空题存进去只会是垃圾。
fn blocking_issues(stem: &str) -> Vec<String> {
    let mut issues: Vec<String> = Vec::new();
    if stem.trim().is_empty() {
        issues.push("缺少题干".into());
    } else if crate::ocr::is_web_ui_stem(stem) {
        // 网页截图底部的按钮被当成一道题；三条识别路径统一在这里兜底拦住。
        issues.push("疑似网页按钮文字，不是题目".into());
    }
    issues
}

/// 不拦人，但必须在卡片上标出来：复核之前这些得补。
/// 以前缺答案 / 解析也算「不能存」，识别结果一半都被挡在门外；
/// 现在它们只是待办，勾上就能先存草稿。知识点是归章依据，归类以外的链路必须匹配到。
fn pending_fields(
    kind: &str,
    options: &[OptionItem],
    answer: &str,
    analysis: &str,
    knowledge_ids: &[String],
    require_knowledge: bool,
) -> Vec<String> {
    let mut issues: Vec<String> = Vec::new();
    if answer.trim().is_empty() {
        issues.push("缺答案".into());
    }
    if let Some(issue) = crate::repo::choice_answer_issue(kind, options, answer) {
        issues.push(issue);
    }
    if analysis.trim().is_empty() {
        issues.push("缺解析".into());
    }
    if require_knowledge && knowledge_ids.is_empty() {
        issues.push("没匹配到知识点（题库按知识点归章）".into());
    }
    issues
}

/// 把模型 / 网页 / OCR 给的一道题整理成任务结果项：解析知识点、补问题清单。
pub(crate) fn normalize_draft(
    conn: &Connection,
    subject_id: &str,
    value: &Value,
    require_knowledge: bool,
) -> Value {
    let kind = value
        .get("type")
        .and_then(|node| node.as_str())
        .and_then(text::normalize_type)
        .unwrap_or("SOLVED");
    let stem = value
        .get("stem")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let answer = value
        .get("answer")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let analysis = value
        .get("analysis")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let difficulty = difficulty_of(value.get("difficulty"));
    // 旧包可能只带 examPoints 而没有 knowledgeNames：那也是知识点名，并进来一起解析，
    // 不另立一个概念，也不让题因为缺词表就丢掉归类信息。
    let mut names = strings(value.get("knowledgeNames"));
    if names.is_empty() {
        names = strings(value.get("examPoints"));
    }
    let warnings = strings(value.get("warnings"));

    let knowledge_ids: Vec<String> = names
        .iter()
        .filter_map(|name| resolve_point(conn, subject_id, name))
        .collect();
    let blocking = blocking_issues(&stem);
    let options = options_of(value.get("options"));
    let issues = pending_fields(
        kind,
        &options,
        &answer,
        &analysis,
        &knowledge_ids,
        require_knowledge,
    );

    json!({
        "type": kind,
        "stem": stem,
        "options": options,
        "answer": answer,
        "analysis": analysis,
        "difficulty": difficulty,
        "knowledgeNames": names,
        "knowledgeIds": knowledge_ids,
        "subjectId": subject_id,
        "warnings": warnings,
        "blocking": blocking,
        "issues": issues,
        "source": value.get("source").cloned().unwrap_or(Value::Null),
        "year": value.get("year").cloned().unwrap_or(Value::Null),
        "confidence": value.get("confidence").cloned().unwrap_or(Value::Null),
    })
}

fn questions_of(value: &Value) -> Vec<Value> {
    value
        .get("questions")
        .and_then(|node| node.as_array())
        .cloned()
        .or_else(|| value.as_array().cloned())
        .unwrap_or_default()
}

fn is_choice(kind: &str) -> bool {
    kind == "SINGLE" || kind == "MULTI"
}

fn push_warning(map: &mut serde_json::Map<String, Value>, text: String) {
    match map.get_mut("warnings").and_then(|node| node.as_array_mut()) {
        Some(list) => list.push(json!(text)),
        None => {
            map.insert("warnings".into(), json!([text]));
        }
    }
}

/// 模型常常不按题型的字面意思走：要解答题也照样给单选。
/// 生成后按用户所选的题型纠正一次，并把差异写进 warnings，让人核对时看得见。
fn enforce_type(item: &mut Value, requested: &[String]) {
    let Some(map) = item.as_object_mut() else {
        return;
    };
    let kind = map
        .get("type")
        .and_then(|node| node.as_str())
        .unwrap_or("SOLVED")
        .to_string();
    if requested.is_empty() || requested.iter().any(|code| code == &kind) {
        // 非选择题不该带选项，模型偶尔会顺手给 A/B/C/D，直接清掉。
        if !is_choice(&kind) {
            map.insert("options".into(), json!([]));
        }
        return;
    }
    let target = if requested.len() == 1 {
        requested[0].clone()
    } else if is_choice(&kind) {
        requested
            .iter()
            .find(|code| is_choice(code))
            .cloned()
            .unwrap_or_else(|| requested[0].clone())
    } else {
        requested
            .iter()
            .find(|code| !is_choice(code))
            .cloned()
            .unwrap_or_else(|| requested[0].clone())
    };
    let answer = map
        .get("answer")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .to_string();
    map.insert("type".into(), json!(target));
    if !is_choice(&target) {
        map.insert("options".into(), json!([]));
    }
    push_warning(
        map,
        format!(
            "题型与所选不符：要{}，模型给了{}，已按所选的改，请核对答案与解析",
            text::type_label(&target),
            text::type_label(&kind)
        ),
    );
    if !is_choice(&target) {
        let looks_like_letter = !answer.is_empty()
            && answer.chars().count() <= 3
            && answer.chars().all(|ch| ch.is_ascii_uppercase());
        if looks_like_letter {
            push_warning(map, "答案原来是选项字母，请改成结论文字".to_string());
        }
    }
}

fn run_generate(
    ctx: &JobContext,
    http: &reqwest::blocking::Client,
    job_id: &str,
    input: &Value,
) -> Result<Value, String> {
    let conn = db::open(&ctx.db_path)?;
    let subject_id = input
        .get("subjectId")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .to_string();
    if subject_id.is_empty() {
        return Err("先选一个学科".into());
    }
    let subject = subject_name(&conn, &subject_id);
    // 前端传的是枚举值；顺手兼容中文写法，别让「简答题」这种输入落成空。
    let kinds: Vec<String> = strings(input.get("types"))
        .iter()
        .filter_map(|raw| text::normalize_type(raw).map(|code| code.to_string()))
        .collect();
    let kinds = if kinds.is_empty() {
        vec!["SINGLE".to_string(), "FILL".to_string()]
    } else {
        kinds
    };
    let difficulty = input
        .get("difficulty")
        .and_then(|node| node.as_str())
        .unwrap_or("中档")
        .to_string();
    // 用户选的难度是硬性要求：模型常在示例里照抄 3，不能让它自己判。
    let difficulty_level = difficulty_of(input.get("difficulty"));
    let count = input
        .get("count")
        .and_then(|node| node.as_i64())
        .unwrap_or(5)
        .clamp(1, 20);
    let with_figures = input
        .get("withFigures")
        .and_then(|node| node.as_bool())
        .unwrap_or(false);
    let focus = input.get("focus").and_then(|node| node.as_str());
    let knowledge = knowledge_names(&conn, &subject_id);

    // 分批生成，一批最多 5 道：单次 JSON 出太多题容易被 max_tokens 截断，
    // 情境也容易互相重复；分批后每批独立解析，前一批失败不影响后面，
    // 进度条还能实时往前走，而不是 15% 卡到 70%。
    const BATCH: i64 = 5;
    let mut cooldowns = HashMap::new();
    let mut items: Vec<Value> = Vec::new();
    let mut done: i64 = 0;
    while done < count {
        let batch_count = (count - done).min(BATCH);
        let messages = prompts::generate(
            &subject,
            &kinds,
            &difficulty,
            difficulty_level,
            batch_count,
            &knowledge,
            focus,
            with_figures,
        );
        let raw = llm::complete(
            &conn,
            http,
            "question",
            &messages,
            None,
            true,
            6000,
            &mut cooldowns,
        )?;
        done += batch_count;
        set_progress(&conn, job_id, 15 + (55 * done / count.max(1)));
        let parsed = llm::parse_json(&raw, "出题模型")?;
        let questions = questions_of(&parsed);
        // needFigure 在 normalize_draft 里会被丢掉（它只认白名单字段），先抄下来再贴回去。
        let need_flags: Vec<bool> = questions
            .iter()
            .map(|value| {
                value
                    .get("needFigure")
                    .and_then(|node| node.as_bool())
                    .unwrap_or(false)
            })
            .collect();
        let mut batch: Vec<Value> = questions
            .iter()
            .map(|value| normalize_draft(&conn, &subject_id, value, true))
            .collect();
        for (item, need) in batch.iter_mut().zip(need_flags) {
            item["needFigure"] = json!(need);
        }
        for item in &mut batch {
            enforce_type(item, &kinds);
            // 提示词已经写死难度，这里再兜一层：模型万一还是回 3，也按用户选的来。
            item["difficulty"] = json!(difficulty_level);
        }
        // 模型多给了就只留要求的数量，别让「要 5 道出 9 道」悄悄发生。
        batch.truncate(batch_count as usize);
        items.extend(batch);
    }
    if items.is_empty() {
        return Err("模型没有给出题目，换个模型或减少数量再试".into());
    }

    // 带图出题第二段：对 needFigure=true 的题逐道走制图链路（场景 JSON → 本地 SVG），
    // 图挂不上就降级成纯文字题，不让整个任务失败。题干补 `![](图1)` 引用，
    // 提交时 stage_figures 会像截图识别的图一样转成正式附件。
    if with_figures {
        let mode = if subject.contains("数学") { "math" } else { "general" };
        let tmp_dir = data_dir_of(ctx).join("tmp");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let total = items.len() as i64;
        for (index, item) in items.iter_mut().enumerate() {
            if !item
                .get("needFigure")
                .and_then(|node| node.as_bool())
                .unwrap_or(false)
            {
                continue;
            }
            let stem = item
                .get("stem")
                .and_then(|node| node.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if stem.is_empty() {
                continue;
            }
            set_progress(
                &conn,
                job_id,
                (70 + 25 * index as i64 / total.max(1)).min(95),
            );
            let figure = crate::diagram::generate(&conn, http, &subject, &stem, "", mode);
            let Ok(value) = figure else {
                continue;
            };
            let Some(svg) = value.get("svg").and_then(|node| node.as_str()) else {
                continue;
            };
            let path = tmp_dir.join(format!("gen-{}.svg", uuid::Uuid::new_v4()));
            if std::fs::write(&path, svg).is_err() {
                continue;
            }
            item["figures"] = json!([{"ref": "图1", "path": path.to_string_lossy()}]);
            item["stem"] = json!(format!("{stem}\n\n![](图1)"));
        }
    }

    set_progress(&conn, job_id, 90);
    Ok(json!({"items": items, "subjectName": subject}))
}

/// 识别一张图，返回归一化后的题目条目。大图在识别前本地缩放（原图不动）。
/// cooldowns 在整批图片间共享，避免多图连续触发 429。
/// 识别引擎：`local` 只用本机 OCR，`vision` 只用视觉模型，`auto` 先视觉模型再回退本机 OCR。
fn capture_engine(input: &Value) -> String {
    match input.get("engine").and_then(|node| node.as_str()) {
        Some("local") => "local".into(),
        Some("vision") => "vision".into(),
        _ => "auto".into(),
    }
}

/// 数据目录：数据库文件所在的父目录。`tmp/figures` 就放在它下面。
fn data_dir_of(ctx: &JobContext) -> PathBuf {
    ctx.db_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 视觉模型识别：读图、缩放、调模型、解析 JSON。返回归一化后的条目列表。
/// 模型标出的图形区域会在 `extract_figures` 里按原图裁出来，随题一起预览与入库。
fn vision_recognize(
    conn: &Connection,
    http: &reqwest::blocking::Client,
    job_id: &str,
    tmp_dir: &Path,
    path: &str,
    source_url: &str,
    queue_id: &str,
    subject_id: &str,
    subject: &str,
    knowledge: &[String],
    base_progress: i64,
    span_progress: i64,
    cooldowns: &mut HashMap<String, std::time::Instant>,
    fallback_note: Option<&str>,
) -> Result<Vec<Value>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读不了截图：{e}"))?;
    let (mime, _) = text::sniff_image(&bytes).ok_or_else(|| "截图不是图片".to_string())?;
    // 大图缩放件只用于识别；库里和队列里保留原图。
    let (send_mime, send_bytes) = if bytes.len() > 8 * 1024 * 1024 {
        let img = image::load_from_memory(&bytes).map_err(|e| format!("图片解码失败：{e}"))?;
        let scaled = img.thumbnail(2000, 2000);
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(scaled.to_rgba8())
            .write_to(
                &mut std::io::Cursor::new(&mut buf),
                image::ImageFormat::Jpeg,
            )
            .map_err(|e| format!("图片压缩失败：{e}"))?;
        ("image/jpeg".to_string(), buf)
    } else {
        (mime.to_string(), bytes)
    };
    set_progress(conn, job_id, base_progress + span_progress / 3);
    let messages = prompts::capture(subject, knowledge);
    let image = ImageInput {
        mime: send_mime,
        bytes: send_bytes,
    };
    let raw = llm::complete(
        conn,
        http,
        "capture",
        &messages,
        Some(&image),
        true,
        4096,
        cooldowns,
    )?;
    set_progress(conn, job_id, base_progress + (span_progress * 2) / 3);
    let parsed = llm::parse_json(&raw, "识别模型")?;
    let raw_questions = questions_of(&parsed);
    let mut items: Vec<Value> = raw_questions
        .iter()
        .map(|value| normalize_draft(conn, subject_id, value, true))
        .collect();
    for (index, item) in items.iter_mut().enumerate() {
        // 模型用 `![](图N)` 标出的图形，按原图 bbox 裁出来给预览与入库用。
        let figure_warnings = raw_questions
            .get(index)
            .map(|source| extract_figures(path, tmp_dir, job_id, source, item))
            .unwrap_or_default();
        // 每题记住自己来自哪张图，commit 时按题挂原图；来源网址自动填。
        if let Some(map) = item.as_object_mut() {
            map.insert("capturePath".into(), json!(path));
            map.insert("queueId".into(), json!(queue_id));
            map.insert("engine".into(), json!("vision"));
            if !source_url.is_empty() && map.get("source").map(|v| v.is_null()).unwrap_or(true) {
                map.insert("source".into(), json!(source_url));
            }
            // 题干提到「如图」但模型没给图形区域：提醒复核时补图，别当成纯文字题入库。
            let has_figure = map
                .get("figures")
                .and_then(|node| node.as_array())
                .map(|list| !list.is_empty())
                .unwrap_or(false);
            let stem = map
                .get("stem")
                .and_then(|node| node.as_str())
                .unwrap_or("")
                .to_string();
            if !has_figure
                && ["如图", "图中", "下图", "上图"]
                    .iter()
                    .any(|word| stem.contains(word))
            {
                push_warning(
                    map,
                    "题干提到图形，但这次没定位到图片；入库后建议回原图核对".into(),
                );
            }
            if let Some(note) = fallback_note {
                push_warning(map, note.to_string());
            }
            for warning in figure_warnings {
                push_warning(map, warning);
            }
        }
    }
    Ok(items)
}

/// 把模型标出的图形区域从原图裁出来，存到 `tmp/figures/<job>` 供预览。
/// 返回需要写进 warnings 的提示（裁剪失败不拦题，只是少一张图）。
fn extract_figures(
    path: &str,
    tmp_dir: &Path,
    job_id: &str,
    source: &Value,
    item: &mut Value,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let raw = source
        .get("figures")
        .and_then(|node| node.as_array())
        .cloned()
        .unwrap_or_default();
    if raw.is_empty() {
        return warnings;
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => return vec![format!("图形区域没读出来：{error}")],
    };
    let image = match image::load_from_memory(&bytes) {
        Ok(image) => image,
        Err(error) => return vec![format!("图形区域没读出来：{error}")],
    };
    let (width, height) = (image.width() as f64, image.height() as f64);
    let dir = tmp_dir.join("figures").join(job_id);
    if std::fs::create_dir_all(&dir).is_err() {
        return vec!["图形区域没存下来：临时目录不可写".into()];
    }
    let mut kept = Vec::new();
    for (index, figure) in raw.iter().enumerate() {
        let reference = figure
            .get("ref")
            .and_then(|node| node.as_str())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| format!("图{}", index + 1));
        let Some(bbox) = figure.get("bbox").and_then(|node| node.as_array()) else {
            warnings.push(format!("{reference} 没给位置，没能裁出来"));
            continue;
        };
        let nums: Vec<f64> = bbox.iter().filter_map(|node| node.as_f64()).collect();
        if nums.len() < 4 {
            warnings.push(format!("{reference} 位置不完整，没能裁出来"));
            continue;
        }
        // 兼容 0–1 与 0–1000 两种归一化写法。
        let scale = if nums.iter().cloned().fold(0.0_f64, f64::max) <= 1.5 {
            1.0
        } else {
            1000.0
        };
        let left = (nums[0] / scale * width).clamp(0.0, width);
        let top = (nums[1] / scale * height).clamp(0.0, height);
        let right = ((nums[0] + nums[2]) / scale * width).clamp(0.0, width);
        let bottom = ((nums[1] + nums[3]) / scale * height).clamp(0.0, height);
        let crop_w = right - left;
        let crop_h = bottom - top;
        if crop_w < 16.0 || crop_h < 16.0 {
            warnings.push(format!("{reference} 区域太小，没能裁出来"));
            continue;
        }
        let cropped = image.crop_imm(
            left.round() as u32,
            top.round() as u32,
            crop_w.round() as u32,
            crop_h.round() as u32,
        );
        // 预览与任务结果都按需加载，压到 1400px 以内即可。
        let cropped = if cropped.width() > 1400 {
            let ratio = 1400.0 / cropped.width() as f64;
            cropped.resize_exact(
                1400,
                ((cropped.height() as f64 * ratio).round() as u32).max(1),
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            cropped
        };
        let file = dir.join(format!("{index}-{}.jpg", uuid::Uuid::new_v4()));
        let rgb = image::DynamicImage::ImageRgb8(cropped.to_rgb8());
        if rgb
            .save_with_format(&file, image::ImageFormat::Jpeg)
            .is_err()
        {
            warnings.push(format!("{reference} 没能存成图片"));
            continue;
        }
        kept.push(json!({
            "ref": reference,
            "path": file.to_string_lossy(),
            // 归一化框（0–1）跟着图一起回给前端：模型框歪了，人可以拖着改，
            // 改完按这个框重新裁一张。老结果没这个字段，前端就退回整张图。
            "box": {
                "x": (left / width).clamp(0.0, 1.0),
                "y": (top / height).clamp(0.0, 1.0),
                "w": (crop_w / width).clamp(0.0, 1.0),
                "h": (crop_h / height).clamp(0.0, 1.0),
            }
        }));
    }
    if let Some(map) = item.as_object_mut() {
        map.insert("figures".into(), json!(kept));
    }
    warnings
}

/// 本机 OCR 识别：离线、不花模型额度。返回归一化后的条目列表。
fn local_recognize(
    conn: &Connection,
    job_id: &str,
    path: &str,
    source_url: &str,
    queue_id: &str,
    subject_id: &str,
    base_progress: i64,
    span_progress: i64,
) -> Result<Vec<Value>, String> {
    set_progress(conn, job_id, base_progress + span_progress / 5);
    let raw = crate::ocr::recognize_questions(path, source_url)?;
    let mut items: Vec<Value> = raw
        .iter()
        .map(|value| normalize_draft(conn, subject_id, value, true))
        .collect();
    if items.is_empty() {
        return Err("本机 OCR 读到了文字，但没整理出题干".into());
    }
    for item in &mut items {
        if let Some(map) = item.as_object_mut() {
            map.insert("capturePath".into(), json!(path));
            map.insert("queueId".into(), json!(queue_id));
            map.insert("engine".into(), json!("local"));
            // 本机 OCR 只读文字，题干里的图形不会保留；提示用户换 AI 识图。
            let stem = map
                .get("stem")
                .and_then(|node| node.as_str())
                .unwrap_or("")
                .to_string();
            if ["如图", "图中", "下图", "上图"]
                .iter()
                .any(|word| stem.contains(word))
            {
                push_warning(
                    map,
                    "本机 OCR 只读文字，图形没保留；要连图一起入库请切「AI 识图」".into(),
                );
            }
            if !source_url.is_empty()
                && map
                    .get("source")
                    .map(|value| value.is_null())
                    .unwrap_or(true)
            {
                map.insert("source".into(), json!(source_url));
            }
        }
    }
    set_progress(conn, job_id, base_progress + span_progress);
    Ok(items)
}

fn capture_one_image(
    conn: &Connection,
    http: &reqwest::blocking::Client,
    job_id: &str,
    tmp_dir: &Path,
    engine: &str,
    path: &str,
    source_url: &str,
    queue_id: &str,
    subject_id: &str,
    subject: &str,
    knowledge: &[String],
    base_progress: i64,
    span_progress: i64,
    cooldowns: &mut HashMap<String, std::time::Instant>,
) -> Result<Vec<Value>, String> {
    // `local`：只用本机 OCR，认不出就报错让用户切「AI 识图」。
    if engine == "local" {
        return local_recognize(
            conn,
            job_id,
            path,
            source_url,
            queue_id,
            subject_id,
            base_progress,
            span_progress,
        )
        .map_err(|error| format!("{error}；可以切到「AI 识图」再试"));
    }

    // `vision`：只用视觉模型。
    if engine == "vision" {
        return vision_recognize(
            conn,
            http,
            job_id,
            tmp_dir,
            path,
            source_url,
            queue_id,
            subject_id,
            subject,
            knowledge,
            base_progress,
            span_progress,
            cooldowns,
            None,
        );
    }

    // `auto`：大模型优先——视觉模型认题更准，尤其是公式和图片题。
    // 只有模型不可用（没配置 Key、网络不通、服务商报错）时才回退本机 OCR。
    match vision_recognize(
        conn,
        http,
        job_id,
        tmp_dir,
        path,
        source_url,
        queue_id,
        subject_id,
        subject,
        knowledge,
        base_progress,
        span_progress,
        cooldowns,
        None,
    ) {
        Ok(items) => return Ok(items),
        Err(vision_error) => {
            // 视觉模型失败，回退本机 OCR，并把视觉失败的原因写进 warnings。
            let note = format!("AI 识图失败，已改用本机 OCR：{vision_error}");
            return local_recognize(
                conn,
                job_id,
                path,
                source_url,
                queue_id,
                subject_id,
                base_progress,
                span_progress,
            )
            .map(|mut items| {
                for item in &mut items {
                    if let Some(map) = item.as_object_mut() {
                        push_warning(map, note.clone());
                    }
                }
                items
            })
            .map_err(|local_error| {
                format!("AI 识图失败：{vision_error}；本机 OCR 也没成功：{local_error}")
            });
        }
    }
}

pub(crate) fn resolve_subject(conn: &Connection, input: &Value) -> (String, String, Vec<String>) {
    let subject_id = input
        .get("subjectId")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .to_string();
    let subject_id = if subject_id.is_empty() {
        conn.query_row("select id from subject order by sort limit 1", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap_or_else(|_| "subj-MATH".to_string())
    } else {
        subject_id
    };
    (
        subject_id.clone(),
        subject_name(conn, &subject_id),
        knowledge_names(conn, &subject_id),
    )
}

fn run_capture(
    ctx: &JobContext,
    http: &reqwest::blocking::Client,
    job_id: &str,
    input: &Value,
) -> Result<Value, String> {
    let conn = db::open(&ctx.db_path)?;
    let path = input
        .get("imagePath")
        .and_then(|node| node.as_str())
        .ok_or_else(|| "没有拿到截图".to_string())?;
    let source_url = input
        .get("sourceUrl")
        .and_then(|node| node.as_str())
        .unwrap_or("");
    // 这张图如果是连拍队列里的某一项，识别过程要回写它的状态；
    // 否则队列里永远停在「待识别」，人看着像没生效。
    let queue_id = input
        .get("queueId")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !queue_id.is_empty() {
        let _ = crate::capture_store::mark(&conn, &queue_id, "RUNNING", Some(job_id), "");
    }
    let engine = capture_engine(input);
    let tmp_dir = data_dir_of(ctx).join("tmp");
    let (subject_id, subject, knowledge) = resolve_subject(&conn, input);
    let mut cooldowns = HashMap::new();
    let outcome = capture_one_image(
        &conn,
        http,
        job_id,
        &tmp_dir,
        &engine,
        path,
        source_url,
        &queue_id,
        &subject_id,
        &subject,
        &knowledge,
        10,
        80,
        &mut cooldowns,
    );
    let items = match outcome {
        Ok(items) if !items.is_empty() => items,
        Ok(_) => {
            let message = "这张图里没认出题目，换一张更清晰的截图";
            if !queue_id.is_empty() {
                let _ =
                    crate::capture_store::mark(&conn, &queue_id, "FAILED", Some(job_id), message);
            }
            return Err(message.into());
        }
        Err(error) => {
            if !queue_id.is_empty() {
                let _ = crate::capture_store::mark(&conn, &queue_id, "FAILED", Some(job_id), &error);
            }
            return Err(error);
        }
    };
    if !queue_id.is_empty() {
        let _ = crate::capture_store::set_item_count(&conn, &queue_id, items.len() as i64);
    }
    set_progress(&conn, job_id, 95);
    Ok(json!({"items": items, "subjectName": subject, "imagePath": path}))
}

/// 批量识别：输入 images=[{path, sourceUrl, queueId}]，逐张识别后合并。
/// 单张失败不影响其它张；失败原因回写队列项，可单独重试。
fn run_capture_batch(
    ctx: &JobContext,
    http: &reqwest::blocking::Client,
    job_id: &str,
    input: &Value,
) -> Result<Value, String> {
    let conn = db::open(&ctx.db_path)?;
    let images: Vec<(String, String, String)> = input
        .get("images")
        .and_then(|node| node.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|row| {
                    let path = row.get("path")?.as_str()?.to_string();
                    let source_url = row
                        .get("sourceUrl")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let queue_id = row
                        .get("queueId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some((path, source_url, queue_id))
                })
                .collect()
        })
        .unwrap_or_default();
    if images.is_empty() {
        return Err("队列里没有待识别的图片".into());
    }
    let engine = capture_engine(input);
    let tmp_dir = data_dir_of(ctx).join("tmp");
    let (subject_id, subject, knowledge) = resolve_subject(&conn, input);
    let mut cooldowns = HashMap::new();
    let mut all = Vec::new();
    let mut failures = Vec::new();
    let total = images.len() as i64;
    for (index, (path, source_url, queue_id)) in images.iter().enumerate() {
        if cancelled(&conn, job_id) {
            return Err("已取消".into());
        }
        if !queue_id.is_empty() {
            let _ = crate::capture_store::mark(&conn, queue_id, "RUNNING", Some(job_id), "");
        }
        let base = (index as i64 * 90) / total;
        match capture_one_image(
            &conn,
            http,
            job_id,
            &tmp_dir,
            &engine,
            path,
            source_url,
            queue_id,
            &subject_id,
            &subject,
            &knowledge,
            base,
            90 / total,
            &mut cooldowns,
        ) {
            Ok(mut items) => {
                if !queue_id.is_empty() {
                    let _ =
                        crate::capture_store::set_item_count(&conn, queue_id, items.len() as i64);
                }
                all.append(&mut items);
            }
            Err(error) => {
                if !queue_id.is_empty() {
                    let _ =
                        crate::capture_store::mark(&conn, queue_id, "FAILED", Some(job_id), &error);
                }
                failures.push(format!("第 {} 张：{error}", index + 1));
            }
        }
    }
    if all.is_empty() {
        return Err(if failures.is_empty() {
            "没有认出题目".into()
        } else {
            failures.join("；")
        });
    }
    set_progress(&conn, job_id, 95);
    Ok(json!({
        "items": all,
        "subjectName": subject,
        "imagePath": "",
        "failures": failures,
    }))
}

/// 文字取题：不需要视觉模型，走「对话」用途即可。
fn run_capture_text(
    ctx: &JobContext,
    http: &reqwest::blocking::Client,
    job_id: &str,
    input: &Value,
) -> Result<Value, String> {
    let conn = db::open(&ctx.db_path)?;
    let text_input = input
        .get("text")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .trim();
    if text_input.is_empty() {
        return Err("先粘贴题目文字".into());
    }
    let (subject_id, subject, knowledge) = resolve_subject(&conn, input);
    let source_url = input
        .get("sourceUrl")
        .and_then(|node| node.as_str())
        .unwrap_or("");
    let messages = prompts::capture_text(&subject, &knowledge, text_input);
    set_progress(&conn, job_id, 30);
    let mut cooldowns = HashMap::new();
    let raw = llm::complete(
        &conn,
        http,
        "chat",
        &messages,
        None,
        true,
        4096,
        &mut cooldowns,
    )?;
    let parsed = llm::parse_json(&raw, "整理模型")?;
    let mut items: Vec<Value> = questions_of(&parsed)
        .iter()
        .map(|value| normalize_draft(&conn, &subject_id, value, true))
        .collect();
    for item in &mut items {
        if let Some(map) = item.as_object_mut() {
            if !source_url.is_empty() && map.get("source").map(|v| v.is_null()).unwrap_or(true) {
                map.insert("source".into(), json!(source_url));
            }
        }
    }
    if items.is_empty() {
        return Err("没从文字里认出题目，检查粘贴内容是否完整".into());
    }
    set_progress(&conn, job_id, 90);
    Ok(json!({"items": items, "subjectName": subject, "imagePath": ""}))
}

fn run_classify(
    ctx: &JobContext,
    http: &reqwest::blocking::Client,
    job_id: &str,
    input: &Value,
) -> Result<Value, String> {
    let conn = db::open(&ctx.db_path)?;
    let ids = strings(input.get("questionIds"));
    if ids.is_empty() {
        return Err("先勾选要补充的题目".into());
    }
    let ids: Vec<String> = ids.into_iter().take(50).collect();
    let subjects: Vec<String> = conn
        .prepare("select name from subject order by sort")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();

    let mut cooldowns = HashMap::new();
    let mut items = Vec::new();
    let total = ids.len().max(1);
    for (index, id) in ids.iter().enumerate() {
        if cancelled(&conn, job_id) {
            return Err("已取消".into());
        }
        let row: Option<(String, String, String, String, i64)> = conn
            .query_row(
                "select ifnull(subject_id, ''), type, stem, answer, difficulty from question where id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((subject_id, kind, stem, answer, difficulty)) = row else {
            continue;
        };
        let subject_id = if subject_id.is_empty() {
            conn.query_row("select id from subject order by sort limit 1", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap_or_default()
        } else {
            subject_id
        };
        let knowledge = knowledge_names(&conn, &subject_id);
        let material = format!(
            "题型：{}\n难度：{}\n题干：{}\n答案：{}",
            text::type_label(&kind),
            difficulty,
            stem,
            if answer.is_empty() {
                "（缺失）"
            } else {
                &answer
            }
        );
        let messages = prompts::classify(&subjects, &knowledge, &material);
        let raw = match llm::complete(
            &conn,
            http,
            "classify",
            &messages,
            None,
            true,
            1600,
            &mut cooldowns,
        ) {
            Ok(raw) => raw,
            Err(error) => {
                items.push(json!({"questionId": id, "error": error}));
                continue;
            }
        };
        let parsed = match llm::parse_json(&raw, "归类模型") {
            Ok(value) => value,
            Err(error) => {
                items.push(json!({"questionId": id, "error": error}));
                continue;
            }
        };
        let knowledge_names_out = strings(parsed.get("knowledgeNames"));
        let single = parsed
            .get("knowledgeName")
            .and_then(|node| node.as_str())
            .map(|name| vec![name.to_string()])
            .unwrap_or_default();
        let chosen: Vec<String> = if knowledge_names_out.is_empty() {
            single
        } else {
            knowledge_names_out
        };
        let knowledge_ids: Vec<String> = chosen
            .iter()
            .filter_map(|name| resolve_point(&conn, &subject_id, name))
            .collect();
        let subject_out = parsed
            .get("subject")
            .and_then(|node| node.as_str())
            .unwrap_or("");
        let subject_resolved = conn
            .query_row(
                "select id from subject where name = ?1 limit 1",
                [subject_out],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten();

        items.push(json!({
            "questionId": id,
            "stem": stem.clone(),
            "stemExcerpt": text::excerpt(&stem, 60),
            "subjectId": subject_resolved,
            "subjectName": subject_out,
            "difficulty": parsed.get("difficulty").and_then(|node| node.as_i64()).map(|v| v.clamp(1,5)),
            "knowledgeIds": knowledge_ids,
            "knowledgeNames": chosen,
            "answer": parsed.get("answer").and_then(|node| node.as_str()).unwrap_or(""),
            "analysis": parsed.get("analysis").and_then(|node| node.as_str()).unwrap_or(""),
            "confidence": parsed.get("confidence").cloned().unwrap_or(Value::Null),
            "note": parsed.get("note").and_then(|node| node.as_str()).unwrap_or(""),
            "error": Value::Null,
        }));
        set_progress(&conn, job_id, ((index + 1) as i64 * 90) / total as i64);
    }
    Ok(json!({"items": items}))
}

fn run(
    ctx: &JobContext,
    http: &reqwest::blocking::Client,
    job_id: &str,
    kind: &str,
    input: &Value,
) {
    let result = match kind {
        "generate" => run_generate(ctx, http, job_id, input),
        "capture" => run_capture(ctx, http, job_id, input),
        "capture_batch" => run_capture_batch(ctx, http, job_id, input),
        "capture_text" => run_capture_text(ctx, http, job_id, input),
        "classify" => run_classify(ctx, http, job_id, input),
        _ => Err(format!("未知任务类型：{kind}")),
    };
    let Ok(conn) = db::open(&ctx.db_path) else {
        return;
    };
    match result {
        Ok(value) => {
            let _ = conn.execute(
                "update generation_job set status = 'DONE', progress = 100, result_json = ?2, updated_at = ?3
                 where id = ?1 and status = 'RUNNING'",
                params![job_id, value.to_string(), now()],
            );
        }
        Err(error) => {
            let status = if error.contains("已取消") {
                "CANCELLED"
            } else {
                "FAILED"
            };
            let _ = conn.execute(
                "update generation_job set status = ?2, error = ?3, updated_at = ?4 where id = ?1",
                params![job_id, status, error, now()],
            );
        }
    }
}

pub fn start(
    ctx: JobContext,
    http: reqwest::blocking::Client,
    conn: &Connection,
    kind: &str,
    input: Value,
) -> Result<String, String> {
    if !matches!(
        kind,
        "generate" | "capture" | "capture_batch" | "capture_text" | "classify"
    ) {
        return Err("未知任务类型".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "insert into generation_job (id, kind, status, input_json, progress, created_at, updated_at)
         values (?1, ?2, 'RUNNING', ?3, 0, ?4, ?4)",
        params![id, kind, input.to_string(), now()],
    )
    .map_err(|e| e.to_string())?;
    let job_id = id.clone();
    let kind_owned = kind.to_string();
    std::thread::spawn(move || {
        run(&ctx, &http, &job_id, &kind_owned, &input);
    });
    Ok(id)
}

/// 把一批已经结构化的题（网页 DOM 抓取）直接做成「已完成」任务。
///
/// 网页取题不需要起线程、也不需要模型：结构化结果已经在手里，
/// 但必须复用同一套预览 / 校对 / 存草稿流程，所以照样落一条 `generation_job`。
pub fn create_ready_job(
    conn: &Connection,
    kind: &str,
    input: Value,
    items: &[Value],
    subject_id: &str,
) -> Result<String, String> {
    let normalized: Vec<Value> = items
        .iter()
        .map(|value| normalize_draft(conn, subject_id, value, true))
        .collect();
    if normalized.is_empty() {
        return Err("这一页没抓到题目".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let result = json!({
        "items": normalized,
        "subjectName": subject_name(conn, subject_id),
        "imagePath": "",
    });
    conn.execute(
        "insert into generation_job (id, kind, status, progress, input_json, result_json, created_at, updated_at)
         values (?1, ?2, 'DONE', 100, ?3, ?4, ?5, ?5)",
        params![id, kind, input.to_string(), result.to_string(), now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// 预览里就地改一道题：只改任务结果，题库里什么都还没写。
/// 缺答案、缺知识点的题改齐了就能存草稿，不必「只能丢弃」。
pub fn update_item(
    conn: &Connection,
    job_id: &str,
    index: usize,
    patch: &Value,
) -> Result<Value, String> {
    let job = load_job(conn, job_id)?;
    if job.status == "COMMITTED" {
        return Err("这批题已经存过草稿了，去题库的草稿箱里改".into());
    }
    let mut result = job.result.ok_or_else(|| "这个任务还没有结果".to_string())?;
    // 除 classify（给已有题补元数据、本就展示知识点）外，所有整理题目链路
    // 都把「缺知识点」计入待补提示：知识点决定题目归章，缺了要在预览里看得见。
    let require_knowledge = job.kind != "classify";
    let items = result
        .get_mut("items")
        .and_then(|node| node.as_array_mut())
        .ok_or_else(|| "这份结果里没有题目".to_string())?;
    let item = items
        .get_mut(index)
        .ok_or_else(|| "这道题不在这份结果里".to_string())?;
    let map = item
        .as_object_mut()
        .ok_or_else(|| "这道题的数据不完整".to_string())?;

    // 题型：换成枚举值；非选择题一律把选项清空，避免「解答题带 A/B/C/D」。
    if let Some(kind) = patch.get("type").and_then(|node| node.as_str()) {
        if let Some(code) = text::normalize_type(kind) {
            map.insert("type".into(), json!(code));
            if !is_choice(code) {
                map.insert("options".into(), json!([]));
            }
        }
    }
    if let Some(options) = patch.get("options") {
        if is_choice(map.get("type").and_then(|node| node.as_str()).unwrap_or("")) {
            map.insert("options".into(), json!(options_of(Some(options))));
        }
    }
    for field in ["stem", "answer", "analysis"] {
        if let Some(value) = patch.get(field).and_then(|node| node.as_str()) {
            map.insert(field.into(), json!(value.trim()));
        }
    }
    if let Some(difficulty) = patch.get("difficulty").and_then(|node| node.as_i64()) {
        map.insert("difficulty".into(), json!(difficulty.clamp(1, 5)));
    }
    if let Some(knowledge) = patch.get("knowledgeIds") {
        let ids: Vec<String> = strings(Some(knowledge));
        let mut names = Vec::new();
        for id in &ids {
            let name: Option<String> = conn
                .query_row(
                    "select name from knowledge_point where id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()
                .ok()
                .flatten();
            if let Some(name) = name {
                names.push(name);
            }
        }
        map.insert("knowledgeIds".into(), json!(ids));
        map.insert("knowledgeNames".into(), json!(names));
    }

    // AI 制图：草稿预览里生成的图以 {ref, path} 形式带进来，合并进 item.figures。
    // 合并而不是覆盖，避免生成多张图时后一张把前一张冲掉；按 ref 去重，重生成同一引用
    // 时以后者为准。提交时 stage_figures 会按题面引用把这些临时图转成正式 asset。
    if let Some(figures) = patch.get("figures").and_then(|node| node.as_array()) {
        let mut merged: Vec<Value> = map
            .get("figures")
            .and_then(|node| node.as_array())
            .cloned()
            .unwrap_or_default();
        for figure in figures {
            let reference = figure.get("ref").and_then(|node| node.as_str()).unwrap_or("");
            if reference.is_empty() {
                continue;
            }
            merged.retain(|existing| {
                existing.get("ref").and_then(|node| node.as_str()) != Some(reference)
            });
            merged.push(figure.clone());
        }
        map.insert("figures".into(), json!(merged));
    }

    // 改完重算：题干补上了才可存，答案/解析补上了「待补」就消失。
    let stem = map
        .get("stem")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .to_string();
    let answer = map
        .get("answer")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .to_string();
    let analysis = map
        .get("analysis")
        .and_then(|node| node.as_str())
        .unwrap_or("")
        .to_string();
    let knowledge_ids = strings(map.get("knowledgeIds"));
    let kind = map
        .get("type")
        .and_then(|node| node.as_str())
        .unwrap_or("SOLVED")
        .to_string();
    let options: Vec<OptionItem> =
        serde_json::from_value(map.get("options").cloned().unwrap_or(json!([])))
            .unwrap_or_default();
    map.insert("blocking".into(), json!(blocking_issues(&stem)));
    map.insert(
        "issues".into(),
        json!(pending_fields(
            &kind,
            &options,
            &answer,
            &analysis,
            &knowledge_ids,
            require_knowledge,
        )),
    );

    let updated = item.clone();
    conn.execute(
        "update generation_job set result_json = ?2, updated_at = ?3 where id = ?1",
        params![job_id, result.to_string(), now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(updated)
}

fn input_of(conn: &Connection, id: &str) -> Result<Value, String> {
    conn.query_row(
        "select input_json from generation_job where id = ?1",
        [id],
        |row| row.get::<_, String>(0),
    )
    .map_err(|_| "任务不在了".to_string())
    .and_then(|raw| serde_json::from_str(&raw).map_err(|e| e.to_string()))
}

/// 图片任务先整批验原图：任何一张不可用，都不能先写进半批题目。
/// 文字与结构化网页任务没有原图，不走这道门槛。
fn preflight_capture_sources(
    kind: &str,
    items: &[Value],
    rows: &[usize],
    fallback_capture_path: Option<&str>,
) -> Result<(), String> {
    if !matches!(kind, "capture" | "capture_batch") {
        return Ok(());
    }
    for index in rows {
        let Some(item) = items.get(*index) else {
            continue;
        };
        let path = item
            .get("capturePath")
            .and_then(|node| node.as_str())
            .or(fallback_capture_path)
            .unwrap_or("")
            .trim();
        if path.is_empty() {
            return Err(format!("第 {} 条：找不到原图，请重新识别", index + 1));
        }
        if !Path::new(path).is_file() {
            return Err(format!("第 {} 条：原图不存在，请重新识别", index + 1));
        }
        let bytes = std::fs::read(path)
            .map_err(|error| format!("第 {} 条：读取原图失败（{error}），请重新识别", index + 1))?;
        if text::sniff_image(&bytes).is_none() {
            return Err(format!(
                "第 {} 条：原图不是支持的 JPG / PNG / WebP / GIF 图片，请重新识别",
                index + 1
            ));
        }
    }
    Ok(())
}

/// 把任务结果里勾选的条目写进题库。AI 题一律 `DRAFT + needs_review`。
pub fn commit(
    conn: &mut Connection,
    data_dir: &Path,
    job_id: &str,
    rows: &[usize],
) -> Result<Value, String> {
    let job = load_job(conn, job_id)?;
    if job.status != "DONE" {
        return Err("任务还没跑完".into());
    }
    let result = job.result.ok_or_else(|| "任务没有结果".to_string())?;
    let items = result
        .get("items")
        .and_then(|node| node.as_array())
        .cloned()
        .unwrap_or_default();

    let mut imported = 0;
    let mut skipped = 0;
    let mut needs_work = 0;
    let mut failed = Vec::new();
    let mut ids = Vec::new();
    let is_capture = matches!(
        job.kind.as_str(),
        "capture" | "capture_batch" | "capture_text" | "capture_web"
    );
    // 单图任务的兜底原图；批量/文字任务每题自带 capturePath。
    let fallback_capture_path = if job.kind == "capture" {
        input_of(conn, job_id)?
            .get("imagePath")
            .and_then(|node| node.as_str())
            .map(|text| text.to_string())
    } else {
        None
    };
    preflight_capture_sources(&job.kind, &items, rows, fallback_capture_path.as_deref())?;

    for index in rows {
        let Some(item) = items.get(*index) else {
            continue;
        };
        if let Some(error) = item.get("error").and_then(|node| node.as_str()) {
            skipped += 1;
            failed.push(format!("第 {} 条：{error}", index + 1));
            continue;
        }
        if job.kind == "classify" {
            match apply_classify(conn, item) {
                Ok(true) => imported += 1,
                Ok(false) => skipped += 1,
                Err(error) => {
                    skipped += 1;
                    failed.push(error);
                }
            }
            continue;
        }

        // 只有「没题干」才拦；缺答案 / 解析 / 知识点只是待补，草稿箱收得下。
        let blocking = item
            .get("blocking")
            .and_then(|node| node.as_array())
            .cloned()
            .unwrap_or_default();
        if !blocking.is_empty() {
            skipped += 1;
            failed.push(format!(
                "第 {} 条：{}",
                index + 1,
                blocking
                    .iter()
                    .filter_map(|value| value.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            ));
            continue;
        }
        let has_pending_fields = item
            .get("issues")
            .and_then(|node| node.as_array())
            .map(|list| !list.is_empty())
            .unwrap_or(false);

        let subject_id = item
            .get("subjectId")
            .and_then(|node| node.as_str())
            .map(|text| text.to_string());
        // 题目 id 先定下来：图形要按「assets/<题目 id>/」落盘，入库后正文才能引用。
        let draft_id = uuid::Uuid::new_v4().to_string();
        let figures = stage_figures(data_dir, &draft_id, item, &mut failed, *index);
        let stem = rewrite_figure_refs(
            item.get("stem")
                .and_then(|node| node.as_str())
                .unwrap_or(""),
            &figures,
        );
        let mut options: Vec<OptionItem> =
            serde_json::from_value(item.get("options").cloned().unwrap_or(json!([])))
                .unwrap_or_default();
        for option in &mut options {
            option.text = rewrite_figure_refs(&option.text, &figures);
        }
        let input = QuestionInput {
            id: Some(draft_id.clone()),
            subject_id,
            kind: item
                .get("type")
                .and_then(|node| node.as_str())
                .unwrap_or("SOLVED")
                .to_string(),
            stem,
            options,
            answer: item
                .get("answer")
                .and_then(|node| node.as_str())
                .unwrap_or("")
                .to_string(),
            analysis: item
                .get("analysis")
                .and_then(|node| node.as_str())
                .unwrap_or("")
                .to_string(),
            difficulty: item
                .get("difficulty")
                .and_then(|node| node.as_i64())
                .unwrap_or(3),
            tags: Vec::new(),
            knowledge_ids: strings(item.get("knowledgeIds")),
            source: item
                .get("source")
                .and_then(|node| node.as_str())
                .map(|text| text.to_string()),
            year: item.get("year").and_then(|node| node.as_i64()),
            status: Some("DRAFT".into()),
            origin: Some(if is_capture { "CAPTURE" } else { "AI" }.into()),
            needs_review: Some(true),
            // 存下来的是草稿，不是被驳回的题：复核前 review_note 保持空，
            // 否则题库详情会把新草稿显示成「已驳回：AI 生成…」。
            review_note: None,
        };

        match crate::repo::save(conn, &input) {
            Ok(id) => {
                // 正文里的 `![](图N)` 已换成 `![](nbasset:<id>)`，这里补上图片记录。
                // 截图是识别用的临时文件，不入库；题目只保留题干 / 选项里真正用到的图。
                insert_figure_assets(conn, &id, &figures);
                imported += 1;
                if has_pending_fields {
                    needs_work += 1;
                }
                ids.push(id);
            }
            Err(error) => {
                skipped += 1;
                failed.push(format!("第 {} 条：{error}", index + 1));
            }
        }
    }

    conn.execute(
        "update generation_job set status = 'COMMITTED', updated_at = ?2 where id = ?1",
        params![job_id, now()],
    )
    .map_err(|e| e.to_string())?;

    Ok(json!({
        "imported": imported,
        "skipped": skipped,
        "needsWork": needs_work,
        "questionIds": ids,
        "problems": failed,
    }))
}

/// 识别时裁出来的图形：入库前先落到 `assets/<题目 id>/`，正文里换成 `nbasset:<id>`。
struct FigureAsset {
    reference: String,
    id: String,
    relative: String,
    content_type: String,
    sha: String,
    width: i64,
    height: i64,
}

fn stage_figures(
    data_dir: &Path,
    question_id: &str,
    item: &Value,
    problems: &mut Vec<String>,
    index: usize,
) -> Vec<FigureAsset> {
    let raw = item
        .get("figures")
        .and_then(|node| node.as_array())
        .cloned()
        .unwrap_or_default();
    if raw.is_empty() {
        return Vec::new();
    }
    let dir = data_dir.join("assets").join(question_id);
    if std::fs::create_dir_all(&dir).is_err() {
        problems.push(format!("第 {} 条：图形目录建不出来", index + 1));
        return Vec::new();
    }
    // 只保存题干 / 选项里真正引用到的图；模型多给的 figures 不落库。
    let referenced = collect_figure_refs(item);
    let mut assets = Vec::new();
    for (position, figure) in raw.iter().enumerate() {
        let reference = figure
            .get("ref")
            .and_then(|node| node.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if reference.is_empty() || !referenced.iter().any(|ref_| ref_ == &reference) {
            continue;
        }
        let path = figure
            .get("path")
            .and_then(|node| node.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if reference.is_empty() || path.is_empty() {
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                problems.push(format!(
                    "第 {} 条：{reference} 的图没保留，复核时可以重新截图",
                    index + 1
                ));
                continue;
            }
        };
        let Some((mime, ext)) = text::sniff_image(&bytes) else {
            continue;
        };
        let file_name = format!("figure-{}.{ext}", position + 1);
        let target = dir.join(&file_name);
        if std::fs::write(&target, &bytes).is_err() {
            problems.push(format!("第 {} 条：{reference} 的图没写进去", index + 1));
            continue;
        }
        let (width, height) = image::image_dimensions(&target).unwrap_or((0, 0));
        let sha = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(&bytes))
        };
        assets.push(FigureAsset {
            reference,
            id: uuid::Uuid::new_v4().to_string(),
            relative: format!("{question_id}/{file_name}"),
            content_type: mime.to_string(),
            sha,
            width: width as i64,
            height: height as i64,
        });
    }
    assets
}

/// 题干与选项里出现的图形引用名（`![](图1)` 里的「图1」）。
fn collect_figure_refs(item: &Value) -> Vec<String> {
    let mut texts: Vec<String> = Vec::new();
    if let Some(stem) = item.get("stem").and_then(|node| node.as_str()) {
        texts.push(stem.to_string());
    }
    if let Some(options) = item.get("options").and_then(|node| node.as_array()) {
        for option in options {
            if let Some(text) = option.get("text").and_then(|node| node.as_str()) {
                texts.push(text.to_string());
            }
        }
    }
    let mut refs = Vec::new();
    for text in texts {
        let mut rest = text.as_str();
        while let Some(start) = rest.find("![") {
            let Some(end) = rest[start..].find(')') else {
                break;
            };
            let chunk = &rest[start..start + end + 1];
            if let Some(split) = chunk.find("](") {
                let target = chunk[split + 2..chunk.len() - 1].trim();
                if !target.is_empty() && !target.starts_with("nbasset:") {
                    refs.push(target.to_string());
                }
            }
            rest = &rest[start + end + 1..];
        }
    }
    refs
}

fn insert_figure_assets(conn: &Connection, question_id: &str, figures: &[FigureAsset]) {
    for (sort, figure) in figures.iter().enumerate() {
        let target = conn.execute(
            "insert into question_asset (id, question_id, kind, relative_path, content_type, sha256, width, height, sort)
             values (?1, ?2, 'FIGURE', ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                figure.id,
                question_id,
                figure.relative,
                figure.content_type,
                figure.sha,
                figure.width,
                figure.height,
                sort as i64
            ],
        );
        // 图片记录插不进去不该拖垮题目本身：正文里的引用会指向空图，复核时能补。
        let _ = target;
    }
}

/// 把正文里的 `![](图1)` 换成 `![](nbasset:<id>)`；没裁出来的占位直接清掉，
/// 免得入库后题干里留一串 `![](图2)` 的文本。
fn rewrite_figure_refs(text: &str, figures: &[FigureAsset]) -> String {
    let mut out = text.to_string();
    for figure in figures {
        out = out.replace(
            &format!("]({})", figure.reference),
            &format!("](nbasset:{})", figure.id),
        );
        out = out.replace(
            &format!("]（{}）", figure.reference),
            &format!("](nbasset:{})", figure.id),
        );
    }
    strip_unresolved_figures(&out)
}

fn strip_unresolved_figures(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("![") {
        let Some(end) = rest[start..].find(')') else {
            break;
        };
        let chunk = &rest[start..start + end + 1];
        if chunk.contains("nbasset:") {
            out.push_str(&rest[..start + end + 1]);
        } else {
            out.push_str(&rest[..start]);
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

fn apply_classify(conn: &mut Connection, item: &Value) -> Result<bool, String> {
    let Some(question_id) = item.get("questionId").and_then(|node| node.as_str()) else {
        return Ok(false);
    };
    let current = crate::repo::get(conn, question_id)?;
    let mut input = QuestionInput {
        id: Some(current.id.clone()),
        subject_id: current.subject_id.clone(),
        kind: current.kind.clone(),
        stem: current.stem.clone(),
        options: current.options.clone(),
        answer: current.answer.clone(),
        analysis: current.analysis.clone(),
        difficulty: current.difficulty,
        tags: current.tags.clone(),
        knowledge_ids: current
            .knowledge
            .iter()
            .map(|point| point.id.clone())
            .collect(),
        source: current.source.clone(),
        year: current.year,
        status: Some(current.status.clone()),
        origin: Some(current.origin.clone()),
        needs_review: Some(true),
        review_note: Some("AI 补充结果，确认后请复查".into()),
    };

    if let Some(subject) = item.get("subjectId").and_then(|node| node.as_str()) {
        if !subject.is_empty() {
            input.subject_id = Some(subject.to_string());
        }
    }
    if let Some(difficulty) = item.get("difficulty").and_then(|node| node.as_i64()) {
        input.difficulty = difficulty.clamp(1, 5);
    }
    // 知识点是唯一归类依据：把模型给的名字按（可能改过的）学科解析成本地 id，
    // 复用与识别链路同一套归一化匹配；没有名字时才退回已有的 id 列表。
    let names = strings(item.get("knowledgeNames"));
    if !names.is_empty() {
        let subject = input.subject_id.as_deref().unwrap_or("");
        let resolved: Vec<String> = names
            .iter()
            .filter_map(|name| resolve_point(conn, subject, name))
            .collect();
        if !resolved.is_empty() {
            input.knowledge_ids = resolved;
        }
    } else {
        let knowledge = strings(item.get("knowledgeIds"));
        if !knowledge.is_empty() {
            input.knowledge_ids = knowledge;
        }
    }
    let answer = item
        .get("answer")
        .and_then(|node| node.as_str())
        .unwrap_or("");
    let mut ai_answer = false;
    let mut ai_analysis = false;
    if input.answer.trim().is_empty() && !answer.trim().is_empty() {
        input.answer = answer.to_string();
        ai_answer = true;
    }
    let analysis = item
        .get("analysis")
        .and_then(|node| node.as_str())
        .unwrap_or("");
    if input.analysis.trim().is_empty() && !analysis.trim().is_empty() {
        input.analysis = analysis.to_string();
        ai_analysis = true;
    }

    crate::repo::save(conn, &input)?;
    if ai_answer || ai_analysis {
        crate::repo::mark_ai_fields(conn, question_id, ai_answer, ai_analysis)?;
    }
    Ok(true)
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use rusqlite::Connection;

    /// 单张「识别这张」失败时也要把队列项标成 FAILED：以前不带 queueId，
    /// 队列里永远停在「待识别」，人看着像这次识别没生效。
    #[test]
    fn failed_single_capture_marks_queue_failed() {
        let dir = std::env::temp_dir().join(format!("nb-queue-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("neuralkb.db");
        let mut conn = db::open(&db_path).unwrap();
        db::migrate(&mut conn).unwrap();
        let queued =
            crate::capture_store::enqueue(&conn, "/definitely-missing.png", None, "").unwrap();
        let ctx = JobContext {
            db_path: db_path.clone(),
        };
        let http = reqwest::blocking::Client::new();
        let error = run_capture(
            &ctx,
            &http,
            "job-missing",
            &json!({
                "imagePath": "/definitely-missing.png",
                "engine": "vision",
                "queueId": queued.id,
            }),
        )
        .unwrap_err();
        assert!(!error.is_empty());
        let after = crate::capture_store::get(&conn, &queued.id).unwrap();
        assert_eq!(after.status, "FAILED", "失败原因没回写队列");
        assert_eq!(after.job_id.as_deref(), Some("job-missing"));
        assert!(!after.error.is_empty(), "队列项要带上失败原因");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 裁图的同时要把归一化框留下来：前端靠它画可拖的框，没有它就只能从整张图重框。
    #[test]
    fn extract_figures_keeps_normalized_box() {
        let dir = std::env::temp_dir().join(format!("nb-fig-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let shot = dir.join("shot.png");
        image::RgbaImage::from_pixel(400, 200, image::Rgba([250, 250, 250, 255]))
            .save(&shot)
            .unwrap();
        let mut item = json!({});
        let source = json!({"figures": [{"ref": "图1", "bbox": [100, 50, 200, 100]}]});
        let warnings = extract_figures(&shot.to_string_lossy(), &dir, "job-x", &source, &mut item);
        assert!(warnings.is_empty(), "{warnings:?}");
        let figures = item["figures"].as_array().unwrap();
        assert_eq!(figures.len(), 1);
        assert!(figures[0]["path"].as_str().unwrap().ends_with(".jpg"));
        let bbox = &figures[0]["box"];
        // 0–1000 相对坐标 → 0–1：100/1000 宽、50/1000 高
        assert!((bbox["x"].as_f64().unwrap() - 0.1).abs() < 1e-6, "{bbox}");
        assert!((bbox["y"].as_f64().unwrap() - 0.05).abs() < 1e-6, "{bbox}");
        assert!((bbox["w"].as_f64().unwrap() - 0.2).abs() < 1e-6, "{bbox}");
        assert!((bbox["h"].as_f64().unwrap() - 0.1).abs() < 1e-6, "{bbox}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 网页抓来的难度可能是数字、中文或「中等」，归一化后统一落在 1–5。
    #[test]
    fn difficulty_reads_numbers_and_words() {
        assert_eq!(difficulty_of(Some(&json!(5))), 5);
        assert_eq!(difficulty_of(Some(&json!(3.0))), 3);
        assert_eq!(difficulty_of(Some(&json!("4"))), 4);
        assert_eq!(difficulty_of(Some(&json!("很容易"))), 1);
        assert_eq!(difficulty_of(Some(&json!("很难"))), 5);
        assert_eq!(difficulty_of(Some(&json!("较难"))), 4);
        assert_eq!(difficulty_of(Some(&json!("容易"))), 2);
        assert_eq!(difficulty_of(Some(&json!(""))), 3);
        assert_eq!(difficulty_of(None), 3);
    }

    /// 模型写知识点标题时常带空格、书名号、括号注释、全角标点；精确匹配漏掉的
    /// 这些写法，归一化后应能命中本地目录，本学科优先。
    #[test]
    fn resolve_point_matches_normalized_knowledge_names() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "create table knowledge_point (id text, subject_id text, name text);
             insert into knowledge_point values
               ('kp-1','subj-MATH','勾股定理'),
               ('kp-2','subj-PHYS','力的合成');",
        )
        .unwrap();
        // 精确匹配仍走原样
        assert_eq!(
            resolve_point(&conn, "subj-MATH", "勾股定理").as_deref(),
            Some("kp-1")
        );
        // 括号注释 / 书名号 / 空格：本学科命中
        assert_eq!(
            resolve_point(&conn, "subj-MATH", "《勾股定理》（八年级）").as_deref(),
            Some("kp-1")
        );
        // 全角空格 + 后缀
        assert_eq!(
            resolve_point(&conn, "subj-MATH", "勾 股 定 理　").as_deref(),
            Some("kp-1")
        );
        // 本学科对不上时回落全局
        assert_eq!(
            resolve_point(&conn, "subj-MATH", "力的合成").as_deref(),
            Some("kp-2")
        );
        // 完全对不上仍返回 None
        assert_eq!(resolve_point(&conn, "subj-MATH", " Boltzmann 分布"), None);
    }

    /// 旧题块只给 examPoints 时按知识点名解析复用，别让题进池后抽不到。
    #[test]
    fn legacy_exam_points_reuse_local_knowledge() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "create table knowledge_point (id text, subject_id text, name text);
             insert into knowledge_point values ('kp-1','subj-MATH','集合');",
        )
        .unwrap();
        let item = normalize_draft(
            &conn,
            "subj-MATH",
            &json!({
                "type": "SINGLE",
                "stem": "设集合 A={1,2}，子集个数是（ ）",
                "options": [{"key": "A", "text": "2"}, {"key": "B", "text": "3"}],
                "answer": "B",
                "examPoints": ["集合"],
                "source": "https://example.com/q/1",
            }),
            false,
        );
        assert_eq!(item["knowledgeNames"], json!(["集合"]));
        assert_eq!(item["knowledgeIds"], json!(["kp-1"]));
        assert!(
            item.get("examPoints").is_none(),
            "归一化结果里不再有独立考点字段"
        );
        assert_eq!(item["difficulty"], 3);
        assert_eq!(item["blocking"], json!([]));
        assert_eq!(item["source"], "https://example.com/q/1");
        assert!(!item["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue.as_str().unwrap_or("").contains("选项")));
    }

    #[test]
    fn invalid_choice_answer_is_a_pending_issue_not_a_blocker() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("create table knowledge_point (id text, subject_id text, name text);")
            .unwrap();
        let item = normalize_draft(
            &conn,
            "subj-MATH",
            &json!({
                "type": "SINGLE",
                "stem": "1+1 等于几？",
                "options": [{"key": "A", "text": "1"}, {"key": "B", "text": "2"}],
                "answer": "C",
                "analysis": "1+1=2。",
                "knowledgeNames": ["加法"],
            }),
            false,
        );
        assert_eq!(item["blocking"], json!([]));
        assert!(item["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue == "选择题答案只能填实际选项的字母"));
    }

    /// 纯字符串选项数组也要能收。
    #[test]
    fn plain_string_options_become_keys() {
        let options = options_of(Some(&json!(["2", "3", "4"])));
        assert_eq!(options[0].key, "A");
        assert_eq!(options[2].text, "4");
    }
}
