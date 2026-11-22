//! 成绩：整卷总分（考试 / 小测）+ 按人一行的分数。
//!
//! 与「作业」分工：
//! - 作业按题判分，分数是系统汇总出来的，改分走批改台；
//! - 考试常常是整卷一个总分（纸笔考完直接登分），所以单独一张表。
//!
//! 约定：分数 `null` 表示缺考或还没录，与 0 分严格区分——统计时不计入平均，
//! 名次里也不占位。及格线 / 优秀线留空则按满分的 60% / 85% 算。

use crate::text;
use chrono::Local;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const KINDS: &[&str] = &["EXAM", "QUIZ", "UNIT"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExamRow {
    pub id: String,
    pub class_id: String,
    pub class_name: String,
    pub title: String,
    pub kind: String,
    pub full_score: f64,
    pub pass_line: f64,
    pub good_line: f64,
    pub taken_at: String,
    pub note: String,
    /// 已登分人数（不含缺考）。
    pub entered: i64,
    /// 班里在册人数。
    pub student_count: i64,
    pub average: Option<f64>,
    pub highest: Option<f64>,
    pub lowest: Option<f64>,
    pub pass_rate: Option<f64>,
    pub good_rate: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExamInput {
    pub id: Option<String>,
    pub class_id: String,
    pub title: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default = "hundred")]
    pub full_score: f64,
    #[serde(default)]
    pub pass_line: Option<f64>,
    #[serde(default)]
    pub good_line: Option<f64>,
    /// 考试日期，`2026-11-18`；留空取今天。
    #[serde(default)]
    pub taken_at: Option<String>,
    #[serde(default)]
    pub note: String,
}

fn hundred() -> f64 {
    100.0
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreInput {
    pub student_id: String,
    /// null = 缺考 / 未录入。
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreCell {
    pub student_id: String,
    pub student_no: String,
    pub name: String,
    pub score: Option<f64>,
    /// 班内名次（并列同名次）；没有分数时为 None。
    pub rank: Option<i64>,
    pub note: String,
    /// 与班均的差，正数是高于平均。
    pub vs_average: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DistBucket {
    pub label: String,
    pub from: f64,
    pub to: f64,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExamStats {
    pub entered: i64,
    pub missing: i64,
    pub average: Option<f64>,
    pub highest: Option<f64>,
    pub lowest: Option<f64>,
    pub median: Option<f64>,
    pub pass: i64,
    pub good: i64,
    pub pass_rate: Option<f64>,
    pub good_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExamSheet {
    pub exam: ExamRow,
    pub rows: Vec<ScoreCell>,
    pub stats: ExamStats,
    pub dist: Vec<DistBucket>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudentExam {
    pub exam_id: String,
    pub title: String,
    pub kind: String,
    pub taken_at: String,
    pub class_name: String,
    pub full_score: f64,
    pub score: Option<f64>,
    pub rank: Option<i64>,
    pub class_size: i64,
    pub class_avg: Option<f64>,
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn kind_of(raw: &str) -> String {
    let value = raw.trim().to_uppercase();
    if KINDS.contains(&value.as_str()) {
        value
    } else {
        "EXAM".into()
    }
}

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn pass_of(full: f64, pass: Option<f64>) -> f64 {
    pass.unwrap_or_else(|| round1(full * 0.6))
}

pub fn good_of(full: f64, good: Option<f64>) -> f64 {
    good.unwrap_or_else(|| round1(full * 0.85))
}

pub fn exam_list(conn: &Connection, class_id: Option<&str>) -> Result<Vec<ExamRow>, String> {
    let mut sql = String::from(EXAM_SELECT);
    sql.push_str(" where e.deleted_at is null");
    if class_id.filter(|value| !value.is_empty()).is_some() {
        sql.push_str(" and e.class_id = ?1");
    }
    sql.push_str(" order by e.taken_at desc, e.created_at desc");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<ExamRow> = match class_id.filter(|value| !value.is_empty()) {
        Some(id) => stmt
            .query_map([id], map_exam)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect(),
        None => stmt
            .query_map([], map_exam)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect(),
    };
    Ok(rows)
}

pub fn exam_get(conn: &Connection, exam_id: &str) -> Result<ExamRow, String> {
    conn.query_row(
        &format!("{EXAM_SELECT} where e.id = ?1 and e.deleted_at is null"),
        [exam_id],
        map_exam,
    )
    .map_err(|_| "找不到这次考试".to_string())
}

pub fn exam_save(conn: &Connection, input: &ExamInput) -> Result<String, String> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err("考试名称不能为空".into());
    }
    let full = input.full_score;
    if !(full > 0.0 && full <= 1000.0) {
        return Err("满分要在 0 到 1000 之间".into());
    }
    let pass = input.pass_line.filter(|value| *value >= 0.0);
    let good = input.good_line.filter(|value| *value >= 0.0);
    if let (Some(pass), Some(good)) = (pass, good) {
        if good < pass {
            return Err("优秀线不能低于及格线".into());
        }
    }
    if pass.unwrap_or(0.0) > full || good.unwrap_or(0.0) > full {
        return Err("及格线 / 优秀线不能高于满分".into());
    }
    let class_ok: bool = conn
        .query_row(
            "select exists(select 1 from local_class where id = ?1 and deleted_at is null)",
            [&input.class_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .unwrap_or(false);
    if !class_ok {
        return Err("找不到这个班级".into());
    }
    let taken_at = input
        .taken_at
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(today);
    let now = text::now();
    match input.id.as_deref().filter(|value| !value.is_empty()) {
        Some(id) => {
            let changed = conn
                .execute(
                    "update local_exam
                        set title = ?2, kind = ?3, full_score = ?4, pass_line = ?5,
                            good_line = ?6, taken_at = ?7, note = ?8, updated_at = ?9
                      where id = ?1 and deleted_at is null",
                    params![
                        id,
                        title,
                        kind_of(&input.kind),
                        full,
                        pass,
                        good,
                        taken_at,
                        input.note.trim(),
                        now
                    ],
                )
                .map_err(|e| e.to_string())?;
            if changed == 0 {
                return Err("找不到这次考试".into());
            }
            Ok(id.to_string())
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "insert into local_exam
                    (id, class_id, title, kind, full_score, pass_line, good_line,
                     taken_at, note, created_at, updated_at)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    id,
                    input.class_id,
                    title,
                    kind_of(&input.kind),
                    full,
                    pass,
                    good,
                    taken_at,
                    input.note.trim(),
                    now
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(id)
        }
    }
}

pub fn exam_delete(conn: &Connection, exam_id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "update local_exam set deleted_at = ?2, updated_at = ?2 where id = ?1 and deleted_at is null",
            params![exam_id, text::now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("找不到这次考试".into());
    }
    Ok(())
}

/// 一次登分：按人 upsert。传进来的分数必须在 0..=满分，超了整批不落库。
pub fn exam_scores_save(
    conn: &mut Connection,
    exam_id: &str,
    rows: &[ScoreInput],
) -> Result<i64, String> {
    let exam = exam_get(conn, exam_id)?;
    for row in rows {
        if let Some(score) = row.score {
            if !(0.0..=exam.full_score + 0.001).contains(&score) {
                return Err(format!("分数超出范围：{score}（满分 {}）", exam.full_score));
            }
        }
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let now = text::now();
    let mut saved = 0;
    for row in rows {
        tx.execute(
            "insert into local_exam_score (exam_id, student_id, score, note, updated_at)
             values (?1, ?2, ?3, ?4, ?5)
             on conflict (exam_id, student_id) do update
                set score = excluded.score, note = excluded.note, updated_at = excluded.updated_at",
            params![exam_id, row.student_id, row.score, row.note.trim(), now],
        )
        .map_err(|e| e.to_string())?;
        saved += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(saved)
}

pub fn exam_sheet(conn: &Connection, exam_id: &str) -> Result<ExamSheet, String> {
    let exam = exam_get(conn, exam_id)?;

    let mut stmt = conn
        .prepare(
            "select s.id, s.name, s.student_no, sc.score, coalesce(sc.note, '')
               from local_class_member m
               join local_student s on s.id = m.student_id
               left join local_exam_score sc on sc.exam_id = ?1 and sc.student_id = s.id
              where m.class_id = ?2 and s.deleted_at is null and s.status = 'ACTIVE'
              order by s.student_no, s.name",
        )
        .map_err(|e| e.to_string())?;
    let raw: Vec<(String, String, String, Option<f64>, String)> = stmt
        .query_map(params![exam_id, exam.class_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();

    let stats = stats_of(&raw, &exam);
    let ranks = ranks_of(&raw);
    let rows = raw
        .into_iter()
        .map(|(student_id, name, student_no, score, note)| ScoreCell {
            vs_average: match (score, stats.average) {
                (Some(score), Some(average)) => Some(round1(score - average)),
                _ => None,
            },
            rank: ranks.get(&student_id).copied(),
            student_id,
            student_no,
            name,
            score,
            note,
        })
        .collect();

    Ok(ExamSheet {
        dist: dist_of(conn, exam_id, exam.full_score)?,
        stats,
        rows,
        exam,
    })
}

/// 一个学生的历次考试：分数、班内名次、班均，按时间倒序。
pub fn student_exams(
    conn: &Connection,
    student_id: &str,
    limit: i64,
) -> Result<Vec<StudentExam>, String> {
    let mut stmt = conn
        .prepare(
            "select e.id, e.title, e.kind, e.taken_at, c.name, e.full_score, sc.score
               from local_exam e
               join local_class c on c.id = e.class_id
               left join local_exam_score sc on sc.exam_id = e.id and sc.student_id = ?1
              where e.deleted_at is null
                and exists (select 1 from local_class_member m
                             where m.class_id = e.class_id and m.student_id = ?1)
              order by e.taken_at desc
              limit ?2",
        )
        .map_err(|e| e.to_string())?;
    let base: Vec<(String, String, String, String, String, f64, Option<f64>)> = stmt
        .query_map(params![student_id, limit], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();

    let mut out = Vec::with_capacity(base.len());
    for (exam_id, title, kind, taken_at, class_name, full_score, score) in base {
        let (class_size, class_avg, rank) = class_standing(conn, &exam_id, student_id, score)?;
        out.push(StudentExam {
            exam_id,
            title,
            kind,
            taken_at,
            class_name,
            full_score,
            score,
            rank,
            class_size,
            class_avg,
        });
    }
    Ok(out)
}

/// 一次考试里某个人的处境：有分数的人数、班均、名次（并列同名次）。
fn class_standing(
    conn: &Connection,
    exam_id: &str,
    student_id: &str,
    score: Option<f64>,
) -> Result<(i64, Option<f64>, Option<i64>), String> {
    let mut stmt = conn
        .prepare(
            "select sc.student_id, sc.score
               from local_exam_score sc
               join local_student s on s.id = sc.student_id and s.deleted_at is null
              where sc.exam_id = ?1 and sc.score is not null",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, f64)> = stmt
        .query_map([exam_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    let size = rows.len() as i64;
    if rows.is_empty() {
        return Ok((size, None, None));
    }
    let avg = round1(rows.iter().map(|(_, value)| *value).sum::<f64>() / rows.len() as f64);
    let rank = score.map(|mine| {
        rows.iter()
            .filter(|(_, value)| *value > mine + 0.001)
            .count() as i64
            + 1
    });
    // 自己不在分数表里（缺考）时，名次为空；这里只核对自己这一行是不是真被统计到了。
    let seen = rows.iter().any(|(id, _)| id == student_id);
    Ok((size, Some(avg), if seen { rank } else { None }))
}

const EXAM_SELECT: &str = "select e.id, e.class_id, coalesce(c.name, ''), e.title, e.kind,
        e.full_score, e.pass_line, e.good_line, e.taken_at, e.note,
        (select count(*) from local_exam_score sc
          where sc.exam_id = e.id and sc.score is not null) as entered,
        (select count(*) from local_class_member m
           join local_student s on s.id = m.student_id
          where m.class_id = e.class_id and s.deleted_at is null and s.status = 'ACTIVE') as members,
        (select avg(sc.score) from local_exam_score sc
          where sc.exam_id = e.id and sc.score is not null) as average,
        (select max(sc.score) from local_exam_score sc
          where sc.exam_id = e.id and sc.score is not null) as highest,
        (select min(sc.score) from local_exam_score sc
          where sc.exam_id = e.id and sc.score is not null) as lowest,
        (select count(*) from local_exam_score sc
          where sc.exam_id = e.id and sc.score is not null
            and sc.score >= coalesce(e.pass_line, e.full_score * 0.6)) as pass_count,
        (select count(*) from local_exam_score sc
          where sc.exam_id = e.id and sc.score is not null
            and sc.score >= coalesce(e.good_line, e.full_score * 0.85)) as good_count
   from local_exam e
   left join local_class c on c.id = e.class_id";

fn map_exam(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExamRow> {
    let full: f64 = row.get(5)?;
    let pass_line: Option<f64> = row.get(6)?;
    let good_line: Option<f64> = row.get(7)?;
    let entered: i64 = row.get(10)?;
    let average: Option<f64> = row.get(12)?;
    let pass_count: i64 = row.get(15)?;
    let good_count: i64 = row.get(16)?;
    let rate = |count: i64| {
        if entered > 0 {
            Some(round1(count as f64 / entered as f64 * 100.0))
        } else {
            None
        }
    };
    Ok(ExamRow {
        id: row.get(0)?,
        class_id: row.get(1)?,
        class_name: row.get(2)?,
        title: row.get(3)?,
        kind: row.get(4)?,
        full_score: full,
        pass_line: pass_of(full, pass_line),
        good_line: good_of(full, good_line),
        taken_at: row.get(8)?,
        note: row.get(9)?,
        entered,
        student_count: row.get(11)?,
        average: average.map(round1),
        highest: row.get(13)?,
        lowest: row.get(14)?,
        pass_rate: rate(pass_count),
        good_rate: rate(good_count),
    })
}

/// 统计只看有分数的人：缺考不拉低平均，也不占名次。
fn stats_of(raw: &[(String, String, String, Option<f64>, String)], exam: &ExamRow) -> ExamStats {
    let mut scores: Vec<f64> = raw
        .iter()
        .filter_map(|(_, _, _, score, _)| *score)
        .collect();
    scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let entered = scores.len() as i64;
    let missing = raw.len() as i64 - entered;
    if scores.is_empty() {
        return ExamStats {
            entered: 0,
            missing,
            average: None,
            highest: None,
            lowest: None,
            median: None,
            pass: 0,
            good: 0,
            pass_rate: None,
            good_rate: None,
        };
    }
    let sum: f64 = scores.iter().sum();
    let median = if scores.len() % 2 == 0 {
        (scores[scores.len() / 2 - 1] + scores[scores.len() / 2]) / 2.0
    } else {
        scores[scores.len() / 2]
    };
    let pass = scores
        .iter()
        .filter(|value| **value >= exam.pass_line)
        .count() as i64;
    let good = scores
        .iter()
        .filter(|value| **value >= exam.good_line)
        .count() as i64;
    ExamStats {
        entered,
        missing,
        average: Some(round1(sum / entered as f64)),
        highest: scores.last().copied(),
        lowest: scores.first().copied(),
        median: Some(round1(median)),
        pass,
        good,
        pass_rate: Some(round1(pass as f64 / entered as f64 * 100.0)),
        good_rate: Some(round1(good as f64 / entered as f64 * 100.0)),
    }
}

/// 名次：并列同名次（1,2,2,4），缺考没有名次。
fn ranks_of(raw: &[(String, String, String, Option<f64>, String)]) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    for (student_id, _, _, score, _) in raw {
        let Some(mine) = score else { continue };
        let ahead = raw
            .iter()
            .filter(|(_, _, _, other, _)| other.map(|value| value > *mine + 0.001).unwrap_or(false))
            .count() as i64;
        out.insert(student_id.clone(), ahead + 1);
    }
    out
}

/// 分数段：按满分百分比切五档，换 100 / 150 分制都一样看。
fn dist_of(conn: &Connection, exam_id: &str, full: f64) -> Result<Vec<DistBucket>, String> {
    let mut stmt = conn
        .prepare(
            "select score from local_exam_score
              where exam_id = ?1 and score is not null",
        )
        .map_err(|e| e.to_string())?;
    let scores: Vec<f64> = stmt
        .query_map([exam_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    let bands = [
        (0.0, 60.0, "不及格"),
        (60.0, 70.0, "60–69"),
        (70.0, 80.0, "70–79"),
        (80.0, 90.0, "80–89"),
        (90.0, 100.1, "90+"),
    ];
    Ok(bands
        .iter()
        .map(|(from, to, label)| {
            let count = scores
                .iter()
                .filter(|score| {
                    let pct = **score / full * 100.0;
                    pct >= *from && pct < *to
                })
                .count() as i64;
            DistBucket {
                label: (*label).to_string(),
                from: *from,
                to: *to,
                count,
            }
        })
        .collect())
}
