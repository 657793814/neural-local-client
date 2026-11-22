//! 学情：知识点矩阵、趋势、错题本。
//!
//! 掌握度公式与中心 `ClassService.refreshMastery` **逐行对齐**（决策 D2 的落地）：
//!
//! ```text
//! days   = max(0, (now - submitted_at) / 24h)
//! weight = 1 / (1 + days / 14)              // 两周半衰，越新的作答越算数
//! factor = correct ? 0.75 + difficulty*0.05 : 0.05
//! mastery = round(min(100, sum(weight*factor) / sum(weight) * 100))
//! ```
//!
//! 只算「已判过对错」的作答（`correct is not null`）：主观题待批时既不算对也不算错，
//! 界面上单独显示「待批改 N 题，暂不计入」。`superseded_at is not null` 的重交不参与。
//!
//! 与中心的差别只有一处：本地不落 `study_progress` 表，每次按需重算。作答记录本身
//! 就是完整输入，重算结果与中心写库后的值一致，也省掉「改了分忘刷新」的漂移。

use crate::text;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 掌握度分档：与中心 85 线一致；>0 且 <85 是「在学」，其余「未开始」。
pub const MASTERED_LINE: i32 = 85;

// ---------------------------------------------------------------------------
// 数据形状
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KpRow {
    pub id: String,
    pub name: String,
    /// 0–100；没有任何已判分作答时为 0，`answered` 也是 0。
    pub mastery: i32,
    pub status: String,
    pub answered: i64,
    pub correct: i64,
    /// 这道知识点下还没批的作答数（不计入掌握度）。
    pub pending: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassStudentRow {
    pub id: String,
    pub student_no: String,
    pub name: String,
    pub mastery: i32,
    pub status: String,
    pub answered: i64,
    pub correct: i64,
    pub pending: i64,
    pub score_rate: f64,
    /// 与 `kps` 等长的掌握度数组；-1 表示这个学生在这个知识点上没有作答。
    pub cells: Vec<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommonWrong {
    pub question_id: String,
    pub excerpt: String,
    pub wrong: i64,
    pub total: i64,
    pub rate: i64,
    pub knowledge: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassInsight {
    pub class_id: String,
    pub class_name: String,
    pub kps: Vec<KpRow>,
    pub students: Vec<ClassStudentRow>,
    pub common_wrong: Vec<CommonWrong>,
    /// 这个班里还没批的作答数。
    pub pending_grade: i64,
    /// 已判分作答的得分率（0–1）。
    pub score_rate: f64,
    /// 参与统计的已判分作答数，用来判断「数据够不够画矩阵」。
    pub graded_total: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrendPoint {
    /// 该周的周一，`11-10` 这种短格式。
    pub week: String,
    pub mastery: i32,
    pub answered: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WrongRow {
    pub id: String,
    pub question_id: String,
    pub excerpt: String,
    pub answer: String,
    pub state: String,
    pub wrong_count: i64,
    pub review_count: i64,
    pub last_wrong_at: String,
    pub next_review_at: Option<String>,
    pub knowledge: Vec<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentRow {
    pub assignment_id: String,
    pub title: String,
    pub submitted_at: Option<String>,
    pub score: f64,
    pub max_score: f64,
    pub pending: i64,
    pub late: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudentInsight {
    pub student_id: String,
    pub student_no: String,
    pub name: String,
    pub grade: String,
    pub overall: i32,
    pub status: String,
    pub answered: i64,
    pub correct: i64,
    pub pending: i64,
    pub score_rate: f64,
    pub kps: Vec<KpRow>,
    pub trend: Vec<TrendPoint>,
    pub wrong_count: i64,
    pub recent: Vec<RecentRow>,
    // —— 以下为「个人也看得见位置」补的字段 ——
    /// 所属班级名（可能不止一个，取排在最前的一个做参照）。
    pub class_names: Vec<String>,
    /// 参照班级的平均掌握度；没有班级或班里没有作答时为 None。
    pub class_avg: Option<i32>,
    /// 在这个班里按掌握度排第几（1 开始），只在有参照班时给。
    pub rank: Option<i64>,
    pub class_size: Option<i64>,
    /// 近两周掌握度变化，正数是进步。
    pub delta: i32,
    /// 最薄弱的 3 个知识点（mastery < 70），前端直接列出「先补这几个」。
    pub weak: Vec<KpBrief>,
}

/// 知识点的一句话摘要，名单里一行放三个。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KpBrief {
    pub id: String,
    pub name: String,
    pub mastery: i32,
    pub answered: i64,
}

/// 学生名单的一行：只放「现在要不要找他」的判断依据，细节留到下钻页。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RosterRow {
    pub id: String,
    pub student_no: String,
    pub name: String,
    pub grade: String,
    pub class_ids: Vec<String>,
    pub class_names: Vec<String>,
    pub mastery: i32,
    pub status: String,
    pub answered: i64,
    pub correct: i64,
    pub pending: i64,
    pub score_rate: f64,
    /// 近两周掌握度变化（正数是进步）。
    pub delta: i32,
    /// 到期该复习的错题数。
    pub due_review: i64,
    /// 最薄弱的 3 个知识点。
    pub weak: Vec<KpBrief>,
    /// 最近一次作答时间；从没交过为 None。
    pub last_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassBrief {
    pub id: String,
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RosterSummary {
    /// 名单人数（受筛选影响）。
    pub total: i64,
    /// 有已判分作答的人数。
    pub active: i64,
    pub weak: i64,
    pub mastered: i64,
    pub due_review: i64,
    /// 超过两周没做题的人数：手感会掉，值得提醒。
    pub idle: i64,
    pub pending: i64,
}

/// 学生维度的学情总览：一个人一个人地看，而不是先进班级再找。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Roster {
    pub students: Vec<RosterRow>,
    pub classes: Vec<ClassBrief>,
    pub summary: RosterSummary,
    pub graded_total: i64,
}

/// 一次已判分的作答，聚合前的最小单位。
#[derive(Debug, Clone)]
struct Graded {
    student_id: String,
    kp_id: String,
    correct: bool,
    at: DateTime<Local>,
    difficulty: i64,
}

// ---------------------------------------------------------------------------
// 公式
// ---------------------------------------------------------------------------

/// 统一公式本体。`rows` 里只放已判分的作答；`as_of` 用来回放历史（趋势）。
fn mastery_of(rows: &[(bool, DateTime<Local>, i64)], as_of: DateTime<Local>) -> Option<i32> {
    let mut weight_sum = 0.0;
    let mut score_sum = 0.0;
    for (correct, at, difficulty) in rows {
        if *at > as_of {
            continue;
        }
        let days = (as_of - *at).num_hours().max(0) as f64 / 24.0;
        let weight = 1.0 / (1.0 + days / 14.0);
        let factor = if *correct {
            0.75 + *difficulty as f64 * 0.05
        } else {
            0.05
        };
        weight_sum += weight;
        score_sum += weight * factor;
    }
    if weight_sum == 0.0 {
        return None;
    }
    Some((score_sum / weight_sum * 100.0).min(100.0).round() as i32)
}

fn status_of(mastery: i32, answered: i64) -> String {
    if answered == 0 {
        "NOT_STARTED".into()
    } else if mastery >= MASTERED_LINE {
        "MASTERED".into()
    } else {
        "LEARNING".into()
    }
}

fn parse_at(raw: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|at| at.with_timezone(&Local))
}

// ---------------------------------------------------------------------------
// 取数
// ---------------------------------------------------------------------------

/// 拉一批已判分作答；`class_id` 与 `student_id` 至少要给一个。
/// 待批（`correct is null`）单独查，不进掌握度。
fn load_graded(
    conn: &Connection,
    class_id: Option<&str>,
    student_id: Option<&str>,
) -> Result<Vec<Graded>, String> {
    let mut sql = String::from(
        "select a.student_id, qk.knowledge_point_id, a.correct, a.submitted_at, q.difficulty
           from local_attempt a
           join question_knowledge qk on qk.question_id = a.question_id
           join question q on q.id = a.question_id
          where a.superseded_at is null and a.correct is not null",
    );
    if let Some(_) = class_id {
        sql.push_str(
            " and a.student_id in (select student_id from local_class_member where class_id = ?1)",
        );
    } else if student_id.is_some() {
        sql.push_str(" and a.student_id = ?1");
    } else {
        return Err("要么给班级，要么给学生".into());
    }

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let key = class_id.or(student_id).unwrap_or_default().to_string();
    let rows = stmt
        .query_map([key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows.flatten() {
        if let Some(at) = parse_at(&row.3) {
            out.push(Graded {
                student_id: row.0,
                kp_id: row.1,
                correct: row.2,
                at,
                difficulty: row.4,
            });
        }
    }
    Ok(out)
}

/// 全量已判分作答（学生名单一次算完，不按班/按人反复查）。
fn load_graded_every(conn: &Connection) -> Result<Vec<Graded>, String> {
    let mut stmt = conn
        .prepare(
            "select a.student_id, qk.knowledge_point_id, a.correct, a.submitted_at, q.difficulty
               from local_attempt a
               join question_knowledge qk on qk.question_id = a.question_id
               join question q on q.id = a.question_id
              where a.superseded_at is null and a.correct is not null",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        if let Some(at) = parse_at(&row.3) {
            out.push(Graded {
                student_id: row.0,
                kp_id: row.1,
                correct: row.2,
                at,
                difficulty: row.4,
            });
        }
    }
    Ok(out)
}

/// 全量待批作答，按 (学生, 知识点) 计数。
fn pending_every(conn: &Connection) -> Result<HashMap<(String, String), i64>, String> {
    let mut stmt = conn
        .prepare(
            "select a.student_id, qk.knowledge_point_id, count(*)
               from local_attempt a
               join question_knowledge qk on qk.question_id = a.question_id
              where a.superseded_at is null and a.correct is null
              group by a.student_id, qk.knowledge_point_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

fn kp_names(conn: &Connection) -> HashMap<String, String> {
    let mut stmt = match conn.prepare("select id, name from knowledge_point") {
        Ok(stmt) => stmt,
        Err(_) => return HashMap::new(),
    };
    stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

/// 到期该复习的错题数，按学生汇总。
fn due_by_student(conn: &Connection) -> HashMap<String, i64> {
    let mut stmt = match conn.prepare(
        "select student_id, count(*) from local_wrong_book
          where deleted_at is null and state <> 'MASTERED'
            and (next_review_at is null or next_review_at <= ?1)
          group by student_id",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return HashMap::new(),
    };
    stmt.query_map([text::now()], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

/// 每个学生的得分率，一次算完（名单里逐个查会变成 N 次 SQL）。
fn score_rate_every(conn: &Connection) -> HashMap<String, f64> {
    let mut stmt = match conn.prepare(
        "select a.student_id, coalesce(sum(a.score), 0), coalesce(sum(a.max_score), 0)
           from local_attempt a
          where a.superseded_at is null and a.correct is not null
          group by a.student_id",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return HashMap::new(),
    };
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, f64>(1)?,
            row.get::<_, f64>(2)?,
        ))
    })
    .map(|rows| {
        rows.flatten()
            .map(|(id, sum, max)| (id, if max > 0.0 { sum / max } else { 0.0 }))
            .collect()
    })
    .unwrap_or_default()
}

/// 掌握度变化：现在 vs 两周前。两周前没有作答就是 0（谈不上涨跌）。
fn delta_of(rows: &[(bool, DateTime<Local>, i64)], now: DateTime<Local>) -> i32 {
    let before = now - Duration::days(14);
    match (mastery_of(rows, before), mastery_of(rows, now)) {
        (Some(old), Some(current)) => current - old,
        _ => 0,
    }
}

/// 某班级用到的知识点（按作业里出现的题反推），保持课程顺序。
fn class_kps(conn: &Connection, class_id: &str) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "select kp.id, kp.name
               from knowledge_point kp
              where kp.id in (
                    select qk.knowledge_point_id
                      from local_assignment_question aq
                      join local_assignment asg
                        on asg.id = aq.assignment_id and asg.deleted_at is null
                      join question_knowledge qk on qk.question_id = aq.question_id
                     where asg.class_id = ?1
              )
              order by kp.sort, kp.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([class_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

/// 补齐学生自己的知识点：他做过题但班级作业里没覆盖到的，也要能下钻看到。
fn student_kps(conn: &Connection, student_id: &str) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "select kp.id, kp.name
               from knowledge_point kp
              where kp.id in (
                    select qk.knowledge_point_id
                      from local_attempt a
                      join question_knowledge qk on qk.question_id = a.question_id
                     where a.student_id = ?1 and a.superseded_at is null
              )
              order by kp.sort, kp.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([student_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

fn pending_by_kp(
    conn: &Connection,
    class_id: Option<&str>,
    student_id: Option<&str>,
) -> Result<HashMap<(String, String), i64>, String> {
    let mut sql = String::from(
        "select a.student_id, qk.knowledge_point_id, count(*)
           from local_attempt a
           join question_knowledge qk on qk.question_id = a.question_id
          where a.superseded_at is null and a.correct is null",
    );
    if class_id.is_some() {
        sql.push_str(
            " and a.student_id in (select student_id from local_class_member where class_id = ?1)",
        );
    } else {
        sql.push_str(" and a.student_id = ?1");
    }
    sql.push_str(" group by a.student_id, qk.knowledge_point_id");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let key = class_id.or(student_id).unwrap_or_default().to_string();
    let rows = stmt
        .query_map([key], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

/// 一个学生的整体掌握度 + 得分率，班级列表和详情共用。
fn summarize(rows: &[Graded], now: DateTime<Local>) -> (i32, i64, i64) {
    let flat: Vec<(bool, DateTime<Local>, i64)> = rows
        .iter()
        .map(|row| (row.correct, row.at, row.difficulty))
        .collect();
    let mastery = mastery_of(&flat, now).unwrap_or(0);
    let answered = rows.len() as i64;
    let correct = rows.iter().filter(|row| row.correct).count() as i64;
    (mastery, answered, correct)
}

fn score_rate(conn: &Connection, class_id: Option<&str>, student_id: &str) -> f64 {
    let sql = if class_id.is_some() {
        "select coalesce(sum(a.score), 0), coalesce(sum(a.max_score), 0)
           from local_attempt a
           join local_assignment asg on asg.id = a.assignment_id and asg.deleted_at is null
          where a.superseded_at is null and a.correct is not null
            and a.student_id in (select student_id from local_class_member where class_id = ?1)"
    } else {
        "select coalesce(sum(a.score), 0), coalesce(sum(a.max_score), 0)
           from local_attempt a
          where a.superseded_at is null and a.correct is not null and a.student_id = ?1"
    };
    let key = class_id.unwrap_or(student_id).to_string();
    conn.query_row(sql, [key], |row| {
        Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?))
    })
    .map(|(sum, max)| if max > 0.0 { sum / max } else { 0.0 })
    .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// 班级视图
// ---------------------------------------------------------------------------

pub fn class_insight(conn: &Connection, class_id: &str) -> Result<ClassInsight, String> {
    let (class_name,): (String,) = conn
        .query_row(
            "select name from local_class where id = ?1 and deleted_at is null",
            [class_id],
            |row| Ok((row.get(0)?,)),
        )
        .map_err(|_| "找不到这个班级".to_string())?;
    let now = Local::now();

    let kps = class_kps(conn, class_id)?;
    let kp_index: HashMap<&str, usize> = kps
        .iter()
        .enumerate()
        .map(|(index, (id, _))| (id.as_str(), index))
        .collect();
    let graded = load_graded(conn, Some(class_id), None)?;
    let pending = pending_by_kp(conn, Some(class_id), None)?;

    // (学生, 知识点) → 作答列表
    let mut by_cell: HashMap<(String, String), Vec<(bool, DateTime<Local>, i64)>> = HashMap::new();
    let mut by_student: HashMap<String, Vec<Graded>> = HashMap::new();
    for row in &graded {
        by_cell
            .entry((row.student_id.clone(), row.kp_id.clone()))
            .or_default()
            .push((row.correct, row.at, row.difficulty));
        by_student
            .entry(row.student_id.clone())
            .or_default()
            .push(row.clone());
    }

    // 知识点列汇总：把这个知识点下全班所有作答摊平算。
    let mut kp_rows = Vec::with_capacity(kps.len());
    for (id, name) in &kps {
        let cells: Vec<(bool, DateTime<Local>, i64)> = by_cell
            .iter()
            .filter(|((_, kp), _)| kp == id)
            .flat_map(|(_, values)| values.clone())
            .collect();
        let mastery = mastery_of(&cells, now).unwrap_or(0);
        let answered = cells.len() as i64;
        let correct = cells.iter().filter(|(ok, _, _)| *ok).count() as i64;
        let pending_count: i64 = pending
            .iter()
            .filter(|((_, kp), _)| kp == id)
            .map(|(_, count)| *count)
            .sum();
        kp_rows.push(KpRow {
            id: id.clone(),
            name: name.clone(),
            mastery,
            status: status_of(mastery, answered),
            answered,
            correct,
            pending: pending_count,
        });
    }

    let mut stmt = conn
        .prepare(
            "select s.id, s.name, s.student_no
               from local_student s
               join local_class_member m on m.student_id = s.id
              where m.class_id = ?1 and s.deleted_at is null and s.status = 'ACTIVE'
              order by s.name",
        )
        .map_err(|e| e.to_string())?;
    let members: Vec<(String, String, String)> = stmt
        .query_map([class_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();

    let mut students = Vec::with_capacity(members.len());
    for (id, name, student_no) in members {
        let mine = by_student.get(&id).cloned().unwrap_or_default();
        let (mastery, answered, correct) = summarize(&mine, now);
        let mut cells = vec![-1i32; kps.len()];
        for ((student, kp), values) in &by_cell {
            if student != &id {
                continue;
            }
            if let Some(index) = kp_index.get(kp.as_str()) {
                cells[*index] = mastery_of(values, now).unwrap_or(-1);
            }
        }
        let pending_count: i64 = pending
            .iter()
            .filter(|((student, _), _)| student == &id)
            .map(|(_, count)| *count)
            .sum();
        students.push(ClassStudentRow {
            id: id.clone(),
            student_no,
            name,
            mastery,
            status: status_of(mastery, answered),
            answered,
            correct,
            pending: pending_count,
            score_rate: score_rate(conn, Some(class_id), &id),
            cells,
        });
    }
    // 薄弱的排前面：老师最该看的是一眼扫到谁掉队。
    students.sort_by(|a, b| a.mastery.cmp(&b.mastery).then(a.name.cmp(&b.name)));

    Ok(ClassInsight {
        class_id: class_id.to_string(),
        class_name,
        kps: kp_rows,
        students,
        common_wrong: common_wrong(conn, class_id, 5)?,
        pending_grade: conn
            .query_row(
                "select count(*) from local_attempt a
                   join local_assignment asg on asg.id = a.assignment_id and asg.deleted_at is null
                  where a.superseded_at is null and a.correct is null and asg.class_id = ?1",
                [class_id],
                |row| row.get(0),
            )
            .unwrap_or(0),
        score_rate: score_rate(conn, Some(class_id), ""),
        graded_total: graded.len() as i64,
    })
}

/// 共性错题：同一个班做错的人次最多的题。
fn common_wrong(conn: &Connection, class_id: &str, limit: i64) -> Result<Vec<CommonWrong>, String> {
    let mut stmt = conn
        .prepare(
            "select a.question_id, q.stem, q.answer,
                    sum(case when a.correct = 0 then 1 else 0 end) as wrong, count(*) as total
               from local_attempt a
               join question q on q.id = a.question_id
               join local_assignment asg on asg.id = a.assignment_id and asg.deleted_at is null
              where a.superseded_at is null and a.correct is not null and asg.class_id = ?1
              group by a.question_id
             having wrong > 0
              order by wrong desc, wrong * 1.0 / total desc
              limit ?2",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![class_id, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(CommonWrong {
            question_id: row.0.clone(),
            excerpt: text::excerpt(&row.1, 60),
            wrong: row.3,
            total: row.4,
            rate: if row.4 > 0 { row.3 * 100 / row.4 } else { 0 },
            knowledge: knowledge_names(conn, &row.0),
        });
    }
    Ok(out)
}

fn knowledge_names(conn: &Connection, question_id: &str) -> Vec<String> {
    let mut stmt = match conn.prepare(
        "select kp.name from question_knowledge qk
           join knowledge_point kp on kp.id = qk.knowledge_point_id
          where qk.question_id = ?1 order by kp.sort limit 3",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([question_id], |row| row.get::<_, String>(0))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 学生名单（个人视角的入口）
// ---------------------------------------------------------------------------

/// 薄弱知识点线：低于这个分数就值得单独补一补（与「按薄弱点出题」同一口径）。
const WEAK_KP_LINE: i32 = 70;

/// 学生名单：老师想「看看某个孩子最近怎么样」时的第一屏。
///
/// 与班级视图的分工——这里横向比人（谁掉队、谁在退步、谁该复习），
/// 点进去再纵向看一个人的知识点、趋势与错题本。筛选放在前端，数据一次给全，
/// 一个班几十上百人也就是一次查询 + 内存聚合的量级。
pub fn roster(
    conn: &Connection,
    class_id: Option<&str>,
    keyword: Option<&str>,
) -> Result<Roster, String> {
    let now = Local::now();

    let mut stmt = conn
        .prepare(
            "select id, name, grade, student_no from local_student
              where deleted_at is null and status = 'ACTIVE' order by name",
        )
        .map_err(|e| e.to_string())?;
    let students: Vec<(String, String, String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();

    // 班级、成员、每班人数：一次读完，前端切班级不用再请求。
    let mut stmt = conn
        .prepare(
            "select c.id, c.name, m.student_id
               from local_class c
               join local_class_member m on m.class_id = c.id
              where c.deleted_at is null
              order by c.name",
        )
        .map_err(|e| e.to_string())?;
    let triples: Vec<(String, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();

    let mut classes: Vec<ClassBrief> = Vec::new();
    let mut members: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (cid, cname, sid) in triples {
        match classes.iter_mut().find(|row| row.id == cid) {
            Some(row) => row.count += 1,
            None => classes.push(ClassBrief {
                id: cid.clone(),
                name: cname.clone(),
                count: 1,
            }),
        }
        members.entry(sid).or_default().push((cid, cname));
    }

    let needle = keyword.unwrap_or("").trim().to_lowercase();
    let picked: Vec<(String, String, String, String)> = students
        .into_iter()
        .filter(|(id, name, grade, student_no)| {
            if let Some(cid) = class_id.filter(|value| !value.is_empty()) {
                let inside = members
                    .get(id)
                    .map(|rows| rows.iter().any(|(class, _)| class == cid))
                    .unwrap_or(false);
                if !inside {
                    return false;
                }
            }
            needle.is_empty()
                || name.to_lowercase().contains(&needle)
                || grade.to_lowercase().contains(&needle)
                || student_no.contains(&needle)
        })
        .collect();

    let graded_all = load_graded_every(conn)?;
    let pending_all = pending_every(conn)?;
    let names = kp_names(conn);
    let dues = due_by_student(conn);
    let rates = score_rate_every(conn);

    let wanted: std::collections::HashSet<&str> =
        picked.iter().map(|(id, _, _, _)| id.as_str()).collect();
    let mut by_student: HashMap<&str, Vec<&Graded>> = HashMap::new();
    let mut graded_total = 0i64;
    for row in &graded_all {
        let student = row.student_id.as_str();
        if !wanted.contains(student) {
            continue;
        }
        graded_total += 1;
        by_student.entry(student).or_default().push(row);
    }

    let mut summary = RosterSummary {
        total: 0,
        active: 0,
        weak: 0,
        mastered: 0,
        due_review: 0,
        idle: 0,
        pending: 0,
    };
    let mut rows = Vec::with_capacity(picked.len());

    for (id, name, grade, student_no) in &picked {
        let mine = by_student.get(id.as_str()).cloned().unwrap_or_default();
        let flat: Vec<(bool, DateTime<Local>, i64)> = mine
            .iter()
            .map(|row| (row.correct, row.at, row.difficulty))
            .collect();
        let mastery = mastery_of(&flat, now).unwrap_or(0);
        let answered = flat.len() as i64;
        let correct = flat.iter().filter(|(ok, _, _)| *ok).count() as i64;

        let mut by_kp: HashMap<&str, Vec<(bool, DateTime<Local>, i64)>> = HashMap::new();
        for row in &mine {
            by_kp.entry(row.kp_id.as_str()).or_default().push((
                row.correct,
                row.at,
                row.difficulty,
            ));
        }
        let mut weak: Vec<KpBrief> = by_kp
            .iter()
            .filter_map(|(kp, values)| {
                let value = mastery_of(values, now)?;
                if value >= WEAK_KP_LINE {
                    return None;
                }
                Some(KpBrief {
                    id: kp.to_string(),
                    name: names.get(*kp).cloned().unwrap_or_else(|| kp.to_string()),
                    mastery: value,
                    answered: values.len() as i64,
                })
            })
            .collect();
        weak.sort_by(|a, b| a.mastery.cmp(&b.mastery));
        weak.truncate(3);

        let pending_count: i64 = pending_all
            .iter()
            .filter(|((student, _), _)| student == id)
            .map(|(_, count)| *count)
            .sum();
        let due = dues.get(id).copied().unwrap_or(0);
        let last = mine.iter().map(|row| row.at).max();

        summary.total += 1;
        if answered > 0 {
            summary.active += 1;
        }
        if answered > 0 && mastery > 0 && mastery < 40 {
            summary.weak += 1;
        }
        if answered > 0 && mastery >= MASTERED_LINE {
            summary.mastered += 1;
        }
        summary.due_review += due;
        summary.pending += pending_count;
        if last
            .map(|at| now - at > Duration::days(14))
            .unwrap_or(false)
        {
            summary.idle += 1;
        }

        let (class_ids, class_names) = members
            .get(id.as_str())
            .map(|rows| {
                (
                    rows.iter().map(|(cid, _)| cid.clone()).collect(),
                    rows.iter().map(|(_, cname)| cname.clone()).collect(),
                )
            })
            .unwrap_or_default();

        rows.push(RosterRow {
            id: id.clone(),
            student_no: student_no.clone(),
            name: name.clone(),
            grade: grade.clone(),
            class_ids,
            class_names,
            mastery,
            status: status_of(mastery, answered),
            answered,
            correct,
            pending: pending_count,
            score_rate: rates.get(id).copied().unwrap_or(0.0),
            delta: delta_of(&flat, now),
            due_review: due,
            weak,
            last_at: last.map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, false)),
        });
    }

    // 薄弱的排前面；还没交过作业的排最后，别把「没数据」当成「0 分」混淆。
    rows.sort_by(|a, b| {
        (a.answered == 0)
            .cmp(&(b.answered == 0))
            .then(a.mastery.cmp(&b.mastery))
            .then(a.name.cmp(&b.name))
    });

    Ok(Roster {
        students: rows,
        classes,
        summary,
        graded_total,
    })
}

// ---------------------------------------------------------------------------
// 学生视图
// ---------------------------------------------------------------------------

pub fn student_insight(conn: &Connection, student_id: &str) -> Result<StudentInsight, String> {
    let (name, grade, student_no): (String, String, String) = conn
        .query_row(
            "select name, grade, student_no from local_student where id = ?1 and deleted_at is null",
            [student_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| "找不到这个学生".to_string())?;
    let now = Local::now();

    let graded = load_graded(conn, None, Some(student_id))?;
    let pending = pending_by_kp(conn, None, Some(student_id))?;
    let (overall, answered, correct) = summarize(&graded, now);

    let mut by_kp: HashMap<String, Vec<(bool, DateTime<Local>, i64)>> = HashMap::new();
    for row in &graded {
        by_kp
            .entry(row.kp_id.clone())
            .or_default()
            .push((row.correct, row.at, row.difficulty));
    }

    // 学生自己做过的知识点 + 所在班级作业覆盖到的，合并去重（一个知识点只出现一次）。
    let mut kps = student_kps(conn, student_id)?;
    for (id, name) in class_kps_of_student(conn, student_id) {
        if !kps.iter().any(|(existing, _)| existing == &id) {
            kps.push((id, name));
        }
    }

    let mut rows = Vec::with_capacity(kps.len());
    for (id, kp_name) in &kps {
        let values = by_kp.get(id).cloned().unwrap_or_default();
        let mastery = mastery_of(&values, now).unwrap_or(0);
        let total = values.len() as i64;
        let ok = values.iter().filter(|(hit, _, _)| *hit).count() as i64;
        let pending_count: i64 = pending
            .iter()
            .filter(|((_, kp), _)| kp == id)
            .map(|(_, count)| *count)
            .sum();
        rows.push(KpRow {
            id: id.clone(),
            name: kp_name.clone(),
            mastery,
            status: status_of(mastery, total),
            answered: total,
            correct: ok,
            pending: pending_count,
        });
    }
    // 薄弱的排前面；没作答的知识点是「还没练」，不是「0 分」，排到最后。
    rows.sort_by(|a, b| {
        (a.answered == 0)
            .cmp(&(b.answered == 0))
            .then(a.mastery.cmp(&b.mastery))
            .then(a.name.cmp(&b.name))
    });

    let flat: Vec<(bool, DateTime<Local>, i64)> = graded
        .iter()
        .map(|row| (row.correct, row.at, row.difficulty))
        .collect();
    let weak: Vec<KpBrief> = rows
        .iter()
        .filter(|row| row.answered > 0 && row.mastery < WEAK_KP_LINE)
        .take(3)
        .map(|row| KpBrief {
            id: row.id.clone(),
            name: row.name.clone(),
            mastery: row.mastery,
            answered: row.answered,
        })
        .collect();
    let peers = peers_of(conn, student_id, now);

    Ok(StudentInsight {
        student_id: student_id.to_string(),
        student_no,
        name,
        grade,
        overall,
        status: status_of(overall, answered),
        answered,
        correct,
        pending: pending.values().sum(),
        score_rate: score_rate(conn, None, student_id),
        kps: rows,
        trend: trend(&graded, now, 8),
        wrong_count: conn
            .query_row(
                "select count(*) from local_wrong_book
                  where student_id = ?1 and deleted_at is null and state <> 'MASTERED'",
                [student_id],
                |row| row.get(0),
            )
            .unwrap_or(0),
        recent: recent(conn, student_id)?,
        class_names: peers.names.iter().map(|(_, name)| name.clone()).collect(),
        class_avg: peers.avg,
        rank: peers.rank,
        class_size: peers.size,
        delta: delta_of(&flat, now),
        weak,
    })
}

/// 学生在班里的位置：只看排在最前的那个班，避免「跨班平均」把口径搅浑。
#[derive(Default)]
struct Peers {
    names: Vec<(String, String)>,
    avg: Option<i32>,
    rank: Option<i64>,
    size: Option<i64>,
}

fn peers_of(conn: &Connection, student_id: &str, now: DateTime<Local>) -> Peers {
    let mut stmt = match conn.prepare(
        "select c.id, c.name
           from local_class c
           join local_class_member m on m.class_id = c.id
          where m.student_id = ?1 and c.deleted_at is null
          order by c.name",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Peers::default(),
    };
    let names: Vec<(String, String)> = stmt
        .query_map([student_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();
    let Some((class_id, _)) = names.first() else {
        return Peers::default();
    };

    let graded = load_graded(conn, Some(class_id), None).unwrap_or_default();
    let mut by_student: HashMap<&str, Vec<(bool, DateTime<Local>, i64)>> = HashMap::new();
    for row in &graded {
        by_student
            .entry(row.student_id.as_str())
            .or_default()
            .push((row.correct, row.at, row.difficulty));
    }
    // 只给有已判分作答的人排名：没交作业不参与比较。
    let mut scores: Vec<(String, i32)> = by_student
        .iter()
        .map(|(id, values)| (id.to_string(), mastery_of(values, now).unwrap_or(0)))
        .collect();
    scores.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let avg = if scores.is_empty() {
        None
    } else {
        let sum: i64 = scores.iter().map(|(_, value)| *value as i64).sum();
        Some((sum / scores.len() as i64) as i32)
    };
    Peers {
        rank: scores
            .iter()
            .position(|(id, _)| id == student_id)
            .map(|index| index as i64 + 1),
        size: Some(scores.len() as i64),
        avg,
        names,
    }
}

/// 学生参与过的班级里用到的知识点。
fn class_kps_of_student(conn: &Connection, student_id: &str) -> Vec<(String, String)> {
    let mut stmt = match conn.prepare(
        "select kp.id, kp.name
           from knowledge_point kp
          where kp.id in (
                select qk.knowledge_point_id
                  from local_assignment_question aq
                  join local_assignment asg
                    on asg.id = aq.assignment_id and asg.deleted_at is null
                  join question_knowledge qk on qk.question_id = aq.question_id
                 where asg.class_id in (
                       select class_id from local_class_member where student_id = ?1
                 )
          )",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map([student_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

/// 最近 8 周的掌握度曲线：每周把「到周日为止」的作答重算一遍。
fn trend(graded: &[Graded], now: DateTime<Local>, weeks: usize) -> Vec<TrendPoint> {
    let today = now.date_naive();
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let all: Vec<(bool, DateTime<Local>, i64)> = graded
        .iter()
        .map(|row| (row.correct, row.at, row.difficulty))
        .collect();

    let mut out = Vec::with_capacity(weeks);
    for back in (0..weeks as i64).rev() {
        let start: NaiveDate = monday - Duration::days(back * 7);
        let end_date = start + Duration::days(7);
        let end = end_date
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| Local.from_local_datetime(&naive).single())
            .unwrap_or(now)
            .min(now);
        let mastery = mastery_of(&all, end).unwrap_or(0);
        let answered = all.iter().filter(|(_, at, _)| *at <= end).count() as i64;
        out.push(TrendPoint {
            week: start.format("%m-%d").to_string(),
            mastery,
            answered,
        });
    }
    out
}

/// 最近的作业成绩（每个作业取最新一次提交）。
fn recent(conn: &Connection, student_id: &str) -> Result<Vec<RecentRow>, String> {
    let mut stmt = conn
        .prepare(
            "select asg.id, asg.title,
                    max(a.submitted_at) as submitted_at,
                    coalesce(sum(a.score), 0) as score,
                    coalesce(sum(a.max_score), 0) as max_score,
                    sum(case when a.correct is null then 1 else 0 end) as pending,
                    max(a.late) as late
               from local_attempt a
               join local_assignment asg on asg.id = a.assignment_id and asg.deleted_at is null
              where a.student_id = ?1 and a.superseded_at is null
              group by asg.id
              order by submitted_at desc
              limit 8",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([student_id], |row| {
            Ok(RecentRow {
                assignment_id: row.get(0)?,
                title: row.get(1)?,
                submitted_at: row.get(2)?,
                score: row.get(3)?,
                max_score: row.get(4)?,
                pending: row.get(5)?,
                late: row.get::<_, i64>(6)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

// ---------------------------------------------------------------------------
// 错题本
// ---------------------------------------------------------------------------

pub fn wrong_list(
    conn: &Connection,
    student_id: &str,
    state: Option<&str>,
) -> Result<Vec<WrongRow>, String> {
    let mut sql = String::from(
        "select w.id, w.question_id, q.stem, q.answer, w.state, w.wrong_count,
                w.review_count, w.last_wrong_at, w.next_review_at, w.note
           from local_wrong_book w
           join question q on q.id = w.question_id
          where w.student_id = ?1 and w.deleted_at is null",
    );
    if state.is_some() && state != Some("") && state != Some("ALL") {
        sql.push_str(" and w.state = ?2");
    }
    sql.push_str(" order by w.last_wrong_at desc limit 200");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<WrongRow> {
        Ok(WrongRow {
            id: row.get(0)?,
            question_id: row.get(1)?,
            excerpt: text::excerpt(&row.get::<_, String>(2)?, 70),
            answer: row.get(3)?,
            state: row.get(4)?,
            wrong_count: row.get(5)?,
            review_count: row.get(6)?,
            last_wrong_at: row.get(7)?,
            next_review_at: row.get(8)?,
            knowledge: Vec::new(),
            note: row.get(9)?,
        })
    };

    let mut out: Vec<WrongRow> =
        if let Some(state) = state.filter(|value| !value.is_empty() && *value != "ALL") {
            stmt.query_map(params![student_id, state], map_row)
                .map_err(|e| e.to_string())?
                .flatten()
                .collect()
        } else {
            stmt.query_map([student_id], map_row)
                .map_err(|e| e.to_string())?
                .flatten()
                .collect()
        };
    for row in &mut out {
        row.knowledge = knowledge_names(conn, &row.question_id);
    }
    Ok(out)
}

/// 错题入本：同一题重复做错只累加次数，把状态拉回 NEW 并安排次日复习。
pub fn record_wrong(conn: &Connection, student_id: &str, question_id: &str, attempt_id: &str) {
    let now = text::now();
    let next = next_review(1);
    let existing: Option<(String, i64)> = conn
        .query_row(
            "select id, wrong_count from local_wrong_book
              where student_id = ?1 and question_id = ?2 and deleted_at is null",
            params![student_id, question_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .unwrap_or(None);

    match existing {
        Some((id, count)) => {
            let _ = conn.execute(
                "update local_wrong_book
                    set wrong_count = ?2, source_attempt_id = ?3, state = 'NEW',
                        last_wrong_at = ?4, next_review_at = ?5, updated_at = ?4
                  where id = ?1",
                params![id, count + 1, attempt_id, now, next],
            );
        }
        None => {
            let _ = conn.execute(
                "insert into local_wrong_book
                    (id, student_id, question_id, source_attempt_id, state, wrong_count,
                     last_wrong_at, next_review_at, review_count, note, created_at, updated_at)
                 values (?1, ?2, ?3, ?4, 'NEW', 1, ?5, ?6, 0, '', ?5, ?5)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    student_id,
                    question_id,
                    attempt_id,
                    now,
                    next
                ],
            );
        }
    }
}

/// 后来做对了：不删记录，标成已掌握，复习提醒取消。
pub fn resolve_wrong(conn: &Connection, student_id: &str, question_id: &str, attempt_id: &str) {
    let _ = conn.execute(
        "update local_wrong_book
            set state = 'MASTERED', next_review_at = null, source_attempt_id = ?3, updated_at = ?4
          where student_id = ?1 and question_id = ?2 and deleted_at is null",
        params![student_id, question_id, attempt_id, text::now()],
    );
}

/// 复习间隔：第一次 1 天，之后 3 天、7 天，四轮之后当作已掌握只看不催。
fn next_review(days: i64) -> String {
    (Local::now() + Duration::days(days)).to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewInput {
    pub id: String,
    /// `NEW` / `REVIEWING` / `MASTERED`。
    pub state: String,
    #[serde(default)]
    pub note: String,
}

pub fn wrong_review(conn: &Connection, input: &ReviewInput) -> Result<(), String> {
    let state = input.state.to_uppercase();
    if !["NEW", "REVIEWING", "MASTERED"].contains(&state.as_str()) {
        return Err(format!("不认识的状态：{}", input.state));
    }
    let (count,): (i64,) = conn
        .query_row(
            "select review_count from local_wrong_book where id = ?1 and deleted_at is null",
            [&input.id],
            |row| Ok((row.get(0)?,)),
        )
        .map_err(|_| "找不到这条错题".to_string())?;

    // 掌握就停催；在复习按 1/3/7 天递进；重置回 NEW 则次日再来。
    let next: Option<String> = match state.as_str() {
        "MASTERED" => None,
        "REVIEWING" => Some(next_review(if count >= 2 { 7 } else { 3 })),
        _ => Some(next_review(1)),
    };
    let note = input.note.trim();
    conn.execute(
        "update local_wrong_book
            set state = ?2, next_review_at = ?3, review_count = review_count + 1,
                note = case when ?4 = '' then note else ?4 end,
                updated_at = ?5
          where id = ?1",
        params![input.id, state, next, note, text::now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn wrong_remove(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute(
        "update local_wrong_book set deleted_at = ?2, updated_at = ?2 where id = ?1",
        params![id, text::now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 首页/侧栏上的待复习数量，按学生合并。
pub fn due_review(conn: &Connection, student_id: Option<&str>) -> Result<i64, String> {
    let now = text::now();
    let count = match student_id {
        Some(id) => conn.query_row(
            "select count(*) from local_wrong_book
              where student_id = ?1 and deleted_at is null and state <> 'MASTERED'
                and (next_review_at is null or next_review_at <= ?2)",
            params![id, now],
            |row| row.get(0),
        ),
        None => conn.query_row(
            "select count(*) from local_wrong_book
              where deleted_at is null and state <> 'MASTERED'
                and (next_review_at is null or next_review_at <= ?1)",
            [now],
            |row| row.get(0),
        ),
    };
    count.map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn at(days_ago: i64) -> DateTime<Local> {
        Local::now() - Duration::days(days_ago)
    }

    /// 中心规格的 5 组固定向量（shared/mastery 的同一组数），本地实现必须一致。
    #[test]
    fn formula_matches_shared_vectors() {
        // 今天的 3 题全对、难度 3：factor 0.90 → 90
        let rows = vec![(true, at(0), 3), (true, at(0), 3), (true, at(0), 3)];
        assert_eq!(mastery_of(&rows, Local::now()), Some(90));

        // 全错只剩 5 分
        let rows = vec![(false, at(0), 3), (false, at(0), 3)];
        assert_eq!(mastery_of(&rows, Local::now()), Some(5));

        // 难度拉满的满分题是 100，不会再超过
        let rows = vec![(true, at(0), 5), (true, at(0), 5)];
        assert_eq!(mastery_of(&rows, Local::now()), Some(100));

        // 对一次错一次、同一天： (0.90 + 0.05) / 2 = 47.5 → 48
        let rows = vec![(true, at(0), 3), (false, at(0), 3)];
        assert_eq!(mastery_of(&rows, Local::now()), Some(48));

        // 14 天前的满分题权重只剩一半（0.5），和今天的错题（权重 1）拉平：
        // (0.5*1.0 + 1*0.05) / 1.5 = 0.3667 → 37
        let rows = vec![(true, at(14), 5), (false, at(0), 3)];
        let value = mastery_of(&rows, Local::now()).unwrap();
        assert!(
            (36..=38).contains(&value),
            "衰减后应在 37 附近，实际 {value}"
        );
    }

    #[test]
    fn decay_favours_recent_answers() {
        // 很久以前全错、最近全对：结果应该高（旧错被衰减掉了）
        let mut rows = vec![(false, at(60), 3), (false, at(60), 3)];
        rows.push((true, at(0), 3));
        // 两个月前的两条错题各只剩 0.19 的权重，被今天这条对题（权重 1）拉起来：
        // (0.189*0.05*2 + 0.90) / 1.378 = 0.667 → 67
        let mastery = mastery_of(&rows, Local::now()).unwrap();
        assert!(
            mastery >= 65,
            "近因加权后应明显高于「两错一对」，实际 {mastery}"
        );
    }

    #[test]
    fn empty_attempts_have_no_mastery() {
        assert_eq!(mastery_of(&[], Local::now()), None);
        assert_eq!(status_of(0, 0), "NOT_STARTED");
        assert_eq!(status_of(90, 3), "MASTERED");
        assert_eq!(status_of(40, 3), "LEARNING");
    }

    #[test]
    fn trend_has_eight_weeks_and_last_point_is_now() {
        let rows = vec![
            Graded {
                student_id: "s1".into(),
                kp_id: "k1".into(),
                correct: true,
                at: at(0),
                difficulty: 3,
            },
            Graded {
                student_id: "s1".into(),
                kp_id: "k1".into(),
                correct: false,
                at: at(30),
                difficulty: 3,
            },
        ];
        let points = trend(&rows, Local::now(), 8);
        assert_eq!(points.len(), 8);
        assert_eq!(points[7].answered, 2, "最后一周应包含全部作答");
        assert!(points[0].answered <= points[7].answered);
    }
}
