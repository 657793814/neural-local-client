//! 按「知识点 + 难度题数」自动组一份作业草稿。
//!
//! 为什么放在后端：选题要按「用得少的题优先」排序、要跨三层表（question /
//! question_knowledge / local_assignment_question）过滤，前端拿题库接口一页页筛
//! 会漏题也会慢。这里只负责**给候选**，不落库——老师改完点保存才写
//! `local_assignment_question`，与手工选题走同一条写入路径。
//!
//! 两条硬约束：
//! - 只从 `PUBLISHED` 且未删除的题里抽（草稿题不该出现在作业里）；
//! - 抽不到就如实报缺口（`gaps`），不拿别的难度、别的知识点硬顶——
//!   老师看到「易 3 题只凑到 1 题」才会去补题库，悄悄顶替只会让作业失真。

use rusqlite::{params_from_iter, Connection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 难度三档。与试卷蓝图的 bands 口径一致（易 1–2 / 中 3 / 难 4–5）。
pub const BANDS: [(&str, i64, i64); 3] = [("easy", 1, 2), ("middle", 3, 3), ("hard", 4, 5)];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolKnowledge {
    pub id: String,
    pub name: String,
    pub path: String,
    pub easy: i64,
    pub middle: i64,
    pub hard: i64,
}

/// 题库「可用量」：按知识点与难度统计，用来在界面上给出能出几题。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pool {
    pub easy: i64,
    pub middle: i64,
    pub hard: i64,
    pub knowledge: Vec<PoolKnowledge>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposedQuestion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub type_label: String,
    pub stem: String,
    pub excerpt: String,
    pub answer: String,
    pub difficulty: i64,
    /// 这道题挂的知识点名字（最多 3 个），用来在预览里让老师确认「确实考这个」。
    pub knowledge: Vec<String>,
    /// 抽它的时候属于哪一档（easy / middle / hard），方便界面按档分组。
    pub band: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gap {
    pub band: String,
    pub label: String,
    pub need: i64,
    pub got: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub questions: Vec<ComposedQuestion>,
    pub gaps: Vec<Gap>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftInput {
    #[serde(default)]
    pub subject_id: Option<String>,
    #[serde(default)]
    pub knowledge_ids: Vec<String>,
    /// {"easy": 3, "middle": 4, "hard": 2}
    #[serde(default)]
    pub counts: BTreeMap<String, i64>,
    /// 题型过滤（SINGLE / MULTI / FILL / SOLVED），空表示不限。
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapInput {
    #[serde(default)]
    pub subject_id: Option<String>,
    #[serde(default)]
    pub knowledge_ids: Vec<String>,
    #[serde(default)]
    pub band: String,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn band_range(band: &str) -> Option<(i64, i64)> {
    BANDS
        .iter()
        .find(|(name, _, _)| *name == band)
        .map(|(_, low, high)| (*low, *high))
}

fn band_label(band: &str) -> &'static str {
    match band {
        "easy" => "易（难度 1–2）",
        "middle" => "中（难度 3）",
        "hard" => "难（难度 4–5）",
        _ => "不限难度",
    }
}

/// 题库可用量。知识点按学科取全量（没挂题的也给出来，题数为 0 本身就是信息）。
pub fn pool(conn: &Connection, subject_id: Option<&str>) -> Result<Pool, String> {
    let mut counts: BTreeMap<String, (i64, i64, i64)> = BTreeMap::new();
    let mut sql = String::from(
        "select qk.knowledge_point_id,
                sum(case when q.difficulty <= 2 then 1 else 0 end),
                sum(case when q.difficulty = 3 then 1 else 0 end),
                sum(case when q.difficulty >= 4 then 1 else 0 end)
         from question_knowledge qk join question q on q.id = qk.question_id
         where q.status = 'PUBLISHED' and q.deleted_at is null",
    );
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(subject) = subject_id.filter(|value| !value.is_empty()) {
        sql.push_str(" and q.subject_id = ?");
        args.push(rusqlite::types::Value::Text(subject.to_string()));
    }
    sql.push_str(" group by qk.knowledge_point_id");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(args), |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ),
            ))
        })
        .map_err(|e| e.to_string())?;
    for item in rows.flatten() {
        counts.insert(item.0, item.1);
    }

    let mut kp_sql = String::from(
        "select kp.id, kp.name, ifnull(ch.name, '')
         from knowledge_point kp left join chapter ch on ch.id = kp.chapter_id",
    );
    let mut kp_args: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(subject) = subject_id.filter(|value| !value.is_empty()) {
        kp_sql.push_str(" where kp.subject_id = ?");
        kp_args.push(rusqlite::types::Value::Text(subject.to_string()));
    }
    kp_sql.push_str(" order by kp.sort, kp.name");

    let mut kp_stmt = conn.prepare(&kp_sql).map_err(|e| e.to_string())?;
    let knowledge: Vec<PoolKnowledge> = kp_stmt
        .query_map(params_from_iter(kp_args), |row| {
            let id: String = row.get(0)?;
            let (easy, middle, hard) = counts.get(&id).copied().unwrap_or((0, 0, 0));
            Ok(PoolKnowledge {
                id,
                name: row.get(1)?,
                path: row.get(2)?,
                easy,
                middle,
                hard,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let easy = knowledge.iter().map(|item| item.easy).sum();
    let middle = knowledge.iter().map(|item| item.middle).sum();
    let hard = knowledge.iter().map(|item| item.hard).sum();
    Ok(Pool {
        easy,
        middle,
        hard,
        knowledge,
    })
}

/// 抽题：按「在作业里用得少的优先 → 新题优先」，同分再按 id 稳定排序。
///
/// 之所以不用 `random()`：同一套条件点两次「生成」应该得到同一份卷子，
/// 老师才能「再生成一次」对比。想要不同的题就点「换一题」，那里会排除当前题。
fn pick(
    conn: &Connection,
    subject_id: Option<&str>,
    band: Option<(i64, i64)>,
    types: &[String],
    knowledge_ids: &[String],
    exclude: &[String],
    limit: usize,
) -> Result<Vec<String>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut sql = String::from(
        "select q.id
         from question q
         where q.status = 'PUBLISHED' and q.deleted_at is null",
    );
    let mut args: Vec<rusqlite::types::Value> = Vec::new();

    if let Some(subject) = subject_id.filter(|value| !value.is_empty()) {
        sql.push_str(" and q.subject_id = ?");
        args.push(rusqlite::types::Value::Text(subject.to_string()));
    }
    if let Some((low, high)) = band {
        sql.push_str(" and q.difficulty between ? and ?");
        args.push(rusqlite::types::Value::Integer(low));
        args.push(rusqlite::types::Value::Integer(high));
    }
    if !types.is_empty() {
        sql.push_str(&format!(
            " and q.type in ({})",
            vec!["?"; types.len()].join(",")
        ));
        for item in types {
            args.push(rusqlite::types::Value::Text(item.clone()));
        }
    }
    if !knowledge_ids.is_empty() {
        sql.push_str(&format!(
            " and exists (select 1 from question_knowledge k where k.question_id = q.id
                and k.knowledge_point_id in ({}))",
            vec!["?"; knowledge_ids.len()].join(",")
        ));
        for id in knowledge_ids {
            args.push(rusqlite::types::Value::Text(id.clone()));
        }
    }
    if !exclude.is_empty() {
        sql.push_str(&format!(
            " and q.id not in ({})",
            vec!["?"; exclude.len()].join(",")
        ));
        for id in exclude {
            args.push(rusqlite::types::Value::Text(id.clone()));
        }
    }
    sql.push_str(
        " order by (select count(*) from local_assignment_question aq where aq.question_id = q.id) asc,
                   q.created_at desc, q.id asc limit ?",
    );
    args.push(rusqlite::types::Value::Integer(limit as i64));

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(args), |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn hydrate(conn: &Connection, id: &str, band: &str) -> Result<Option<ComposedQuestion>, String> {
    let mut stmt = conn
        .prepare(
            "select q.id, q.type, q.stem, q.answer, q.difficulty
             from question q where q.id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([id]).map_err(|e| e.to_string())?;
    let Some(row) = rows.next().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let kind: String = row.get(1).map_err(|e| e.to_string())?;
    let stem: String = row.get(2).map_err(|e| e.to_string())?;

    let mut kp_stmt = conn
        .prepare(
            "select kp.name from question_knowledge qk
             join knowledge_point kp on kp.id = qk.knowledge_point_id
             where qk.question_id = ?1 order by kp.sort, kp.name limit 3",
        )
        .map_err(|e| e.to_string())?;
    let knowledge: Vec<String> = kp_stmt
        .query_map([id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    Ok(Some(ComposedQuestion {
        id: row.get(0).map_err(|e| e.to_string())?,
        type_label: crate::text::type_label(&kind).to_string(),
        kind,
        excerpt: crate::text::excerpt(&stem, 120),
        stem,
        answer: row.get(3).map_err(|e| e.to_string())?,
        difficulty: row.get(4).map_err(|e| e.to_string())?,
        knowledge,
        band: band.to_string(),
    }))
}

/// 生成草稿：按档依次抽，抽到多少是多少，缺口单独返回。
pub fn draft(conn: &Connection, input: &DraftInput) -> Result<Draft, String> {
    let mut picked: Vec<ComposedQuestion> = Vec::new();
    let mut exclude: Vec<String> = input.exclude.clone();
    let mut gaps: Vec<Gap> = Vec::new();

    for (band, low, high) in BANDS {
        let need = input.counts.get(band).copied().unwrap_or(0).max(0);
        if need == 0 {
            continue;
        }
        let ids = pick(
            conn,
            input.subject_id.as_deref(),
            Some((low, high)),
            &input.types,
            &input.knowledge_ids,
            &exclude,
            need as usize,
        )?;
        // 缺口按「实际拿到」算，不按题库总量算：知识点的题目可能已经被前面的档抽走。
        if (ids.len() as i64) < need {
            gaps.push(Gap {
                band: band.to_string(),
                label: band_label(band).to_string(),
                need,
                got: ids.len() as i64,
            });
        }
        for id in &ids {
            if let Some(item) = hydrate(conn, id, band)? {
                exclude.push(id.clone());
                picked.push(item);
            }
        }
    }

    Ok(Draft {
        questions: picked,
        gaps,
    })
}

/// 换一题：同档、同知识点、同题型里另抽一道，排除当前已有的。
pub fn swap(conn: &Connection, input: &SwapInput) -> Result<Option<ComposedQuestion>, String> {
    let band = band_range(&input.band);
    let ids = pick(
        conn,
        input.subject_id.as_deref(),
        band,
        &input.types,
        &input.knowledge_ids,
        &input.exclude,
        1,
    )?;
    match ids.first() {
        Some(id) => hydrate(conn, id, &input.band),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use rusqlite::params;

    fn setup() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        conn
    }

    fn seed(conn: &Connection, id: &str, kind: &str, difficulty: i64, kp: &str, created: &str) {
        conn.execute(
            "insert into question (id, subject_id, type, stem, answer, difficulty, status, created_at)
             values (?1, 'sub1', ?2, ?3, 'x', ?4, 'PUBLISHED', ?5)",
            params![id, kind, format!("题干 {id}"), difficulty, created],
        )
        .unwrap();
        conn.execute(
            "insert into question_knowledge (question_id, knowledge_point_id) values (?1, ?2)",
            params![id, kp],
        )
        .unwrap();
    }

    fn seed_subject(conn: &Connection) {
        conn.execute("insert into subject (id, name) values ('sub1', '数学')", [])
            .unwrap();
        conn.execute(
            "insert into knowledge_point (id, subject_id, name, sort) values ('kp1', 'sub1', '一元二次方程', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "insert into knowledge_point (id, subject_id, name, sort) values ('kp2', 'sub1', '函数', 2)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn draft_respects_band_quota_and_reports_gaps() {
        let conn = setup();
        seed_subject(&conn);
        seed(&conn, "q1", "SINGLE", 1, "kp1", "2026-01-01");
        seed(&conn, "q2", "FILL", 2, "kp1", "2026-01-02");
        seed(&conn, "q3", "SOLVED", 3, "kp1", "2026-01-03");
        seed(&conn, "q4", "SOLVED", 4, "kp1", "2026-01-04");

        let input = DraftInput {
            subject_id: Some("sub1".into()),
            knowledge_ids: vec!["kp1".into()],
            counts: BTreeMap::from([
                ("easy".to_string(), 2),
                ("middle".to_string(), 1),
                ("hard".to_string(), 2),
            ]),
            types: Vec::new(),
            exclude: Vec::new(),
        };
        let out = draft(&conn, &input).unwrap();
        assert_eq!(out.questions.len(), 4);
        assert_eq!(out.questions[0].band, "easy");
        assert_eq!(out.questions[2].band, "middle");
        // 难档只有 1 道，缺口要如实报出来。
        assert_eq!(out.gaps.len(), 1);
        assert_eq!(out.gaps[0].band, "hard");
        assert_eq!((out.gaps[0].need, out.gaps[0].got), (2, 1));
    }

    #[test]
    fn draft_only_uses_selected_knowledge() {
        let conn = setup();
        seed_subject(&conn);
        seed(&conn, "q1", "SINGLE", 1, "kp1", "2026-01-01");
        seed(&conn, "q2", "SINGLE", 1, "kp2", "2026-01-02");

        let input = DraftInput {
            subject_id: Some("sub1".into()),
            knowledge_ids: vec!["kp2".into()],
            counts: BTreeMap::from([("easy".to_string(), 5)]),
            types: Vec::new(),
            exclude: Vec::new(),
        };
        let out = draft(&conn, &input).unwrap();
        assert_eq!(out.questions.len(), 1);
        assert_eq!(out.questions[0].id, "q2");
        assert_eq!(out.questions[0].knowledge, vec!["函数".to_string()]);
    }

    #[test]
    fn swap_skips_questions_already_in_draft() {
        let conn = setup();
        seed_subject(&conn);
        seed(&conn, "q1", "SINGLE", 1, "kp1", "2026-01-01");
        seed(&conn, "q2", "SINGLE", 1, "kp1", "2026-01-02");

        let first = draft(
            &conn,
            &DraftInput {
                subject_id: Some("sub1".into()),
                knowledge_ids: vec!["kp1".into()],
                counts: BTreeMap::from([("easy".to_string(), 1)]),
                types: Vec::new(),
                exclude: Vec::new(),
            },
        )
        .unwrap();
        let current = first.questions[0].id.clone();
        let next = swap(
            &conn,
            &SwapInput {
                subject_id: Some("sub1".into()),
                knowledge_ids: vec!["kp1".into()],
                band: "easy".into(),
                types: Vec::new(),
                exclude: vec![current.clone()],
            },
        )
        .unwrap()
        .expect("应该还有一道备选");
        assert_ne!(next.id, current);

        // 全部排除时给 None，前端提示「这个条件没有别的题了」。
        let none = swap(
            &conn,
            &SwapInput {
                subject_id: Some("sub1".into()),
                knowledge_ids: vec!["kp1".into()],
                band: "easy".into(),
                types: Vec::new(),
                exclude: vec!["q1".into(), "q2".into()],
            },
        )
        .unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn draft_type_filter_narrows_pool() {
        let conn = setup();
        seed_subject(&conn);
        seed(&conn, "q1", "SINGLE", 1, "kp1", "2026-01-01");
        seed(&conn, "q2", "SOLVED", 1, "kp1", "2026-01-02");

        let out = draft(
            &conn,
            &DraftInput {
                subject_id: Some("sub1".into()),
                knowledge_ids: Vec::new(),
                counts: BTreeMap::from([("easy".to_string(), 3)]),
                types: vec!["SOLVED".into()],
                exclude: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(out.questions.len(), 1);
        assert_eq!(out.questions[0].id, "q2");
    }

    #[test]
    fn pool_counts_by_band() {
        let conn = setup();
        seed_subject(&conn);
        seed(&conn, "q1", "SINGLE", 1, "kp1", "2026-01-01");
        seed(&conn, "q2", "SINGLE", 3, "kp1", "2026-01-02");
        seed(&conn, "q3", "SINGLE", 5, "kp2", "2026-01-03");

        let out = pool(&conn, Some("sub1")).unwrap();
        assert_eq!((out.easy, out.middle, out.hard), (1, 1, 1));
        let kp1 = out.knowledge.iter().find(|item| item.id == "kp1").unwrap();
        assert_eq!((kp1.easy, kp1.middle, kp1.hard), (1, 1, 0));
    }
}
