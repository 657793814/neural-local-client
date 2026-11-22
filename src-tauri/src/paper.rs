//! 试卷：组卷、分值、快照、版面与打印记录。
//!
//! 设计见 `docs/product/local-paper-management-plan.md`：
//! - 试卷只存「编排」，题目内容固化在 `snapshot_json` 里，题库改题不影响历史卷；
//! - 总分是合同值，定稿要求 `sum(题分) == total_score`，不做静默凑数；
//! - 大题默认统一分值（改 unit_score 即整题重算），也允许逐题微调（切 PER_ITEM）；
//! - 排版与打印在前端完成，后端只负责取数、存快照、记录打印。

use crate::models::Page;
use crate::state::AppState;
use crate::text;
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use tauri::State;

// ---------------------------------------------------------------------------
// 数据形状（camelCase，前端直接当 JS 对象用）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperSummary {
    pub id: String,
    pub title: String,
    pub subject_id: Option<String>,
    pub subject_name: String,
    pub grade: String,
    pub exam_type: String,
    pub status: String,
    pub total_score: f64,
    pub assigned_score: f64,
    pub duration_min: i64,
    pub question_count: i64,
    pub score_scheme: String,
    pub created_at: String,
    pub updated_at: String,
    pub finalized_at: Option<String>,
    pub deleted_at: Option<String>,
    /// 打印记录：打过几次、上次什么时候（列表上直接显示，不必再点进去）。
    pub print_count: i64,
    pub last_printed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperQuestion {
    pub ord: i64,
    pub question_id: Option<String>,
    pub score: f64,
    pub snapshot: Value,
    pub flags: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperSection {
    pub id: String,
    pub ord: i64,
    pub title: String,
    /// 大题说明：库里没写时按分值规则自动生成，避免「每小题 X 分」两处各算一遍。
    pub note: String,
    pub kind: String,
    pub score_mode: String,
    pub unit_score: f64,
    pub layout: Value,
    pub score_total: f64,
    pub questions: Vec<PaperQuestion>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperDetail {
    pub id: String,
    pub title: String,
    pub subject_id: Option<String>,
    pub subject_name: String,
    pub grade: String,
    pub exam_type: String,
    pub status: String,
    pub total_score: f64,
    pub assigned_score: f64,
    pub duration_min: i64,
    pub question_count: i64,
    pub score_scheme: String,
    pub layout: Value,
    pub blueprint: Value,
    pub slots: Value,
    pub note: String,
    pub created_at: String,
    pub updated_at: String,
    pub finalized_at: Option<String>,
    pub deleted_at: Option<String>,
    pub sections: Vec<PaperSection>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionScore {
    pub section_id: String,
    pub title: String,
    pub count: i64,
    pub score_mode: String,
    pub unit_score: f64,
    pub assigned: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreCheck {
    pub total: f64,
    pub assigned: f64,
    pub delta: f64,
    pub balanced: bool,
    pub sections: Vec<SectionScore>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PaperQuery {
    pub keyword: Option<String>,
    pub subject_id: Option<String>,
    /// DRAFT / FINAL / ARCHIVED；空表示「未归档的全部」。
    pub status: Option<String>,
    pub trashed: Option<bool>,
    pub page: Option<i64>,
    pub size: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PaperCreateInput {
    pub title: String,
    pub subject_id: Option<String>,
    pub grade: String,
    pub exam_type: String,
    pub score_scheme: String,
    pub duration_min: Option<i64>,
    pub layout: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PaperPatch {
    pub title: Option<String>,
    pub subject_id: Option<String>,
    pub grade: Option<String>,
    pub exam_type: Option<String>,
    pub duration_min: Option<i64>,
    pub layout: Option<Value>,
    pub slots: Option<Value>,
    pub note: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PaperSectionInput {
    pub id: Option<String>,
    pub title: String,
    pub note: String,
    pub kind: String,
    pub score_mode: String,
    pub unit_score: Option<f64>,
    pub layout: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScoreItem {
    pub section_id: String,
    pub ord: i64,
    pub score: f64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SnapshotItem {
    pub section_id: String,
    pub ord: i64,
}

// ---------------------------------------------------------------------------
// 默认值
// ---------------------------------------------------------------------------

/// 新建试卷写入的默认版面（字段只增不改，见规划 §6.4）。
pub fn default_layout() -> Value {
    json!({
        "paperSize": "A4",
        "orientation": "portrait",
        "margins": {"top": 20, "right": 20, "bottom": 20, "left": 20},
        "columns": 1,
        "fontFamily": "",
        "fontSize": 10.5,
        "lineHeight": 1.6,
        "questionGap": 6,
        "optionColumns": 4,
        "answerSpace": "auto",
        "bindingLine": false,
        "watermark": null,
        "answerKeyPage": true,
        "answerSheet": {"enabled": true, "templateId": "builtin-generic", "paperSize": "A4", "columns": 2}
    })
}

/// 内置标准卷方案：只是预填结构与分值，不锁死，老师可以随便改。
pub fn schemes() -> Value {
    json!([
        {
            "id": "XGK1_MATH",
            "name": "新高考Ⅰ卷 · 数学",
            "totalScore": 150,
            "durationMin": 120,
            "sections": [
                {"title": "一、单项选择题", "kind": "SINGLE", "count": 8, "unitScore": 5, "scoreMode": "UNIFORM"},
                {"title": "二、多项选择题", "kind": "MULTI", "count": 3, "unitScore": 6, "scoreMode": "UNIFORM"},
                {"title": "三、填空题", "kind": "FILL", "count": 3, "unitScore": 5, "scoreMode": "UNIFORM"},
                {"title": "四、解答题", "kind": "CALC", "count": 5, "unitScore": 0, "scoreMode": "PER_ITEM"}
            ]
        },
        {
            "id": "GENERAL_100",
            "name": "通用 100 分卷",
            "totalScore": 100,
            "durationMin": 90,
            "sections": [
                {"title": "一、单项选择题", "kind": "SINGLE", "count": 10, "unitScore": 4, "scoreMode": "UNIFORM"},
                {"title": "二、多项选择题", "kind": "MULTI", "count": 5, "unitScore": 4, "scoreMode": "UNIFORM"},
                {"title": "三、填空题", "kind": "FILL", "count": 4, "unitScore": 5, "scoreMode": "UNIFORM"},
                {"title": "四、解答题", "kind": "CALC", "count": 2, "unitScore": 10, "scoreMode": "UNIFORM"}
            ]
        },
        {
            "id": "CUSTOM",
            "name": "自定义（100 分起）",
            "totalScore": 100,
            "durationMin": 90,
            "sections": [
                {"title": "一、试题", "kind": "", "count": 0, "unitScore": 5, "scoreMode": "UNIFORM"}
            ]
        }
    ])
}

fn scheme_of(id: &str) -> Option<Value> {
    schemes()
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
}

// ---------------------------------------------------------------------------
// 读写
// ---------------------------------------------------------------------------

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn json_of(raw: String) -> Value {
    serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({}))
}

/// 分值显示：整数不带小数点，避免卷面上出现「5.0 分」。
fn score_text(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{}", rounded.round() as i64)
    } else {
        let text = format!("{rounded:.2}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn derive_note(mode: &str, count: i64, unit: f64, total: f64) -> String {
    if count <= 0 {
        return String::new();
    }
    if mode == "UNIFORM" && unit > 0.0 {
        format!(
            "本大题共 {count} 小题，每小题 {} 分，共 {} 分",
            score_text(unit),
            score_text(unit * count as f64)
        )
    } else {
        format!("本大题共 {count} 小题，共 {} 分", score_text(total))
    }
}

const SELECT_PAPER: &str = "select p.id, p.title, p.subject_id, coalesce(s.name, '') as subject_name, \
     p.grade, p.exam_type, p.status, p.total_score, p.duration_min, p.question_count, \
     p.score_scheme, p.layout_json, p.blueprint_json, p.slots_json, p.note, \
     p.created_at, p.updated_at, p.finalized_at, p.deleted_at, \
     (select coalesce(sum(q.score), 0) from local_paper_question q where q.paper_id = p.id) as assigned \
     from local_paper p left join subject s on s.id = p.subject_id";

fn map_paper(row: &rusqlite::Row<'_>) -> rusqlite::Result<PaperDetail> {
    Ok(PaperDetail {
        id: row.get(0)?,
        title: row.get(1)?,
        subject_id: row.get(2)?,
        subject_name: row.get(3)?,
        grade: row.get(4)?,
        exam_type: row.get(5)?,
        status: row.get(6)?,
        total_score: row.get(7)?,
        duration_min: row.get(8)?,
        question_count: row.get(9)?,
        score_scheme: row.get(10)?,
        layout: json_of(row.get::<_, String>(11)?),
        blueprint: json_of(row.get::<_, String>(12)?),
        slots: json_of(row.get::<_, String>(13)?),
        note: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        finalized_at: row.get(17)?,
        deleted_at: row.get(18)?,
        assigned_score: row.get(19)?,
        sections: Vec::new(),
    })
}

fn load_sections(conn: &Connection, paper_id: &str) -> Result<Vec<PaperSection>, String> {
    let mut stmt = conn
        .prepare(
            "select id, ord, title, note, kind, score_mode, unit_score, layout_json
             from local_paper_section where paper_id = ?1 order by ord, id",
        )
        .map_err(|e| e.to_string())?;
    let raw: Vec<(String, i64, String, String, String, String, f64, Value)> = stmt
        .query_map([paper_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                json_of(row.get::<_, String>(7)?),
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut qstmt = conn
        .prepare(
            "select ord, question_id, score, snapshot_json, flags_json
             from local_paper_question where paper_id = ?1 and section_id = ?2 order by ord",
        )
        .map_err(|e| e.to_string())?;

    let mut out = Vec::with_capacity(raw.len());
    for (id, ord, title, note, kind, score_mode, unit_score, layout) in raw {
        let questions: Vec<PaperQuestion> = qstmt
            .query_map(params![paper_id, &id], |row| {
                Ok(PaperQuestion {
                    ord: row.get(0)?,
                    question_id: row.get(1)?,
                    score: row.get(2)?,
                    snapshot: json_of(row.get::<_, String>(3)?),
                    flags: json_of(row.get::<_, String>(4)?),
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        let total: f64 = questions.iter().map(|item| item.score).sum();
        let derived = if note.trim().is_empty() {
            derive_note(&score_mode, questions.len() as i64, unit_score, total)
        } else {
            note
        };
        out.push(PaperSection {
            id,
            ord,
            title,
            note: derived,
            kind,
            score_mode,
            unit_score,
            layout,
            score_total: total,
            questions,
        });
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: &str) -> Result<PaperDetail, String> {
    let mut paper = conn
        .query_row(&format!("{SELECT_PAPER} where p.id = ?1"), [id], map_paper)
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "这份试卷不在了，可能已被删除".to_string())?;
    paper.sections = load_sections(conn, id)?;
    Ok(paper)
}

pub fn list(conn: &Connection, query: &PaperQuery) -> Result<Page<PaperSummary>, String> {
    let size = query.size.unwrap_or(30).clamp(1, 100);
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * size;
    let trashed = query.trashed.unwrap_or(false);

    let mut where_sql = String::from(" where ");
    let mut values: Vec<Box<dyn ToSql>> = Vec::new();
    if trashed {
        where_sql.push_str("p.deleted_at is not null");
    } else {
        where_sql.push_str("p.deleted_at is null");
        let status = query.status.as_deref().unwrap_or("").trim();
        if !status.is_empty() {
            where_sql.push_str(" and p.status = ?");
            values.push(Box::new(status.to_string()));
        } else {
            // 归档卷默认收起来，需要时在筛选里显式选「已归档」。
            where_sql.push_str(" and p.status <> 'ARCHIVED'");
        }
    }
    if let Some(subject) = query
        .subject_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        where_sql.push_str(" and p.subject_id = ?");
        values.push(Box::new(subject.to_string()));
    }
    if let Some(keyword) = query.keyword.as_deref() {
        let trimmed = keyword.trim();
        if !trimmed.is_empty() {
            where_sql.push_str(" and (p.title like ? or p.note like ? or p.grade like ?)");
            let pattern = format!("%{trimmed}%");
            for _ in 0..3 {
                values.push(Box::new(pattern.clone()));
            }
        }
    }

    let total: i64 = conn
        .query_row(
            &format!("select count(*) from local_paper p{where_sql}"),
            params_from_iter(values.iter().map(|value| &**value as &dyn ToSql)),
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let sql = format!(
        "select p.id, p.title, p.subject_id, coalesce(s.name, ''), p.grade, p.exam_type, p.status, \
         p.total_score, p.duration_min, p.question_count, p.score_scheme, p.created_at, p.updated_at, \
         p.finalized_at, p.deleted_at, \
         (select coalesce(sum(q.score), 0) from local_paper_question q where q.paper_id = p.id), \
         (select count(*) from local_paper_print pr where pr.paper_id = p.id), \
         (select max(pr.created_at) from local_paper_print pr where pr.paper_id = p.id) \
         from local_paper p left join subject s on s.id = p.subject_id{where_sql} \
         order by p.updated_at desc limit ? offset ?"
    );
    let mut all = values;
    all.push(Box::new(size));
    all.push(Box::new(offset));
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let items: Vec<PaperSummary> = stmt
        .query_map(
            params_from_iter(all.iter().map(|value| &**value as &dyn ToSql)),
            |row| {
                Ok(PaperSummary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    subject_id: row.get(2)?,
                    subject_name: row.get(3)?,
                    grade: row.get(4)?,
                    exam_type: row.get(5)?,
                    status: row.get(6)?,
                    total_score: row.get(7)?,
                    duration_min: row.get(8)?,
                    question_count: row.get(9)?,
                    score_scheme: row.get(10)?,
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                    finalized_at: row.get(13)?,
                    deleted_at: row.get(14)?,
                    assigned_score: row.get(15)?,
                    print_count: row.get(16)?,
                    last_printed_at: row.get(17)?,
                })
            },
        )
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(Page {
        items,
        total,
        page,
        size,
    })
}

fn touch(conn: &Connection, paper_id: &str) -> Result<(), String> {
    conn.execute(
        "update local_paper set updated_at = ?2,
            question_count = (select count(*) from local_paper_question where paper_id = ?1)
         where id = ?1",
        params![paper_id, text::now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn create(conn: &mut Connection, input: &PaperCreateInput) -> Result<PaperDetail, String> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err("给试卷起个名字吧".to_string());
    }
    let scheme_id = if input.score_scheme.trim().is_empty() {
        "CUSTOM"
    } else {
        input.score_scheme.trim()
    };
    let scheme = scheme_of(scheme_id).ok_or_else(|| format!("分值方案不存在：{scheme_id}"))?;
    let total = scheme
        .get("totalScore")
        .and_then(Value::as_f64)
        .unwrap_or(100.0);
    let duration = input.duration_min.unwrap_or_else(|| {
        scheme
            .get("durationMin")
            .and_then(Value::as_i64)
            .unwrap_or(90)
    });
    let layout = input.layout.clone().unwrap_or_else(default_layout);
    let id = new_id();
    let now = text::now();
    let exam_type = if input.exam_type.trim().is_empty() {
        "EXAM"
    } else {
        input.exam_type.trim()
    };
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "insert into local_paper (id, title, subject_id, grade, exam_type, status, total_score,
            score_scheme, duration_min, question_count, layout_json, created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, 'DRAFT', ?6, ?7, ?8, 0, ?9, ?10, ?10)",
        params![
            id,
            title,
            input.subject_id.clone().filter(|value| !value.is_empty()),
            input.grade.trim(),
            exam_type,
            total,
            scheme_id,
            duration,
            serde_json::to_string(&layout).unwrap_or_else(|_| "{}".into()),
            now
        ],
    )
    .map_err(|e| e.to_string())?;

    if let Some(sections) = scheme.get("sections").and_then(Value::as_array) {
        for (index, section) in sections.iter().enumerate() {
            let mode = section
                .get("scoreMode")
                .and_then(Value::as_str)
                .unwrap_or("UNIFORM");
            tx.execute(
                "insert into local_paper_section (id, paper_id, ord, title, note, kind, score_mode, unit_score, layout_json)
                 values (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, '{}')",
                params![
                    new_id(),
                    id,
                    index as i64,
                    section.get("title").and_then(Value::as_str).unwrap_or(""),
                    section.get("kind").and_then(Value::as_str).unwrap_or(""),
                    mode,
                    section.get("unitScore").and_then(Value::as_f64).unwrap_or(0.0)
                ],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    get(conn, &id)
}

pub fn update(conn: &mut Connection, id: &str, patch: &PaperPatch) -> Result<PaperDetail, String> {
    let current = get(conn, id)?;
    if current.deleted_at.is_some() {
        return Err("这份试卷在回收站里，先恢复再改".to_string());
    }
    if current.status != "DRAFT" {
        return Err("已定稿的试卷是只读的，先「重新打开」再改".to_string());
    }
    let title = patch
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| current.title.as_str())
        .to_string();
    let layout = patch.layout.clone().unwrap_or(current.layout);
    let slots = patch.slots.clone().unwrap_or(current.slots);
    conn.execute(
        "update local_paper set title = ?2, subject_id = ?3, grade = ?4, exam_type = ?5,
            duration_min = ?6, layout_json = ?7, slots_json = ?8, note = ?9, updated_at = ?10
         where id = ?1",
        params![
            id,
            title,
            patch
                .subject_id
                .clone()
                .or(current.subject_id.clone())
                .filter(|value| !value.is_empty()),
            patch.grade.clone().unwrap_or(current.grade),
            patch.exam_type.clone().unwrap_or(current.exam_type),
            patch.duration_min.unwrap_or(current.duration_min),
            serde_json::to_string(&layout).unwrap_or_else(|_| "{}".into()),
            serde_json::to_string(&slots).unwrap_or_else(|_| "{}".into()),
            patch.note.clone().unwrap_or(current.note),
            text::now()
        ],
    )
    .map_err(|e| e.to_string())?;
    get(conn, id)
}

pub fn set_total(
    conn: &mut Connection,
    id: &str,
    total: f64,
    confirm: bool,
) -> Result<PaperDetail, String> {
    if !confirm {
        return Err("改总分会让已有分值全部重算，需要先确认".to_string());
    }
    if total <= 0.0 {
        return Err("总分要大于 0".to_string());
    }
    let paper = get(conn, id)?;
    if paper.status != "DRAFT" {
        return Err("已定稿的试卷先「重新打开」才能改总分".to_string());
    }
    conn.execute(
        "update local_paper set total_score = ?2, updated_at = ?3 where id = ?1",
        params![id, total, text::now()],
    )
    .map_err(|e| e.to_string())?;
    get(conn, id)
}

/// 套用标准卷方案：已有大题按题型/标题对齐更新分值口径，不删除已有题。
pub fn apply_scheme(
    conn: &mut Connection,
    id: &str,
    scheme_id: &str,
) -> Result<PaperDetail, String> {
    let scheme = scheme_of(scheme_id).ok_or_else(|| format!("分值方案不存在：{scheme_id}"))?;
    let paper = get(conn, id)?;
    if paper.status != "DRAFT" {
        return Err("已定稿的试卷不能套用方案".to_string());
    }
    let total = scheme
        .get("totalScore")
        .and_then(Value::as_f64)
        .unwrap_or(paper.total_score);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "update local_paper set total_score = ?2, score_scheme = ?3, updated_at = ?4 where id = ?1",
        params![id, total, scheme_id, text::now()],
    )
    .map_err(|e| e.to_string())?;

    if paper.sections.is_empty() {
        if let Some(sections) = scheme.get("sections").and_then(Value::as_array) {
            for (index, section) in sections.iter().enumerate() {
                tx.execute(
                    "insert into local_paper_section (id, paper_id, ord, title, note, kind, score_mode, unit_score, layout_json)
                     values (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, '{}')",
                    params![
                        new_id(),
                        id,
                        index as i64,
                        section.get("title").and_then(Value::as_str).unwrap_or(""),
                        section.get("kind").and_then(Value::as_str).unwrap_or(""),
                        section.get("scoreMode").and_then(Value::as_str).unwrap_or("UNIFORM"),
                        section.get("unitScore").and_then(Value::as_f64).unwrap_or(0.0)
                    ],
                )
                .map_err(|e| e.to_string())?;
            }
        }
    } else if let Some(sections) = scheme.get("sections").and_then(Value::as_array) {
        for section in paper.sections.iter() {
            let hit = sections.iter().find(|item| {
                let kind = item.get("kind").and_then(Value::as_str).unwrap_or("");
                let title = item.get("title").and_then(Value::as_str).unwrap_or("");
                (!section.kind.is_empty() && section.kind == kind) || section.title == title
            });
            if let Some(hit) = hit {
                let mode = hit
                    .get("scoreMode")
                    .and_then(Value::as_str)
                    .unwrap_or("UNIFORM");
                let unit = hit.get("unitScore").and_then(Value::as_f64).unwrap_or(0.0);
                tx.execute(
                    "update local_paper_section set score_mode = ?2, unit_score = ?3 where id = ?1",
                    params![section.id, mode, unit],
                )
                .map_err(|e| e.to_string())?;
                if mode == "UNIFORM" {
                    tx.execute(
                        "update local_paper_question set score = ?2 where paper_id = ?1 and section_id = ?3",
                        params![id, unit, section.id],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    get(conn, id)
}

pub fn score_check(conn: &Connection, id: &str) -> Result<ScoreCheck, String> {
    let paper = get(conn, id)?;
    let sections: Vec<SectionScore> = paper
        .sections
        .iter()
        .map(|section| SectionScore {
            section_id: section.id.clone(),
            title: section.title.clone(),
            count: section.questions.len() as i64,
            score_mode: section.score_mode.clone(),
            unit_score: section.unit_score,
            assigned: section.questions.iter().map(|item| item.score).sum(),
        })
        .collect();
    let assigned: f64 = sections.iter().map(|item| item.assigned).sum();
    let delta = ((paper.total_score - assigned) * 1000.0).round() / 1000.0;
    Ok(ScoreCheck {
        total: paper.total_score,
        assigned,
        delta,
        balanced: delta.abs() < 0.001,
        sections,
    })
}

pub fn finalize(conn: &mut Connection, id: &str) -> Result<PaperDetail, String> {
    let paper = get(conn, id)?;
    if paper.status == "FINAL" {
        return Ok(paper);
    }
    if paper.status != "DRAFT" {
        return Err("只有草稿能定稿".to_string());
    }
    let check = score_check(conn, id)?;
    if paper.question_count == 0 {
        return Err("卷子里还没有题，先加入题目再定稿".to_string());
    }
    if !check.balanced {
        let direction = if check.delta > 0.0 {
            "还差"
        } else {
            "多了"
        };
        let adjustable = check
            .sections
            .iter()
            .filter(|item| item.count > 0)
            .map(|item| {
                format!(
                    "{}（{} 题，现 {} 分）",
                    item.title,
                    item.count,
                    score_text(item.assigned)
                )
            })
            .collect::<Vec<_>>()
            .join("、");
        return Err(format!(
            "分值不平：已分配 {} 分，总分 {} 分，{} {} 分。可调整：{}",
            score_text(check.assigned),
            score_text(check.total),
            direction,
            score_text(check.delta.abs()),
            if adjustable.is_empty() {
                "（还没有题目）".to_string()
            } else {
                adjustable
            }
        ));
    }
    let now = text::now();
    conn.execute(
        "update local_paper set status = 'FINAL', finalized_at = ?2, updated_at = ?2 where id = ?1",
        params![id, now],
    )
    .map_err(|e| e.to_string())?;
    get(conn, id)
}

pub fn set_status(conn: &mut Connection, id: &str, status: &str) -> Result<PaperDetail, String> {
    let allowed = ["DRAFT", "FINAL", "ARCHIVED"];
    if !allowed.contains(&status) {
        return Err("未知的试卷状态".to_string());
    }
    let paper = get(conn, id)?;
    if paper.deleted_at.is_some() {
        return Err("这份试卷在回收站里".to_string());
    }
    let now = text::now();
    conn.execute(
        "update local_paper set status = ?2, updated_at = ?3,
            finalized_at = case when ?2 = 'DRAFT' then null else coalesce(finalized_at, ?3) end
         where id = ?1",
        params![id, status, now],
    )
    .map_err(|e| e.to_string())?;
    get(conn, id)
}

pub fn duplicate(conn: &mut Connection, id: &str) -> Result<PaperDetail, String> {
    let source = get(conn, id)?;
    let new_paper = new_id();
    let now = text::now();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "insert into local_paper (id, title, subject_id, grade, exam_type, status, total_score,
            score_scheme, duration_min, question_count, layout_json, blueprint_json, slots_json, note,
            created_at, updated_at)
         select ?1, title || '（副本）', subject_id, grade, exam_type, 'DRAFT', total_score,
            score_scheme, duration_min, question_count, layout_json, blueprint_json, slots_json, note,
            ?2, ?2 from local_paper where id = ?3",
        params![new_paper, now, id],
    )
    .map_err(|e| e.to_string())?;
    for section in source.sections.iter() {
        let section_id = new_id();
        tx.execute(
            "insert into local_paper_section (id, paper_id, ord, title, note, kind, score_mode, unit_score, layout_json)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                section_id,
                new_paper,
                section.ord,
                section.title,
                section.note,
                section.kind,
                section.score_mode,
                section.unit_score,
                serde_json::to_string(&section.layout).unwrap_or_else(|_| "{}".into())
            ],
        )
        .map_err(|e| e.to_string())?;
        for question in section.questions.iter() {
            tx.execute(
                "insert into local_paper_question (paper_id, section_id, ord, question_id, score, snapshot_json, snapshot_at, flags_json)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    new_paper,
                    section_id,
                    question.ord,
                    question.question_id,
                    question.score,
                    serde_json::to_string(&question.snapshot).unwrap_or_else(|_| "{}".into()),
                    now,
                    serde_json::to_string(&question.flags).unwrap_or_else(|_| "{}".into())
                ],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    get(conn, &new_paper)
}

pub fn soft_delete(conn: &Connection, id: &str, deleted: bool) -> Result<(), String> {
    conn.execute(
        "update local_paper set deleted_at = ?2, updated_at = ?3 where id = ?1",
        params![
            id,
            if deleted { Some(text::now()) } else { None },
            text::now()
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn ensure_draft(conn: &Connection, paper_id: &str) -> Result<(), String> {
    let (status, deleted): (String, Option<String>) = conn
        .query_row(
            "select status, deleted_at from local_paper where id = ?1",
            [paper_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "这份试卷不在了".to_string())?;
    if deleted.is_some() {
        return Err("这份试卷在回收站里，先恢复再改".to_string());
    }
    if status != "DRAFT" {
        return Err("已定稿的试卷是只读的，先「重新打开」再改".to_string());
    }
    Ok(())
}

pub fn save_section(
    conn: &mut Connection,
    paper_id: &str,
    input: &PaperSectionInput,
) -> Result<PaperDetail, String> {
    ensure_draft(conn, paper_id)?;
    let mode = if input.score_mode == "PER_ITEM" {
        "PER_ITEM"
    } else {
        "UNIFORM"
    };
    let unit = input.unit_score.unwrap_or(0.0);
    let layout = input.layout.clone().unwrap_or_else(|| json!({}));
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    match input.id.as_deref().filter(|value| !value.is_empty()) {
        Some(section_id) => {
            let exists: Option<i64> = tx
                .query_row(
                    "select 1 from local_paper_section where id = ?1 and paper_id = ?2",
                    params![section_id, paper_id],
                    |row| Ok(row.get(0)?),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if exists.is_none() {
                return Err("这个大题不在这份试卷里".to_string());
            }
            tx.execute(
                "update local_paper_section set title = ?2, note = ?3, kind = ?4,
                    score_mode = ?5, unit_score = ?6, layout_json = ?7 where id = ?1",
                params![
                    section_id,
                    input.title.trim(),
                    input.note.trim(),
                    input.kind.trim(),
                    mode,
                    unit,
                    serde_json::to_string(&layout).unwrap_or_else(|_| "{}".into())
                ],
            )
            .map_err(|e| e.to_string())?;
            if mode == "UNIFORM" {
                tx.execute(
                    "update local_paper_question set score = ?2 where paper_id = ?1 and section_id = ?3",
                    params![paper_id, unit, section_id],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        None => {
            let ord: i64 = tx
                .query_row(
                    "select coalesce(max(ord), -1) + 1 from local_paper_section where paper_id = ?1",
                    [paper_id],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            tx.execute(
                "insert into local_paper_section (id, paper_id, ord, title, note, kind, score_mode, unit_score, layout_json)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    new_id(),
                    paper_id,
                    ord,
                    input.title.trim(),
                    input.note.trim(),
                    input.kind.trim(),
                    mode,
                    unit,
                    serde_json::to_string(&layout).unwrap_or_else(|_| "{}".into())
                ],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    touch(conn, paper_id)?;
    get(conn, paper_id)
}

pub fn delete_section(
    conn: &mut Connection,
    paper_id: &str,
    section_id: &str,
) -> Result<PaperDetail, String> {
    ensure_draft(conn, paper_id)?;
    conn.execute(
        "delete from local_paper_section where id = ?1 and paper_id = ?2",
        params![section_id, paper_id],
    )
    .map_err(|e| e.to_string())?;
    resequence_sections(conn, paper_id)?;
    touch(conn, paper_id)?;
    get(conn, paper_id)
}

fn resequence_sections(conn: &Connection, paper_id: &str) -> Result<(), String> {
    let mut stmt = conn
        .prepare("select id from local_paper_section where paper_id = ?1 order by ord, id")
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt
        .query_map([paper_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    // 先把 ord 整体挪开，避免唯一性冲突（这里虽无唯一约束，但保持一致习惯）。
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "update local_paper_section set ord = ?2 where id = ?1",
            params![id, 10_000 + index as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "update local_paper_section set ord = ?2 where id = ?1",
            params![id, index as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn reorder_sections(
    conn: &mut Connection,
    paper_id: &str,
    ids: &[String],
) -> Result<PaperDetail, String> {
    ensure_draft(conn, paper_id)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (index, id) in ids.iter().enumerate() {
        tx.execute(
            "update local_paper_section set ord = ?3 where id = ?1 and paper_id = ?2",
            params![id, paper_id, index as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    touch(conn, paper_id)?;
    get(conn, paper_id)
}

/// 加入题目：自动写快照；已在卷内的题跳过，不重复加。
pub fn add_questions(
    conn: &mut Connection,
    paper_id: &str,
    section_id: &str,
    question_ids: &[String],
) -> Result<PaperDetail, String> {
    ensure_draft(conn, paper_id)?;
    let (mode, unit): (String, f64) = conn
        .query_row(
            "select score_mode, unit_score from local_paper_section where id = ?1 and paper_id = ?2",
            params![section_id, paper_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "这个大题不在这份试卷里".to_string())?;

    let mut used: HashSet<String> = {
        let mut stmt = conn
            .prepare(
                "select coalesce(question_id, '') from local_paper_question where paper_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows: Vec<String> = stmt
            .query_map([paper_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        rows.into_iter().collect()
    };
    let mut ord: i64 = conn
        .query_row(
            "select coalesce(max(ord), -1) + 1 from local_paper_question where paper_id = ?1 and section_id = ?2",
            params![paper_id, section_id],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let now = text::now();
    let score = if mode == "UNIFORM" { unit } else { 0.0 };
    let mut added = 0;
    for id in question_ids {
        if used.contains(id) {
            continue;
        }
        let question = match crate::repo::get(conn, id) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let snapshot = serde_json::to_string(&question).map_err(|e| e.to_string())?;
        conn.execute(
            "insert into local_paper_question (paper_id, section_id, ord, question_id, score, snapshot_json, snapshot_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![paper_id, section_id, ord, id, score, snapshot, now],
        )
        .map_err(|e| e.to_string())?;
        used.insert(id.clone());
        ord += 1;
        added += 1;
    }
    if added == 0 {
        return Err("这些题都已经在卷子里了".to_string());
    }
    touch(conn, paper_id)?;
    get(conn, paper_id)
}

pub fn remove_question(
    conn: &mut Connection,
    paper_id: &str,
    section_id: &str,
    ord: i64,
) -> Result<PaperDetail, String> {
    ensure_draft(conn, paper_id)?;
    conn.execute(
        "delete from local_paper_question where paper_id = ?1 and section_id = ?2 and ord = ?3",
        params![paper_id, section_id, ord],
    )
    .map_err(|e| e.to_string())?;
    resequence_questions(conn, paper_id, section_id)?;
    touch(conn, paper_id)?;
    get(conn, paper_id)
}

fn resequence_questions(conn: &Connection, paper_id: &str, section_id: &str) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "select ord, question_id, score, snapshot_json, snapshot_at, flags_json
             from local_paper_question where paper_id = ?1 and section_id = ?2 order by ord",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, Option<String>, f64, String, String, String)> = stmt
        .query_map(params![paper_id, section_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    conn.execute(
        "delete from local_paper_question where paper_id = ?1 and section_id = ?2",
        params![paper_id, section_id],
    )
    .map_err(|e| e.to_string())?;
    for (index, row) in rows.into_iter().enumerate() {
        conn.execute(
            "insert into local_paper_question (paper_id, section_id, ord, question_id, score, snapshot_json, snapshot_at, flags_json)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                paper_id,
                section_id,
                index as i64,
                row.1,
                row.2,
                row.3,
                row.4,
                row.5
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn reorder_questions(
    conn: &mut Connection,
    paper_id: &str,
    section_id: &str,
    ords: &[i64],
) -> Result<PaperDetail, String> {
    ensure_draft(conn, paper_id)?;
    let mut stmt = conn
        .prepare(
            "select ord, question_id, score, snapshot_json, snapshot_at, flags_json
             from local_paper_question where paper_id = ?1 and section_id = ?2 order by ord",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, Option<String>, f64, String, String, String)> = stmt
        .query_map(params![paper_id, section_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    let mut by_ord: std::collections::HashMap<i64, _> =
        rows.into_iter().map(|row| (row.0, row)).collect();
    conn.execute(
        "delete from local_paper_question where paper_id = ?1 and section_id = ?2",
        params![paper_id, section_id],
    )
    .map_err(|e| e.to_string())?;
    for (index, source_ord) in ords.iter().enumerate() {
        if let Some(row) = by_ord.remove(source_ord) {
            conn.execute(
                "insert into local_paper_question (paper_id, section_id, ord, question_id, score, snapshot_json, snapshot_at, flags_json)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    paper_id,
                    section_id,
                    index as i64,
                    row.1,
                    row.2,
                    row.3,
                    row.4,
                    row.5
                ],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    touch(conn, paper_id)?;
    get(conn, paper_id)
}

/// 逐题调分：改过的题独立记分，所在大题自动切到 PER_ITEM（与「统一分值」不再冲突）。
pub fn set_scores(
    conn: &mut Connection,
    paper_id: &str,
    items: &[ScoreItem],
) -> Result<PaperDetail, String> {
    ensure_draft(conn, paper_id)?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for item in items {
        let score = if item.score < 0.0 { 0.0 } else { item.score };
        tx.execute(
            "update local_paper_question set score = ?4
             where paper_id = ?1 and section_id = ?2 and ord = ?3",
            params![paper_id, item.section_id, item.ord, score],
        )
        .map_err(|e| e.to_string())?;
        let unit: f64 = tx
            .query_row(
                "select unit_score from local_paper_section where id = ?1",
                [&item.section_id],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        if (unit - score).abs() > 0.001 {
            tx.execute(
                "update local_paper_section set score_mode = 'PER_ITEM' where id = ?1",
                [&item.section_id],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    touch(conn, paper_id)?;
    get(conn, paper_id)
}

/// 同步题库最新版：只刷新快照，不动分值、顺序。
pub fn refresh_snapshot(
    conn: &mut Connection,
    paper_id: &str,
    items: Option<&[SnapshotItem]>,
) -> Result<PaperDetail, String> {
    ensure_draft(conn, paper_id)?;
    let targets: Vec<(String, i64)> = match items {
        Some(list) if !list.is_empty() => list
            .iter()
            .map(|item| (item.section_id.clone(), item.ord))
            .collect(),
        _ => {
            let mut stmt = conn
                .prepare("select section_id, ord from local_paper_question where paper_id = ?1")
                .map_err(|e| e.to_string())?;
            let rows: Vec<(String, i64)> = stmt
                .query_map([paper_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .collect();
            rows
        }
    };
    let now = text::now();
    for (section_id, ord) in targets {
        let question_id: Option<String> = conn
            .query_row(
                "select question_id from local_paper_question where paper_id = ?1 and section_id = ?2 and ord = ?3",
                params![paper_id, section_id, ord],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(question_id) = question_id else {
            continue;
        };
        let Ok(question) = crate::repo::get(conn, &question_id) else {
            continue;
        };
        let snapshot = serde_json::to_string(&question).map_err(|e| e.to_string())?;
        conn.execute(
            "update local_paper_question set snapshot_json = ?4, snapshot_at = ?5
             where paper_id = ?1 and section_id = ?2 and ord = ?3",
            params![paper_id, section_id, ord, snapshot, now],
        )
        .map_err(|e| e.to_string())?;
    }
    touch(conn, paper_id)?;
    get(conn, paper_id)
}

pub fn log_print(
    conn: &Connection,
    paper_id: &str,
    variant: &str,
    copies: i64,
    pages: i64,
    channel: &str,
) -> Result<(), String> {
    conn.execute(
        "insert into local_paper_print (id, paper_id, variant, copies, pages, channel, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            new_id(),
            paper_id,
            variant,
            copies.max(1),
            pages.max(0),
            channel,
            text::now()
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// P1：蓝图组卷（双向细目表）
//
// 与中心系统同一口径：只取已发布题、缺口明确报出、**不跨知识点/难度顶替、不静默凑数**。
// 本地用确定性抽题（用过的少优先 → 新的优先 → id 兜底），同一份蓝图同一题库抽出同一套卷。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BlueprintSection {
    pub kind: String,
    /// 留空按 kind 自动起名（一、单项选择题）。
    pub title: String,
    pub count: i64,
    pub score_mode: String,
    /// 每格的分值；PER_ITEM 时当「预估分」，抽完由老师逐题改。
    pub unit_score: f64,
    /// 难度档配额，如 {"1":2,"2":4,"3":2}；空表示不限难度。老师更常说「易中难」，见 `bands`。
    pub difficulty: BTreeMap<String, i64>,
    /// 易 / 中 / 难 配额（easy=难度 1-2、medium=3、hard=4-5）。与 `difficulty` 二选一，`difficulty` 优先。
    pub bands: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KnowledgeQuota {
    pub knowledge_id: String,
    pub count: i64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Blueprint {
    pub subject_id: Option<String>,
    pub score_scheme: String,
    pub total_score: f64,
    pub sections: Vec<BlueprintSection>,
    pub knowledge_quota: Vec<KnowledgeQuota>,
    /// 缺题时放宽难度重试（前端「放宽难度再组」开关）。
    pub relax_difficulty: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintGap {
    pub section_title: String,
    pub kind: String,
    /// 0 表示不限难度。
    pub difficulty: i64,
    /// 人看的难度档名字（「偏易 1-2」之类），前端直接显示。
    pub label: String,
    pub need: i64,
    pub got: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintReport {
    pub total_score: f64,
    pub planned_score: f64,
    pub planned_count: i64,
    pub added: i64,
    /// false = 有缺口、整次生成已回滚，卷子没变。
    pub filled: bool,
    pub gaps: Vec<BlueprintGap>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintFillResult {
    pub report: BlueprintReport,
    pub paper: PaperDetail,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CandidateQuery {
    pub section_id: String,
    pub ord: i64,
    pub difficulty: Option<i64>,
    pub keyword: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperCandidate {
    pub id: String,
    pub stem: String,
    pub type_label: String,
    pub difficulty: i64,
    pub knowledge: Vec<String>,
    pub used_count: i64,
}

/// 大题 kind → 题库题型。界面上叫「解答题」，题库里存的是 `SOLVED`，这层映射只此一处。
fn kind_types(kind: &str) -> &'static [&'static str] {
    match kind {
        "SINGLE" => &["SINGLE"],
        "MULTI" => &["MULTI"],
        "FILL" => &["FILL"],
        "CALC" | "SOLVED" => &["SOLVED"],
        _ => &["SINGLE", "MULTI", "FILL", "SOLVED"],
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "SINGLE" => "单项选择题",
        "MULTI" => "多项选择题",
        "FILL" => "填空题",
        _ => "解答题",
    }
}

fn ordinal_cn(index: i64) -> String {
    const DIGITS: [&str; 10] = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];
    if index >= 1 && index <= 10 {
        DIGITS[(index - 1) as usize].to_string()
    } else {
        index.to_string()
    }
}

fn paper_question_ids(conn: &Connection, paper_id: &str) -> Result<HashSet<String>, String> {
    let mut stmt = conn
        .prepare("select coalesce(question_id, '') from local_paper_question where paper_id = ?1")
        .map_err(|e| e.to_string())?;
    let rows: Vec<String> = stmt
        .query_map([paper_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows.into_iter().filter(|id| !id.is_empty()).collect())
}

/// 按蓝图顺序重排大题：蓝图里的大题在前（按蓝图顺序），没被蓝图覆盖的排在后面。
fn reindex_sections(conn: &Connection, paper_id: &str, first: &[String]) -> Result<(), String> {
    let mut stmt = conn
        .prepare("select id from local_paper_section where paper_id = ?1 order by ord")
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt
        .query_map([paper_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    let mut ordered: Vec<String> = first.to_vec();
    for id in ids {
        if !ordered.contains(&id) {
            ordered.push(id);
        }
    }
    for (index, id) in ordered.iter().enumerate() {
        conn.execute(
            "update local_paper_section set ord = ?2 where id = ?1",
            params![id, index as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 难度档：老师按「易 / 中 / 难」想事，库里是 1–5 的整数分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Any,
    Exact(i64),
    Range(i64, i64),
}

impl Band {
    fn label(self) -> String {
        match self {
            Band::Any => "不限难度".to_string(),
            Band::Exact(level) => format!("难度 {level}"),
            Band::Range(low, high) => format!("难度 {low}-{high}"),
        }
    }
}

fn band_of(name: &str) -> Option<Band> {
    match name {
        "easy" => Some(Band::Range(1, 2)),
        "medium" => Some(Band::Exact(3)),
        "hard" => Some(Band::Range(4, 5)),
        _ => None,
    }
}

/// 抽题：只取已发布、未删除、不在排除集里的题；用过的少优先、新的优先、id 兜底。
fn pick_candidates(
    conn: &Connection,
    subject_id: Option<&str>,
    kind: &str,
    band: Band,
    exclude: &HashSet<String>,
    knowledge_prefer: &[String],
    limit: usize,
) -> Result<Vec<String>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let types = kind_types(kind);
    let mut sql = String::from(
        "select q.id,
                (select count(*) from local_paper_question x where x.question_id = q.id) as used
         from question q
         where q.status = 'PUBLISHED' and q.deleted_at is null",
    );
    let mut args: Vec<SqlValue> = Vec::new();

    sql.push_str(&format!(
        " and q.type in ({})",
        vec!["?"; types.len()].join(",")
    ));
    for item in types {
        args.push(SqlValue::Text((*item).to_string()));
    }
    if let Some(subject) = subject_id.filter(|value| !value.is_empty()) {
        sql.push_str(" and q.subject_id = ?");
        args.push(SqlValue::Text(subject.to_string()));
    }
    match band {
        Band::Any => {}
        Band::Exact(level) => {
            sql.push_str(" and q.difficulty = ?");
            args.push(SqlValue::Integer(level));
        }
        Band::Range(low, high) => {
            sql.push_str(" and q.difficulty between ? and ?");
            args.push(SqlValue::Integer(low));
            args.push(SqlValue::Integer(high));
        }
    }
    if !exclude.is_empty() {
        sql.push_str(&format!(
            " and q.id not in ({})",
            vec!["?"; exclude.len()].join(",")
        ));
        for id in exclude {
            args.push(SqlValue::Text(id.clone()));
        }
    }
    if !knowledge_prefer.is_empty() {
        sql.push_str(&format!(
            " and exists (select 1 from question_knowledge k where k.question_id = q.id
                and k.knowledge_point_id in ({}))",
            vec!["?"; knowledge_prefer.len()].join(",")
        ));
        for id in knowledge_prefer {
            args.push(SqlValue::Text(id.clone()));
        }
    }
    sql.push_str(" order by used asc, q.created_at desc, q.id asc limit ?");
    args.push(SqlValue::Integer(limit as i64));

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<String> = stmt
        .query_map(params_from_iter(args), |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

/// 从当前卷倒推一份蓝图：大题 kind + 题数 + 单位分，难度不限。
pub fn blueprint_suggest(conn: &Connection, paper_id: &str) -> Result<Blueprint, String> {
    let paper = get(conn, paper_id)?;
    Ok(Blueprint {
        subject_id: paper.subject_id.clone(),
        score_scheme: paper.score_scheme.clone(),
        total_score: paper.total_score,
        sections: paper
            .sections
            .iter()
            .map(|section| BlueprintSection {
                kind: if section.kind.is_empty() {
                    "CALC".to_string()
                } else {
                    section.kind.clone()
                },
                title: section.title.clone(),
                count: section.questions.len() as i64,
                score_mode: section.score_mode.clone(),
                unit_score: if section.unit_score > 0.0 {
                    section.unit_score
                } else {
                    section
                        .questions
                        .first()
                        .map(|item| item.score)
                        .unwrap_or(0.0)
                },
                difficulty: BTreeMap::new(),
                bands: BTreeMap::new(),
            })
            .collect(),
        knowledge_quota: Vec::new(),
        relax_difficulty: false,
    })
}

fn blueprint_validate(
    blueprint: &Blueprint,
    fallback_total: f64,
) -> Result<(f64, f64, i64), String> {
    if blueprint.sections.is_empty() {
        return Err("蓝图里还没有大题，先加一行再组卷".to_string());
    }
    let mut planned = 0.0;
    let mut planned_count = 0;
    for (index, section) in blueprint.sections.iter().enumerate() {
        if section.count < 0 {
            return Err(format!("第 {} 个大题的题数不能为负", index + 1));
        }
        if section.count > 0 && section.unit_score <= 0.0 {
            return Err(format!(
                "{} 每题分值要大于 0（解答题填预估分，抽完再逐题改）",
                if section.title.is_empty() {
                    kind_label(&section.kind).to_string()
                } else {
                    section.title.clone()
                }
            ));
        }
        planned += section.count as f64 * section.unit_score;
        planned_count += section.count;
    }
    planned = (planned * 1000.0).round() / 1000.0;
    let total = if blueprint.total_score > 0.0 {
        blueprint.total_score
    } else {
        fallback_total
    };
    let delta = ((total - planned) * 1000.0).round() / 1000.0;
    if delta.abs() >= 0.001 {
        let hint = if delta > 0.0 { "还差" } else { "多了" };
        return Err(format!(
            "蓝图分值不平：Σ(题数×分值) = {} 分，总分 {} 分，{} {} 分。先改题数或每题分值，再按蓝图组卷。",
            score_text(planned),
            score_text(total),
            hint,
            score_text(delta.abs())
        ));
    }
    let quota: i64 = blueprint
        .knowledge_quota
        .iter()
        .map(|item| item.count)
        .sum();
    if quota > planned_count {
        return Err(format!(
            "知识点配额合计 {} 题，超过蓝图总题数 {} 题",
            quota, planned_count
        ));
    }
    Ok((planned, total, planned_count))
}

fn blueprint_apply(
    conn: &mut Connection,
    paper_id: &str,
    blueprint: &Blueprint,
    replace: bool,
    exclude_paper: Option<&str>,
) -> Result<BlueprintReport, String> {
    ensure_draft(conn, paper_id)?;
    let paper = get(conn, paper_id)?;
    let (planned_score, total_score, planned_count) =
        blueprint_validate(blueprint, paper.total_score)?;

    let mut exclude = if replace {
        HashSet::new()
    } else {
        paper_question_ids(conn, paper_id)?
    };
    if let Some(other) = exclude_paper {
        exclude.extend(paper_question_ids(conn, other)?);
    }

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if replace {
        tx.execute(
            "delete from local_paper_question where paper_id = ?1",
            [paper_id],
        )
        .map_err(|e| e.to_string())?;
    }

    // 知识点配额：先按配额挑，配额用完后不再限制知识点（避免硬顶替）。
    let mut quota_left: Vec<(String, i64)> = blueprint
        .knowledge_quota
        .iter()
        .filter(|item| item.count > 0 && !item.knowledge_id.is_empty())
        .map(|item| (item.knowledge_id.clone(), item.count))
        .collect();

    let mut gaps: Vec<BlueprintGap> = Vec::new();
    let mut picks: Vec<(String, Vec<String>)> = Vec::new(); // (section_id, question_ids)

    for (index, spec) in blueprint.sections.iter().enumerate() {
        if spec.count == 0 {
            continue;
        }
        let title = if spec.title.trim().is_empty() {
            format!(
                "{}、{}",
                ordinal_cn(index as i64 + 1),
                kind_label(&spec.kind)
            )
        } else {
            spec.title.trim().to_string()
        };
        // 复用同题型的大题，没有就新建一个，避免同一份蓝图每次组卷都多出一批空大题。
        let existing = paper
            .sections
            .iter()
            .find(|section| section.kind == spec.kind)
            .map(|section| section.id.clone());
        let section_id = match existing {
            Some(id) => {
                tx.execute(
                    "update local_paper_section set title = ?2, kind = ?3, score_mode = ?4, unit_score = ?5
                     where id = ?1",
                    params![
                        id,
                        title,
                        spec.kind,
                        if spec.score_mode.is_empty() {
                            "UNIFORM"
                        } else {
                            spec.score_mode.as_str()
                        },
                        spec.unit_score
                    ],
                )
                .map_err(|e| e.to_string())?;
                id
            }
            None => {
                let id = new_id();
                let ord: i64 = tx
                    .query_row(
                        "select coalesce(max(ord), -1) + 1 from local_paper_section where paper_id = ?1",
                        [paper_id],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                tx.execute(
                    "insert into local_paper_section (id, paper_id, ord, title, note, kind, score_mode, unit_score, layout_json)
                     values (?1, ?2, ?3, ?4, '', ?5, ?6, ?7, '{}')",
                    params![
                        id,
                        paper_id,
                        ord,
                        title,
                        spec.kind,
                        if spec.score_mode.is_empty() {
                            "UNIFORM"
                        } else {
                            spec.score_mode.as_str()
                        },
                        spec.unit_score
                    ],
                )
                .map_err(|e| e.to_string())?;
                id
            }
        };

        // 难度配额：放宽开关打开就不限难度；否则先认精确档（difficulty），再认易/中/难（bands）。
        let buckets: Vec<(Band, i64)> = if blueprint.relax_difficulty {
            vec![(Band::Any, spec.count)]
        } else if !spec.difficulty.is_empty() {
            let mut list: Vec<(Band, i64)> = spec
                .difficulty
                .iter()
                .filter_map(|(key, value)| {
                    key.parse::<i64>()
                        .ok()
                        .filter(|level| *level > 0)
                        .map(|level| (Band::Exact(level), *value))
                })
                .filter(|(_, need)| *need > 0)
                .collect();
            list.sort_by_key(|(band, _)| match band {
                Band::Exact(level) => *level,
                _ => 0,
            });
            list
        } else if spec.bands.values().any(|value| *value > 0) {
            let mut list: Vec<(Band, i64)> = spec
                .bands
                .iter()
                .filter_map(|(key, value)| band_of(key).map(|band| (band, *value)))
                .filter(|(_, need)| *need > 0)
                .collect();
            list.sort_by_key(|(band, _)| match band {
                Band::Range(low, _) => *low,
                Band::Exact(level) => *level,
                Band::Any => 0,
            });
            list
        } else {
            vec![(Band::Any, spec.count)]
        };
        let bucket_total: i64 = buckets.iter().map(|(_, need)| *need).sum();
        if bucket_total != spec.count {
            return Err(format!(
                "{} 的难度配额合计 {} 题，与题数 {} 题不一致",
                title, bucket_total, spec.count
            ));
        }

        let mut section_ids: Vec<String> = Vec::new();
        for (band, need) in buckets {
            // 先把还有配额的知识点拿出来优先匹配；配额用完就退回全学科。
            let prefer: Vec<String> = quota_left
                .iter()
                .filter(|(_, left)| *left > 0)
                .map(|(id, _)| id.clone())
                .collect();
            let mut got: Vec<String> = Vec::new();
            if !prefer.is_empty() {
                got = pick_candidates(
                    &tx,
                    blueprint
                        .subject_id
                        .as_deref()
                        .or(paper.subject_id.as_deref()),
                    &spec.kind,
                    band,
                    &exclude,
                    &prefer,
                    need as usize,
                )?;
                for id in &got {
                    for entry in quota_left.iter_mut() {
                        if entry.1 > 0 {
                            entry.1 -= 1;
                            break;
                        }
                    }
                    exclude.insert(id.clone());
                }
            }
            if got.len() < need as usize {
                let rest = pick_candidates(
                    &tx,
                    blueprint
                        .subject_id
                        .as_deref()
                        .or(paper.subject_id.as_deref()),
                    &spec.kind,
                    band,
                    &exclude,
                    &[],
                    (need as usize).saturating_sub(got.len()),
                )?;
                for id in &rest {
                    exclude.insert(id.clone());
                }
                got.extend(rest);
            }
            if got.len() < need as usize {
                gaps.push(BlueprintGap {
                    section_title: title.clone(),
                    kind: spec.kind.clone(),
                    difficulty: match band {
                        Band::Exact(level) => level,
                        _ => 0,
                    },
                    label: band.label(),
                    need,
                    got: got.len() as i64,
                });
            }
            section_ids.extend(got);
        }
        picks.push((section_id, section_ids));
    }

    if !gaps.is_empty() {
        let detail = gaps
            .iter()
            .map(|gap| {
                format!(
                    "{}（{}）需要 {} 题，只找到 {} 题",
                    gap.section_title, gap.label, gap.need, gap.got
                )
            })
            .collect::<Vec<_>>()
            .join("；");
        // 不落库：宁可整次不给，也不留一份凑不满的卷子。
        drop(tx);
        return Ok(BlueprintReport {
            total_score,
            planned_score,
            planned_count,
            added: 0,
            filled: false,
            gaps,
            message: format!(
                "按蓝图组卷没完成，卷子没有改动。缺题：{detail}。可以放宽难度或调低题目数再试。"
            ),
        });
    }

    let now = text::now();
    let section_order: Vec<String> = picks.iter().map(|(id, _)| id.clone()).collect();
    let mut added = 0;
    for (section_id, ids) in picks {
        for (ord, id) in ids.iter().enumerate() {
            let question =
                crate::repo::get(&tx, id).map_err(|_| format!("题目 {id} 取不到了，稍后重试"))?;
            let snapshot = serde_json::to_string(&question).map_err(|e| e.to_string())?;
            let unit: f64 = tx
                .query_row(
                    "select unit_score from local_paper_section where id = ?1",
                    [&section_id],
                    |row| row.get(0),
                )
                .unwrap_or(0.0);
            tx.execute(
                "insert into local_paper_question (paper_id, section_id, ord, question_id, score, snapshot_json, snapshot_at)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![paper_id, section_id, ord as i64, id, unit, snapshot, now],
            )
            .map_err(|e| e.to_string())?;
            added += 1;
        }
    }

    reindex_sections(&tx, paper_id, &section_order)?;

    tx.execute(
        "update local_paper set total_score = ?2, blueprint_json = ?3,
            score_scheme = case when ?4 = '' then score_scheme else ?4 end, updated_at = ?5
         where id = ?1",
        params![
            paper_id,
            planned_score,
            serde_json::to_string(blueprint).unwrap_or_else(|_| "{}".into()),
            blueprint.score_scheme.as_str(),
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    touch(conn, paper_id)?;

    Ok(BlueprintReport {
        total_score,
        planned_score,
        planned_count,
        added,
        filled: true,
        gaps: Vec::new(),
        message: format!(
            "按蓝图加入 {} 题，共 {} 分",
            added,
            score_text(planned_score)
        ),
    })
}

pub fn blueprint_fill(
    conn: &mut Connection,
    paper_id: &str,
    blueprint: &Blueprint,
    replace: bool,
) -> Result<BlueprintFillResult, String> {
    let report = blueprint_apply(conn, paper_id, blueprint, replace, None)?;
    let paper = get(conn, paper_id)?;
    Ok(BlueprintFillResult { report, paper })
}

/// 按此蓝图再组一套（B 卷 / 补考卷）：复制一份草稿，再按蓝图重新抽题，
/// 抽的时候排除原卷用过的题，两份卷不会撞题。
pub fn blueprint_again(
    conn: &mut Connection,
    paper_id: &str,
) -> Result<BlueprintFillResult, String> {
    let source = get(conn, paper_id)?;
    let blueprint = match serde_json::from_value::<Blueprint>(source.blueprint.clone()) {
        Ok(value) if !value.sections.is_empty() => value,
        _ => blueprint_suggest(conn, paper_id)?,
    };
    let copy = duplicate(conn, paper_id)?;
    let report = blueprint_apply(conn, &copy.id, &blueprint, true, Some(paper_id))?;
    let paper = get(conn, &copy.id)?;
    Ok(BlueprintFillResult { report, paper })
}

// ---------------------------------------------------------------------------
// P1：换题与撤销
// ---------------------------------------------------------------------------

/// 换题候选：同学科 + 同题型 + 至少共享一个知识点 + 已发布 + 不在本卷。
/// 共享知识点一个都没有时退回「同题型同科」，不把候选列表做成空的。
pub fn replace_candidates(
    conn: &Connection,
    paper_id: &str,
    query: &CandidateQuery,
) -> Result<Vec<PaperCandidate>, String> {
    let paper = get(conn, paper_id)?;
    let section = paper
        .sections
        .iter()
        .find(|item| item.id == query.section_id)
        .ok_or_else(|| "这个大题不在这份试卷里".to_string())?;
    let current = section
        .questions
        .iter()
        .find(|item| item.ord == query.ord)
        .ok_or_else(|| "这一题不在卷子里".to_string())?;
    let current_id = current.question_id.clone().unwrap_or_default();
    // 大题没写 kind（例如手工建的空大题）时，以这道题自己的题型为准，
    // 否则换题候选会被过滤成另一个题型、列表永远是空的。
    let snapshot_type = current
        .snapshot
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = if !section.kind.is_empty() {
        section.kind.clone()
    } else if !snapshot_type.is_empty() {
        snapshot_type.to_string()
    } else {
        "SOLVED".to_string()
    };
    let types = kind_types(&kind);

    let knowledge: Vec<String> = if current_id.is_empty() {
        Vec::new()
    } else {
        let mut stmt = conn
            .prepare("select knowledge_point_id from question_knowledge where question_id = ?1")
            .map_err(|e| e.to_string())?;
        let rows: Vec<String> = stmt
            .query_map([&current_id], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        rows
    };

    let exclude = paper_question_ids(conn, paper_id)?;
    let limit = query.limit.unwrap_or(20).clamp(1, 60);

    let mut sql = String::from(
        "select q.id, q.stem, q.type, q.difficulty,
                (select count(*) from local_paper_question x where x.question_id = q.id) as used
         from question q
         where q.status = 'PUBLISHED' and q.deleted_at is null",
    );
    let mut args: Vec<SqlValue> = Vec::new();
    sql.push_str(&format!(
        " and q.type in ({})",
        vec!["?"; types.len()].join(",")
    ));
    for item in types {
        args.push(SqlValue::Text((*item).to_string()));
    }
    if let Some(subject) = paper
        .subject_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        sql.push_str(" and q.subject_id = ?");
        args.push(SqlValue::Text(subject.to_string()));
    }
    if let Some(difficulty) = query.difficulty.filter(|value| *value > 0) {
        sql.push_str(" and q.difficulty = ?");
        args.push(SqlValue::Integer(difficulty));
    }
    if let Some(keyword) = query
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        sql.push_str(" and q.stem like ?");
        args.push(SqlValue::Text(format!("%{keyword}%")));
    }
    if !exclude.is_empty() {
        sql.push_str(&format!(
            " and q.id not in ({})",
            vec!["?"; exclude.len()].join(",")
        ));
        for id in &exclude {
            args.push(SqlValue::Text(id.clone()));
        }
    }
    if !knowledge.is_empty() {
        sql.push_str(&format!(
            " and exists (select 1 from question_knowledge k where k.question_id = q.id
                and k.knowledge_point_id in ({}))",
            vec!["?"; knowledge.len()].join(",")
        ));
        for id in &knowledge {
            args.push(SqlValue::Text(id.clone()));
        }
    }
    sql.push_str(" order by used asc, q.created_at desc, q.id asc limit ?");
    args.push(SqlValue::Integer(limit));

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<(String, String, String, i64, i64)> = stmt
        .query_map(params_from_iter(args), |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut out = Vec::with_capacity(rows.len());
    for (id, stem, question_type, difficulty, used) in rows {
        let mut kp = conn
            .prepare(
                "select kp.name from question_knowledge qk
                 join knowledge_point kp on kp.id = qk.knowledge_point_id
                 where qk.question_id = ?1 limit 4",
            )
            .map_err(|e| e.to_string())?;
        let names: Vec<String> = kp
            .query_map([&id], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        out.push(PaperCandidate {
            id,
            stem,
            type_label: text::type_label(&question_type).to_string(),
            difficulty,
            knowledge: names,
            used_count: used,
        });
    }
    Ok(out)
}

/// 换题：题号、分值、大题归属都不动，只换题目与快照；上一题记进 `flags_json` 以便撤销。
pub fn replace_question(
    conn: &mut Connection,
    paper_id: &str,
    section_id: &str,
    ord: i64,
    question_id: &str,
) -> Result<PaperDetail, String> {
    ensure_draft(conn, paper_id)?;
    let row: Option<(Option<String>, f64, String, String, String)> = conn
        .query_row(
            "select question_id, score, flags_json, snapshot_json, snapshot_at
             from local_paper_question where paper_id = ?1 and section_id = ?2 and ord = ?3",
            params![paper_id, section_id, ord],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((current_id, score, _flags, snapshot_json, snapshot_at)) = row else {
        return Err("这一题不在卷子里".to_string());
    };
    if current_id.as_deref() == Some(question_id) {
        return Err("这道题已经在当前位置了".to_string());
    }
    let used: bool = conn
        .query_row(
            "select exists(select 1 from local_paper_question where paper_id = ?1 and question_id = ?2)",
            params![paper_id, question_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if used {
        return Err("这道题已经在卷子里了".to_string());
    }
    let question = crate::repo::get(conn, question_id)?;
    let snapshot = serde_json::to_string(&question).map_err(|e| e.to_string())?;
    let previous: Value = serde_json::from_str(&snapshot_json).unwrap_or(Value::Null);
    let flags = json!({
        "prev": {
            "questionId": current_id,
            "score": score,
            "snapshot": previous,
            "snapshotAt": snapshot_at,
        }
    });
    conn.execute(
        "update local_paper_question set question_id = ?4, snapshot_json = ?5, snapshot_at = ?6, flags_json = ?7
         where paper_id = ?1 and section_id = ?2 and ord = ?3",
        params![
            paper_id,
            section_id,
            ord,
            question_id,
            snapshot,
            text::now(),
            serde_json::to_string(&flags).unwrap_or_else(|_| "{}".into())
        ],
    )
    .map_err(|e| e.to_string())?;
    touch(conn, paper_id)?;
    get(conn, paper_id)
}

pub fn undo_replace(
    conn: &mut Connection,
    paper_id: &str,
    section_id: &str,
    ord: i64,
) -> Result<PaperDetail, String> {
    ensure_draft(conn, paper_id)?;
    let flags_json: Option<String> = conn
        .query_row(
            "select flags_json from local_paper_question where paper_id = ?1 and section_id = ?2 and ord = ?3",
            params![paper_id, section_id, ord],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(flags_json) = flags_json else {
        return Err("这一题不在卷子里".to_string());
    };
    let flags: Value = serde_json::from_str(&flags_json).unwrap_or(json!({}));
    let Some(prev) = flags.get("prev") else {
        return Err("这一题没有可撤销的换题".to_string());
    };
    let question_id = prev
        .get("questionId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "上一次的题目 id 丢了，没法撤销".to_string())?;
    let snapshot = prev.get("snapshot").cloned().unwrap_or(Value::Null);
    if snapshot.is_null() {
        return Err("上一次的快照丢了，没法撤销".to_string());
    }
    conn.execute(
        "update local_paper_question set question_id = ?4, snapshot_json = ?5, flags_json = '{}'
         where paper_id = ?1 and section_id = ?2 and ord = ?3",
        params![
            paper_id,
            section_id,
            ord,
            question_id,
            serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".into())
        ],
    )
    .map_err(|e| e.to_string())?;
    touch(conn, paper_id)?;
    get(conn, paper_id)
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
    state.db.lock().map_err(|_| "数据库忙，请重试".to_string())
}

#[tauri::command]
pub fn paper_schemes() -> Value {
    schemes()
}

#[tauri::command]
pub fn paper_list(
    state: State<'_, AppState>,
    query: Option<PaperQuery>,
) -> Result<Page<PaperSummary>, String> {
    let conn = lock(&state)?;
    list(&conn, &query.unwrap_or_default())
}

#[tauri::command]
pub fn paper_get(state: State<'_, AppState>, id: String) -> Result<PaperDetail, String> {
    let conn = lock(&state)?;
    get(&conn, &id)
}

#[tauri::command]
pub fn paper_create(
    state: State<'_, AppState>,
    input: PaperCreateInput,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    create(&mut conn, &input)
}

#[tauri::command]
pub fn paper_update(
    state: State<'_, AppState>,
    id: String,
    patch: PaperPatch,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    update(&mut conn, &id, &patch)
}

#[tauri::command]
pub fn paper_set_total(
    state: State<'_, AppState>,
    id: String,
    total: f64,
    confirm: Option<bool>,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    set_total(&mut conn, &id, total, confirm.unwrap_or(false))
}

#[tauri::command]
pub fn paper_apply_scheme(
    state: State<'_, AppState>,
    id: String,
    scheme_id: String,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    apply_scheme(&mut conn, &id, &scheme_id)
}

#[tauri::command]
pub fn paper_score_check(state: State<'_, AppState>, id: String) -> Result<ScoreCheck, String> {
    let conn = lock(&state)?;
    score_check(&conn, &id)
}

#[tauri::command]
pub fn paper_finalize(state: State<'_, AppState>, id: String) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    finalize(&mut conn, &id)
}

#[tauri::command]
pub fn paper_reopen(state: State<'_, AppState>, id: String) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    set_status(&mut conn, &id, "DRAFT")
}

#[tauri::command]
pub fn paper_archive(state: State<'_, AppState>, id: String) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    set_status(&mut conn, &id, "ARCHIVED")
}

#[tauri::command]
pub fn paper_duplicate(state: State<'_, AppState>, id: String) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    duplicate(&mut conn, &id)
}

#[tauri::command]
pub fn paper_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    soft_delete(&conn, &id, true)
}

#[tauri::command]
pub fn paper_restore(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    soft_delete(&conn, &id, false)
}

#[tauri::command]
pub fn paper_section_save(
    state: State<'_, AppState>,
    paper_id: String,
    section: PaperSectionInput,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    save_section(&mut conn, &paper_id, &section)
}

#[tauri::command]
pub fn paper_section_delete(
    state: State<'_, AppState>,
    paper_id: String,
    section_id: String,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    delete_section(&mut conn, &paper_id, &section_id)
}

#[tauri::command]
pub fn paper_section_reorder(
    state: State<'_, AppState>,
    paper_id: String,
    ids: Vec<String>,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    reorder_sections(&mut conn, &paper_id, &ids)
}

#[tauri::command]
pub fn paper_add_questions(
    state: State<'_, AppState>,
    paper_id: String,
    section_id: String,
    question_ids: Vec<String>,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    add_questions(&mut conn, &paper_id, &section_id, &question_ids)
}

#[tauri::command]
pub fn paper_remove_question(
    state: State<'_, AppState>,
    paper_id: String,
    section_id: String,
    ord: i64,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    remove_question(&mut conn, &paper_id, &section_id, ord)
}

#[tauri::command]
pub fn paper_reorder_questions(
    state: State<'_, AppState>,
    paper_id: String,
    section_id: String,
    ords: Vec<i64>,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    reorder_questions(&mut conn, &paper_id, &section_id, &ords)
}

#[tauri::command]
pub fn paper_set_scores(
    state: State<'_, AppState>,
    paper_id: String,
    items: Vec<ScoreItem>,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    set_scores(&mut conn, &paper_id, &items)
}

#[tauri::command]
pub fn paper_refresh_snapshot(
    state: State<'_, AppState>,
    paper_id: String,
    items: Option<Vec<SnapshotItem>>,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    refresh_snapshot(&mut conn, &paper_id, items.as_deref())
}

#[tauri::command]
pub fn paper_print_log(
    state: State<'_, AppState>,
    paper_id: String,
    variant: String,
    copies: Option<i64>,
    pages: Option<i64>,
    channel: Option<String>,
) -> Result<(), String> {
    let conn = lock(&state)?;
    log_print(
        &conn,
        &paper_id,
        &variant,
        copies.unwrap_or(1),
        pages.unwrap_or(0),
        &channel.unwrap_or_else(|| "BROWSER".to_string()),
    )
}

#[tauri::command]
pub fn paper_blueprint_suggest(
    state: State<'_, AppState>,
    paper_id: String,
) -> Result<Blueprint, String> {
    let conn = lock(&state)?;
    blueprint_suggest(&conn, &paper_id)
}

#[tauri::command]
pub fn paper_blueprint_fill(
    state: State<'_, AppState>,
    paper_id: String,
    blueprint: Blueprint,
    replace: Option<bool>,
) -> Result<BlueprintFillResult, String> {
    let mut conn = lock(&state)?;
    blueprint_fill(&mut conn, &paper_id, &blueprint, replace.unwrap_or(false))
}

#[tauri::command]
pub fn paper_blueprint_again(
    state: State<'_, AppState>,
    paper_id: String,
) -> Result<BlueprintFillResult, String> {
    let mut conn = lock(&state)?;
    blueprint_again(&mut conn, &paper_id)
}

#[tauri::command]
pub fn paper_replace_candidates(
    state: State<'_, AppState>,
    paper_id: String,
    query: CandidateQuery,
) -> Result<Vec<PaperCandidate>, String> {
    let conn = lock(&state)?;
    replace_candidates(&conn, &paper_id, &query)
}

#[tauri::command]
pub fn paper_replace_question(
    state: State<'_, AppState>,
    paper_id: String,
    section_id: String,
    ord: i64,
    question_id: String,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    replace_question(&mut conn, &paper_id, &section_id, ord, &question_id)
}

#[tauri::command]
pub fn paper_undo_replace(
    state: State<'_, AppState>,
    paper_id: String,
    section_id: String,
    ord: i64,
) -> Result<PaperDetail, String> {
    let mut conn = lock(&state)?;
    undo_replace(&mut conn, &paper_id, &section_id, ord)
}

#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    let target = std::path::Path::new(&path);
    if !target.exists() {
        return Err(format!("文件不存在：{path}"));
    }
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "start", ""]);
        cmd
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(target)
        .spawn()
        .map_err(|e| format!("打不开文件：{e}"))?;
    Ok(())
}
