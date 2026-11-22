//! 讲评清单：把一份作业的批改结果按**题**汇总，给老师上课讲评用。
//!
//! 起因：离线端的批改备注（`local_attempt.remark`）以前只写不读，是个黑洞。
//! 但老师批完 30 份卷子真正想要的不是「再做一个字段」，而是——
//! **这道题几个人错、错在哪、哪些人**。所以清单以题为主体，学生名单是附属信息。
//!
//! 口径说明（与批改台、学情保持一致）：
//! - 只看每个学生**最近一次**提交（`superseded_at is null` 里 `submitted_at` 最大的那个
//!   `submission_id`），重做过的旧卷子不混进来；
//! - `correct = 0` 或 `score` 未满都算错（半分题按「没拿满」算，并在 `partial` 里单独计数）；
//! - `remark` 只统计这一份作业里的，不外延到别的作业。

use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRemark {
    pub student_id: String,
    pub student_name: String,
    pub score: f64,
    pub max_score: f64,
    pub remark: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewQuestion {
    pub question_id: String,
    pub ord: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub type_label: String,
    pub difficulty: i64,
    pub stem: String,
    pub excerpt: String,
    pub answer: String,
    pub score: f64,
    pub knowledge: Vec<String>,
    /// 有作答记录的人数。
    pub attempted: i64,
    /// 判为错的人数（未作答也算错；半分题只要没拿满就算）。
    pub wrong: i64,
    /// 其中「错但拿了部分分」的人数。
    pub partial: i64,
    /// 空答人数。
    pub blank: i64,
    /// 还没批的人数（`correct` 与 `score` 都为空）。
    pub pending: i64,
    /// 已判分的平均得分（只算已批的）。
    pub avg_score: f64,
    /// 得分率 0–1：`avg_score / 本题满分`，没批过时为 null。
    pub rate: Option<f64>,
    /// 讲评优先级：1 = 最该先讲（错得最多，同错数看得分率最低）。
    pub rank: i64,
    pub wrong_students: Vec<String>,
    pub remarks: Vec<ReviewRemark>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewStudent {
    pub id: String,
    pub name: String,
    pub submitted: bool,
    pub late: bool,
    pub score: f64,
    pub max_score: f64,
    pub pending: i64,
}

/// 一份作业的讲评清单。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSheet {
    pub assignment_id: String,
    pub title: String,
    pub class_id: String,
    pub class_name: String,
    pub subject_name: String,
    pub due_at: Option<String>,
    pub total_score: f64,
    /// 批改完成度：已批题数 / 应交题数。
    pub graded_items: i64,
    pub expected_items: i64,
    pub student_count: i64,
    pub submitted_count: i64,
    pub avg_score: f64,
    pub max_score: f64,
    pub generated_at: String,
    pub questions: Vec<ReviewQuestion>,
    pub students: Vec<ReviewStudent>,
}

struct AttemptRow {
    question_id: String,
    student_id: String,
    student_name: String,
    answer: String,
    correct: Option<i64>,
    score: Option<f64>,
    max_score: f64,
    remark: String,
    late: bool,
}

pub fn sheet(conn: &Connection, assignment_id: &str) -> Result<ReviewSheet, String> {
    let (title, class_id, class_name, subject_name, due_at): (
        String,
        String,
        String,
        String,
        Option<String>,
    ) = conn
        .query_row(
            "select a.title, c.id, c.name, ifnull(s.name, ''), a.due_at
             from local_assignment a
             join local_class c on c.id = a.class_id
             left join subject s on s.id = c.subject_id
             where a.id = ?1",
            [assignment_id],
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
        .map_err(|_| "这份作业找不到了".to_string())?;

    // 题目：作业里的顺序与分值。
    struct Item {
        question_id: String,
        ord: i64,
        kind: String,
        difficulty: i64,
        stem: String,
        answer: String,
        score: f64,
        knowledge: Vec<String>,
    }
    let mut items: Vec<Item> = {
        let mut stmt = conn
            .prepare(
                "select aq.question_id, aq.ord, q.type, q.difficulty, q.stem, q.answer, aq.score
                 from local_assignment_question aq join question q on q.id = aq.question_id
                 where aq.assignment_id = ?1 order by aq.ord",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([assignment_id], |row| {
                Ok(Item {
                    question_id: row.get(0)?,
                    ord: row.get(1)?,
                    kind: row.get(2)?,
                    difficulty: row.get(3)?,
                    stem: row.get(4)?,
                    answer: row.get(5)?,
                    score: row.get(6)?,
                    knowledge: Vec::new(),
                })
            })
            .map_err(|e| e.to_string())?;
        rows.filter_map(Result::ok).collect()
    };
    {
        let mut kp = conn
            .prepare(
                "select kp.name from question_knowledge qk
                 join knowledge_point kp on kp.id = qk.knowledge_point_id
                 where qk.question_id = ?1 order by kp.sort, kp.name limit 3",
            )
            .map_err(|e| e.to_string())?;
        for item in &mut items {
            let names = kp
                .query_map([&item.question_id], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            item.knowledge = names.filter_map(Result::ok).collect();
        }
    }

    // 作答：每个学生只取最近一次提交。
    let attempts: Vec<AttemptRow> = {
        let mut stmt = conn
            .prepare(
                "select a.question_id, s.id, s.name, a.answer, a.correct, a.score, a.max_score,
                        a.remark, a.late
                 from local_attempt a join local_student s on s.id = a.student_id
                 where a.assignment_id = ?1 and a.superseded_at is null
                   and a.submission_id = (
                       select b.submission_id from local_attempt b
                       where b.assignment_id = a.assignment_id and b.student_id = a.student_id
                         and b.superseded_at is null
                       order by b.submitted_at desc limit 1)
                 order by s.name",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([assignment_id], |row| {
                Ok(AttemptRow {
                    question_id: row.get(0)?,
                    student_id: row.get(1)?,
                    student_name: row.get(2)?,
                    answer: row.get(3)?,
                    correct: row.get(4)?,
                    score: row.get(5)?,
                    max_score: row.get(6)?,
                    remark: row.get(7)?,
                    late: row.get::<_, Option<i64>>(8)?.unwrap_or(0) != 0,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.filter_map(Result::ok).collect()
    };

    let mut questions: Vec<ReviewQuestion> = Vec::new();
    for item in &items {
        let mut wrong = 0;
        let mut partial = 0;
        let mut blank = 0;
        let mut pending = 0;
        let mut graded = 0;
        let mut score_sum = 0.0;
        let mut wrong_students = Vec::new();
        let mut remarks = Vec::new();

        for row in attempts
            .iter()
            .filter(|row| row.question_id == item.question_id)
        {
            if row.answer.trim().is_empty() && row.correct.is_none() && row.score.is_none() {
                blank += 1;
            }
            match (row.correct, row.score) {
                (None, None) => {
                    pending += 1;
                    continue;
                }
                _ => {
                    graded += 1;
                    score_sum += row.score.unwrap_or(0.0);
                    let full = row.max_score.max(item.score);
                    let failed = row.correct == Some(0)
                        || row.score.map(|value| value + 1e-9 < full).unwrap_or(false);
                    if failed {
                        wrong += 1;
                        if row.score.unwrap_or(0.0) > 0.0 {
                            partial += 1;
                        }
                        wrong_students.push(row.student_name.clone());
                    }
                }
            }
            if !row.remark.trim().is_empty() {
                remarks.push(ReviewRemark {
                    student_id: row.student_id.clone(),
                    student_name: row.student_name.clone(),
                    score: row.score.unwrap_or(0.0),
                    max_score: row.max_score.max(item.score),
                    remark: row.remark.trim().to_string(),
                });
            }
        }

        let attempted = attempts
            .iter()
            .filter(|row| row.question_id == item.question_id)
            .count() as i64;
        let avg_score = if graded > 0 {
            score_sum / graded as f64
        } else {
            0.0
        };
        let rate = if graded > 0 && item.score > 0.0 {
            Some((avg_score / item.score).clamp(0.0, 1.0))
        } else {
            None
        };

        questions.push(ReviewQuestion {
            question_id: item.question_id.clone(),
            ord: item.ord,
            kind: item.kind.clone(),
            type_label: crate::text::type_label(&item.kind).to_string(),
            difficulty: item.difficulty,
            excerpt: crate::text::excerpt(&item.stem, 120),
            stem: item.stem.clone(),
            answer: item.answer.clone(),
            score: item.score,
            knowledge: item.knowledge.clone(),
            attempted,
            wrong,
            partial,
            blank,
            pending,
            avg_score,
            rate,
            rank: 0,
            wrong_students,
            remarks,
        });
    }

    // 讲评优先级放在后端算：前端排序的话，打印稿与界面迟早会给出两种「最该讲的题」。
    let mut order: Vec<usize> = (0..questions.len()).collect();
    order.sort_by(|left, right| {
        let a = &questions[*left];
        let b = &questions[*right];
        b.wrong
            .cmp(&a.wrong)
            .then_with(|| {
                a.rate
                    .unwrap_or(1.0)
                    .partial_cmp(&b.rate.unwrap_or(1.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.ord.cmp(&b.ord))
    });
    for (index, position) in order.iter().enumerate() {
        questions[*position].rank = index as i64 + 1;
    }

    // 学生：交没交、多少分、还有几题待批。
    let mut students: Vec<ReviewStudent> = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "select s.id, s.name from local_class_member m join local_student s on s.id = m.student_id
                 where m.class_id = ?1 and s.deleted_at is null order by s.name",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&class_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        for (id, name) in rows.flatten() {
            let mine: Vec<&AttemptRow> =
                attempts.iter().filter(|row| row.student_id == id).collect();
            let submitted = !mine.is_empty();
            let score: f64 = mine.iter().map(|row| row.score.unwrap_or(0.0)).sum();
            let max_score: f64 = mine.iter().map(|row| row.max_score).sum();
            let pending = mine
                .iter()
                .filter(|row| row.correct.is_none() && row.score.is_none())
                .count() as i64;
            students.push(ReviewStudent {
                id,
                name,
                submitted,
                late: mine.iter().any(|row| row.late),
                score,
                max_score,
                pending,
            });
        }
    }

    let total_score: f64 = items.iter().map(|item| item.score).sum();
    let expected_items = items.len() as i64 * students.len() as i64;
    let graded_items = attempts
        .iter()
        .filter(|row| row.correct.is_some() || row.score.is_some())
        .count() as i64;
    let submitted_count = students.iter().filter(|row| row.submitted).count() as i64;
    let scored: Vec<&ReviewStudent> = students
        .iter()
        .filter(|row| row.submitted && row.pending == 0 && row.max_score > 0.0)
        .collect();
    let avg_score = if scored.is_empty() {
        0.0
    } else {
        scored.iter().map(|row| row.score).sum::<f64>() / scored.len() as f64
    };
    let max_score = scored
        .first()
        .map(|row| row.max_score)
        .unwrap_or(total_score);

    Ok(ReviewSheet {
        assignment_id: assignment_id.to_string(),
        title,
        class_id,
        class_name,
        subject_name,
        due_at,
        total_score,
        graded_items,
        expected_items,
        student_count: students.len() as i64,
        submitted_count,
        avg_score,
        max_score,
        generated_at: crate::text::now(),
        questions,
        students,
    })
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

    fn seed(conn: &Connection) {
        conn.execute("insert into subject (id, name) values ('sub1', '数学')", [])
            .unwrap();
        conn.execute(
            "insert into local_class (id, name, subject_id, join_code, created_at, updated_at)
             values ('cls1', '初二 3 班', 'sub1', 'ABC234', '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();
        for (id, name) in [("s1", "张三"), ("s2", "李四")] {
            conn.execute(
                "insert into local_student (id, name, created_at, updated_at)
                 values (?1, ?2, '2026-01-01', '2026-01-01')",
                params![id, name],
            )
            .unwrap();
            conn.execute(
                "insert into local_class_member (class_id, student_id, joined_at)
                 values ('cls1', ?1, '2026-01-01')",
                params![id],
            )
            .unwrap();
        }
        conn.execute(
            "insert into local_assignment (id, class_id, title, type, status, created_at, updated_at)
             values ('a1', 'cls1', '第二章练习', 'PRACTICE', 'PUBLISHED', '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();
        for (id, ord, score) in [("q1", 0, 5.0), ("q2", 1, 10.0)] {
            conn.execute(
                "insert into question (id, subject_id, type, stem, answer, difficulty, status)
                 values (?1, 'sub1', 'SOLVED', '题干', '答案', 3, 'PUBLISHED')",
                params![id],
            )
            .unwrap();
            conn.execute(
                "insert into local_assignment_question (assignment_id, question_id, ord, score)
                 values ('a1', ?1, ?2, ?3)",
                params![id, ord, score],
            )
            .unwrap();
        }
        // 张三：两题都错，第一题留了备注。
        conn.execute(
            "insert into local_attempt (id, assignment_id, question_id, student_id, submission_id,
                answer, submitted_at, correct, score, max_score, remark)
             values ('t1', 'a1', 'q1', 's1', 'sub1', '1', '2026-01-02', 0, 0, 5, '去括号漏了负号')",
            [],
        )
        .unwrap();
        conn.execute(
            "insert into local_attempt (id, assignment_id, question_id, student_id, submission_id,
                answer, submitted_at, correct, score, max_score, remark)
             values ('t2', 'a1', 'q2', 's1', 'sub1', '2', '2026-01-02', 1, 10, 10, '')",
            [],
        )
        .unwrap();
        // 李四：第一题半分，第二题待批。
        conn.execute(
            "insert into local_attempt (id, assignment_id, question_id, student_id, submission_id,
                answer, submitted_at, correct, score, max_score, remark)
             values ('t3', 'a1', 'q1', 's2', 'sub2', 'x', '2026-01-02', 0, 2.5, 5, '步骤对，结果错')",
            [],
        )
        .unwrap();
        conn.execute(
            "insert into local_attempt (id, assignment_id, question_id, student_id, submission_id,
                answer, submitted_at, correct, score, max_score, remark)
             values ('t4', 'a1', 'q2', 's2', 'sub2', '', '2026-01-02', null, null, 10, '')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn sheet_groups_by_question_and_keeps_remarks() {
        let conn = setup();
        seed(&conn);
        let out = sheet(&conn, "a1").unwrap();
        assert_eq!(out.questions.len(), 2);
        assert_eq!(out.student_count, 2);
        assert_eq!(out.submitted_count, 2);
        assert_eq!(out.total_score, 15.0);

        let first = &out.questions[0];
        assert_eq!(first.wrong, 2);
        assert_eq!(first.partial, 1);
        assert_eq!(first.remarks.len(), 2);
        assert_eq!(first.remarks[0].student_name, "张三");
        assert!((first.avg_score - 1.25).abs() < 1e-9);
        assert!((first.rate.unwrap() - 0.25).abs() < 1e-9);

        let second = &out.questions[1];
        assert_eq!(second.pending, 1);
        assert_eq!(second.blank, 1);
        assert_eq!(second.wrong, 0);
    }

    #[test]
    fn priority_puts_most_wrong_first() {
        let conn = setup();
        seed(&conn);
        let out = sheet(&conn, "a1").unwrap();
        assert_eq!(out.questions[0].rank, 1);
        assert_eq!(out.questions[1].rank, 2);
    }

    #[test]
    fn half_score_counts_as_wrong_but_not_blank() {
        let conn = setup();
        seed(&conn);
        let out = sheet(&conn, "a1").unwrap();
        let first = &out.questions[0];
        assert_eq!(first.blank, 0);
        assert_eq!(
            first.wrong_students,
            vec!["张三".to_string(), "李四".to_string()]
        );
    }
}
