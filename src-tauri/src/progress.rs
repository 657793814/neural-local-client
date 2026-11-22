//! 作业与考试合并到一条时间轴。
//!
//! 两张表的口径本来不同：作业是**按题判分**后汇总出来的得分率，考试是**整卷总分**。
//! 直接把分数放在一起看没有意义（100 分卷和 150 分卷、10 分小练和 100 分月考），
//! 所以这里统一换算成**百分制**（得分 / 满分 × 100），再按日期混排。
//!
//! 两点刻意保留：
//! - 作业还有题没批时，得分率会被算低，单独带 `pending` 让界面标「部分待批」；
//! - 缺考（考试分数为 null）不进时间轴——空点会把趋势线拉断，也会被误读成 0 分。

use crate::scores;
use rusqlite::{params, Connection};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPoint {
    /// `YYYY-MM-DD`；作业取最后一次提交日期，考试取考试日期。
    pub at: String,
    /// ASSIGNMENT / EXAM / QUIZ / UNIT
    pub kind: String,
    pub title: String,
    pub score: f64,
    pub full: f64,
    /// 百分制（0–100），一位小数。
    pub percent: f64,
    pub rank: Option<i64>,
    pub class_size: Option<i64>,
    /// 同一次考试的班均，已换成百分制。
    pub class_avg: Option<f64>,
    /// 作业里还有几题没批：百分比会偏低，界面上要标出来。
    pub pending: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// 时间正序，最新在最后。
    pub points: Vec<ProgressPoint>,
    pub assignments: i64,
    pub exams: i64,
    pub assignment_avg: Option<f64>,
    pub exam_avg: Option<f64>,
    /// 最近三次与之前三次的平均差（百分点），正数是进步。
    pub delta: Option<f64>,
    /// 一句话结论，报告和卡片直接引用。
    pub note: String,
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn percent(score: f64, full: f64) -> Option<f64> {
    if full > 0.0 {
        Some(round1(score / full * 100.0))
    } else {
        None
    }
}

/// 作业：每个作业取最新一次提交的汇总（重交的旧版本已由 `superseded_at` 排除）。
fn assignment_points(conn: &Connection, student_id: &str, limit: i64) -> Vec<ProgressPoint> {
    let mut stmt = match conn.prepare(
        "select asg.id, asg.title, max(a.submitted_at) as at,
                coalesce(sum(a.score), 0) as score,
                coalesce(sum(a.max_score), 0) as full,
                sum(case when a.correct is null then 1 else 0 end) as pending
           from local_attempt a
           join local_assignment asg on asg.id = a.assignment_id and asg.deleted_at is null
          where a.student_id = ?1 and a.superseded_at is null
          group by asg.id
          order by at desc
          limit ?2",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let rows: Vec<(String, String, String, f64, f64, i64)> = stmt
        .query_map(params![student_id, limit], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();

    rows.into_iter()
        .filter_map(|(_, title, at, score, full, pending)| {
            let pct = percent(score, full)?;
            Some(ProgressPoint {
                at: at.get(..10).unwrap_or(&at).to_string(),
                kind: "ASSIGNMENT".into(),
                title,
                score,
                full,
                percent: pct.min(100.0),
                rank: None,
                class_size: None,
                class_avg: None,
                pending,
            })
        })
        .collect()
}

/// 考试：只取有分的；缺考不进时间轴（与「缺考不进平均」同一口径）。
fn exam_points(conn: &Connection, student_id: &str, limit: i64) -> Vec<ProgressPoint> {
    let exams = scores::student_exams(conn, student_id, limit).unwrap_or_default();
    exams
        .into_iter()
        .filter_map(|exam| {
            let score = exam.score?;
            let pct = percent(score, exam.full_score)?;
            Some(ProgressPoint {
                at: exam
                    .taken_at
                    .get(..10)
                    .unwrap_or(&exam.taken_at)
                    .to_string(),
                kind: if exam.kind.is_empty() {
                    "EXAM".into()
                } else {
                    exam.kind
                },
                title: exam.title,
                score,
                full: exam.full_score,
                percent: pct,
                rank: exam.rank,
                class_size: Some(exam.class_size),
                class_avg: exam.class_avg.and_then(|avg| percent(avg, exam.full_score)),
                pending: 0,
            })
        })
        .collect()
}

fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(round1(values.iter().sum::<f64>() / values.len() as f64))
    }
}

pub fn student_progress(
    conn: &Connection,
    student_id: &str,
    limit: i64,
) -> Result<Progress, String> {
    let limit = limit.clamp(3, 60);
    let mut points = assignment_points(conn, student_id, limit);
    points.extend(exam_points(conn, student_id, limit));
    // 同一天里作业在前、考试在后：先看练得怎么样，再看考得怎么样。
    points.sort_by(|a, b| {
        a.at.cmp(&b.at)
            .then(kind_weight(&a.kind).cmp(&kind_weight(&b.kind)))
            .then(a.title.cmp(&b.title))
    });
    // 两类各查 limit 条，合起来最多 2×limit；这里再截一次，前端拿到的就是「最近 N 次」。
    if points.len() > limit as usize {
        points = points.split_off(points.len() - limit as usize);
    }

    let assignment_avg = mean(
        &points
            .iter()
            .filter(|point| point.kind == "ASSIGNMENT")
            .map(|point| point.percent)
            .collect::<Vec<_>>(),
    );
    let exam_avg = mean(
        &points
            .iter()
            .filter(|point| point.kind != "ASSIGNMENT")
            .map(|point| point.percent)
            .collect::<Vec<_>>(),
    );
    let assignments = points
        .iter()
        .filter(|point| point.kind == "ASSIGNMENT")
        .count() as i64;
    let exams = points.len() as i64 - assignments;

    let delta = delta_of(&points);
    let note = note_of(&points, delta, assignments, exams);

    Ok(Progress {
        points,
        assignments,
        exams,
        assignment_avg,
        exam_avg,
        delta,
        note,
    })
}

/// 作业排在考试前面（同一天）。
fn kind_weight(kind: &str) -> u8 {
    if kind == "ASSIGNMENT" {
        0
    } else {
        1
    }
}

/// 最近三次 vs 之前三次：样本不够就不下结论。
fn delta_of(points: &[ProgressPoint]) -> Option<f64> {
    if points.len() < 4 {
        return None;
    }
    let values: Vec<f64> = points.iter().map(|point| point.percent).collect();
    let recent: Vec<f64> = values.iter().rev().take(3).copied().collect();
    let earlier: Vec<f64> = values.iter().rev().skip(3).take(3).copied().collect();
    Some(round1(mean(&recent)? - mean(&earlier)?))
}

fn note_of(points: &[ProgressPoint], delta: Option<f64>, assignments: i64, exams: i64) -> String {
    if points.is_empty() {
        return "还没有可比较的分数：交一次作业或录一次考试，趋势就有了。".into();
    }
    let mut parts = Vec::new();
    if assignments > 0 || exams > 0 {
        parts.push(format!("共 {assignments} 次作业、{exams} 次考试"));
    }
    match delta {
        Some(value) if value >= 5.0 => parts.push(format!("最近三次比之前高 {value} 个百分点")),
        Some(value) if value <= -5.0 => parts.push(format!("最近三次比之前低 {} 个百分点", -value)),
        Some(_) => parts.push("最近几次基本持平".into()),
        None => parts.push("次数还少，先看单次".into()),
    }
    if points.iter().any(|point| point.pending > 0) {
        parts.push("部分作业还有题待批，得分率会偏低".into());
    }
    parts.join("；")
}
