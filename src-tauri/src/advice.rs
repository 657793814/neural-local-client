//! 学情诊断与学习安排。
//!
//! 两层：
//! 1. **规则版**（永远可用，不依赖模型）：把掌握度、薄弱知识点、到期错题、最近考试
//!    翻译成「先补什么、这周每天做什么、大概多少分钟」。口径与学情页完全一致。
//! 2. **AI 版**（可选）：把同一份快照交给模型，让它在规则之上补「为什么」和
//!    「怎么讲给孩子 / 家长听」，输出仍然是同一套结构，前端不用分叉。
//!
//! 硬约束：
//! - 模型只拿结构化的学情数字，不拿姓名以外的隐私，也不拿题目原文；
//! - 模型输出必须过结构校验，不合格的字段一律退回规则版，绝不把半截话给孩子看；
//! - 报告按「数据指纹」缓存，数据没变就不重复花钱。

use crate::insights;
use crate::llm;
use crate::scores;
use crate::text;
use chrono::{DateTime, Local};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// 薄弱知识点线：与「按薄弱点出题」同口径。
const WEAK_LINE: i32 = 70;
/// 已掌握线：与学情页一致。
const MASTERED_LINE: i32 = 85;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdviceStrength {
    pub point: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdviceIssue {
    /// 知识点名；不针对具体知识点时可以给一句概括。
    pub kp: String,
    pub why: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDay {
    pub day: String,
    pub task: String,
    pub minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvicePlan {
    pub daily_minutes: i64,
    pub total_minutes: i64,
    /// 这一周集中在哪几个知识点上。
    pub focus: Vec<String>,
    pub days: Vec<PlanDay>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Advice {
    pub student_id: String,
    /// EMPTY / CRITICAL / WATCH / STEADY / SOLID
    pub level: String,
    pub level_label: String,
    pub headline: String,
    pub strengths: Vec<AdviceStrength>,
    pub issues: Vec<AdviceIssue>,
    pub plan: AdvicePlan,
    pub parent_note: String,
    /// AI / RULE
    pub source: String,
    pub created_at: String,
    /// 生成之后数据又变了：结论还能看，但建议刷新。
    pub stale: bool,
    /// 模型看到的依据，界面上折起来展示，避免「黑箱给结论」。
    pub facts: Vec<String>,
}

// ---------------------------------------------------------------------------
// 事实快照
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ExamFact {
    title: String,
    score: f64,
    full: f64,
    rank: Option<i64>,
    size: i64,
    avg: Option<f64>,
    taken_at: String,
}

#[derive(Debug, Clone)]
struct Facts {
    student_id: String,
    name: String,
    grade: String,
    classes: Vec<String>,
    has_data: bool,
    overall: i32,
    answered: i64,
    correct: i64,
    pending: i64,
    score_rate: f64,
    delta: i32,
    rank: Option<i64>,
    class_size: Option<i64>,
    class_avg: Option<i32>,
    /// (知识点名, 掌握度, 已判题数)
    weak: Vec<(String, i32, i64)>,
    strong: Vec<(String, i32, i64)>,
    wrong_count: i64,
    due_review: i64,
    days_idle: Option<i64>,
    exams: Vec<ExamFact>,
    /// (作业名, 得分, 满分, 待批数, 提交时间)
    recent: Vec<(String, f64, f64, i64, Option<String>)>,
    /// 错题本按知识点汇总：(知识点, 题数, 最近一次做错时间)
    wrong_by_kp: Vec<(String, i64, String)>,
}

impl Facts {
    fn idle_text(&self) -> String {
        match self.days_idle {
            None => "还没交过作业".into(),
            Some(0) => "今天练过".into(),
            Some(1) => "昨天练过".into(),
            Some(days) => format!("{days} 天没练"),
        }
    }

    /// 给模型看的紧凑事实（也是 fingerprint 的输入）。
    fn json(&self) -> Value {
        json!({
            "name": self.name,
            "grade": self.grade,
            "classes": self.classes,
            "mastery": if self.has_data { json!(self.overall) } else { Value::Null },
            "answered": self.answered,
            "correct": self.correct,
            "pending": self.pending,
            "scoreRate": (self.score_rate * 100.0).round() / 100.0,
            "delta2w": self.delta,
            "rank": self.rank,
            "classSize": self.class_size,
            "classAvg": self.class_avg,
            "weak": self.weak.iter().map(|(name, mastery, answered)| json!({
                "kp": name, "mastery": mastery, "answered": answered
            })).collect::<Vec<_>>(),
            "strong": self.strong.iter().map(|(name, mastery, _)| json!({
                "kp": name, "mastery": mastery
            })).collect::<Vec<_>>(),
            "wrongBook": self.wrong_count,
            "wrongByKp": self.wrong_by_kp.iter().map(|(kp, count, last)| json!({
                "kp": kp, "count": count, "lastWrongAt": last
            })).collect::<Vec<_>>(),
            "dueReview": self.due_review,
            "idleDays": self.days_idle,
            "exams": self.exams.iter().map(|exam| json!({
                "title": exam.title, "score": exam.score, "full": exam.full,
                "rank": exam.rank, "classSize": exam.size, "classAvg": exam.avg,
                "date": exam.taken_at
            })).collect::<Vec<_>>(),
            "recentAssignments": self.recent.iter().map(|(title, score, max, pending, at)| json!({
                "title": title, "score": score, "max": max, "pending": pending, "at": at
            })).collect::<Vec<_>>(),
        })
    }

    fn fact_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.has_data {
            out.push(format!(
                "掌握度 {}（已判 {} 题、对 {} 题，得分率 {}%）",
                self.overall,
                self.answered,
                self.correct,
                (self.score_rate * 100.0).round() as i64
            ));
            out.push(format!(
                "近两周{}",
                if self.delta > 0 {
                    format!("进步 {} 分", self.delta)
                } else if self.delta < 0 {
                    format!("退步 {} 分", -self.delta)
                } else {
                    "持平".to_string()
                }
            ));
        } else {
            out.push("还没有已判分的作答".into());
        }
        if let (Some(rank), Some(size)) = (self.rank, self.class_size) {
            out.push(format!("班内第 {rank} / {size}"));
        }
        if let Some(avg) = self.class_avg {
            out.push(format!("班级平均 {avg}"));
        }
        if !self.weak.is_empty() {
            out.push(format!(
                "薄弱知识点：{}",
                self.weak
                    .iter()
                    .map(|(name, mastery, _)| format!("{name}（{mastery}）"))
                    .collect::<Vec<_>>()
                    .join("、")
            ));
        }
        if self.wrong_count > 0 {
            out.push(format!(
                "错题本 {} 道，其中 {} 道到期该复习",
                self.wrong_count, self.due_review
            ));
        }
        if let Some(exam) = self.exams.first() {
            let mut line = format!(
                "最近考试 {}：{} / {}",
                exam.title,
                round1(exam.score),
                round1(exam.full)
            );
            if let (Some(rank), size) = (exam.rank, exam.size) {
                line.push_str(&format!("，班内第 {rank}/{size}"));
            }
            if let Some(avg) = exam.avg {
                line.push_str(&format!("（班均 {avg}）"));
            }
            out.push(line);
        }
        out.push(self.idle_text());
        if self.pending > 0 {
            out.push(format!("{} 题待批，暂不计入掌握度", self.pending));
        }
        out
    }
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn days_since(raw: &str) -> Option<i64> {
    let at = DateTime::parse_from_rfc3339(raw).ok()?;
    let days = (Local::now() - at.with_timezone(&Local)).num_days();
    Some(days.max(0))
}

fn facts_of(conn: &Connection, student_id: &str) -> Result<Facts, String> {
    let detail = insights::student_insight(conn, student_id)?;
    let due = insights::due_review(conn, Some(student_id)).unwrap_or(0);
    let exams = scores::student_exams(conn, student_id, 6)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|exam| {
            exam.score.map(|score| ExamFact {
                title: exam.title,
                score,
                full: exam.full_score,
                rank: exam.rank,
                size: exam.class_size,
                avg: exam.class_avg,
                taken_at: exam.taken_at,
            })
        })
        .collect::<Vec<_>>();

    let mut weak: Vec<(String, i32, i64)> = detail
        .kps
        .iter()
        .filter(|kp| kp.answered > 0 && kp.mastery < WEAK_LINE)
        .map(|kp| (kp.name.clone(), kp.mastery, kp.answered))
        .collect();
    weak.sort_by(|a, b| a.1.cmp(&b.1));
    weak.truncate(3);

    let mut strong: Vec<(String, i32, i64)> = detail
        .kps
        .iter()
        .filter(|kp| kp.answered > 0 && kp.mastery >= MASTERED_LINE)
        .map(|kp| (kp.name.clone(), kp.mastery, kp.answered))
        .collect();
    strong.sort_by(|a, b| b.1.cmp(&a.1));
    strong.truncate(2);

    // 错题本按知识点归堆：让「为什么薄弱」有具体依据，而不是一句「做错的题集中在这里」。
    let mut wrong_map: HashMap<String, (i64, String)> = HashMap::new();
    for row in insights::wrong_list(conn, student_id, None).unwrap_or_default() {
        for kp in &row.knowledge {
            let entry = wrong_map
                .entry(kp.clone())
                .or_insert((0, row.last_wrong_at.clone()));
            entry.0 += 1;
            if row.last_wrong_at > entry.1 {
                entry.1 = row.last_wrong_at.clone();
            }
        }
    }
    let mut wrong_by_kp: Vec<(String, i64, String)> = wrong_map
        .into_iter()
        .map(|(kp, (count, last))| (kp, count, last))
        .collect();
    wrong_by_kp.sort_by(|a, b| b.1.cmp(&a.1));

    Ok(Facts {
        student_id: detail.student_id.clone(),
        name: detail.name.clone(),
        grade: detail.grade.clone(),
        classes: detail.class_names.clone(),
        has_data: detail.answered > 0,
        overall: detail.overall,
        answered: detail.answered,
        correct: detail.correct,
        pending: detail.pending,
        score_rate: detail.score_rate,
        delta: detail.delta,
        rank: detail.rank,
        class_size: detail.class_size,
        class_avg: detail.class_avg,
        weak,
        strong,
        wrong_count: detail.wrong_count,
        due_review: due,
        days_idle: detail
            .recent
            .iter()
            .filter_map(|row| row.submitted_at.as_deref())
            .filter_map(days_since)
            .min(),
        exams,
        recent: detail
            .recent
            .iter()
            .take(6)
            .map(|row| {
                (
                    row.title.clone(),
                    row.score,
                    row.max_score,
                    row.pending,
                    row.submitted_at.clone(),
                )
            })
            .collect(),
        wrong_by_kp,
    })
}

// ---------------------------------------------------------------------------
// 规则版：不联网也能给出「这周做什么」
// ---------------------------------------------------------------------------

fn level_of(facts: &Facts) -> (&'static str, &'static str) {
    if !facts.has_data {
        return ("EMPTY", "待启动");
    }
    if facts.overall < 40 {
        return ("CRITICAL", "重点关注");
    }
    if facts.delta <= -6 {
        return ("WATCH", "需要止损");
    }
    if facts.overall >= MASTERED_LINE {
        return ("SOLID", "稳固");
    }
    ("STEADY", "稳步提升")
}

fn headline_of(facts: &Facts) -> String {
    if !facts.has_data {
        return "还没有作答数据：先交一次作业，才有可分析的材料".into();
    }
    if facts.delta <= -6 {
        return format!(
            "近两周掉了 {} 分，先止损：把错题重做一遍，再谈新内容",
            -facts.delta
        );
    }
    match facts.weak.len() {
        0 if facts.overall >= MASTERED_LINE => {
            "基础扎实，本周重点是把已掌握的点保住，并做一点拔高".into()
        }
        0 => format!("{} 分，没有明显薄弱点，保持节奏、适度加难度", facts.overall),
        1 => format!(
            "{} 分，「{}」是当前唯一明显短板，集中一周补掉",
            facts.overall, facts.weak[0].0
        ),
        _ => format!(
            "{} 分，{} 个知识点明显薄弱，先补「{}」和「{}」",
            facts.overall,
            facts.weak.len(),
            facts.weak[0].0,
            facts.weak[1].0
        ),
    }
}

fn rule_plan(facts: &Facts) -> AdvicePlan {
    // 每天题量随薄弱点、到期错题浮动，但不超过 25 分钟，避免计划定得没人做。
    let due = facts.due_review;
    if !facts.has_data {
        return AdvicePlan {
            daily_minutes: 20,
            total_minutes: 100,
            focus: vec!["先产生一次可分析的作答".into()],
            days: vec![
                PlanDay {
                    day: "周一".into(),
                    task: "把最近一次作业交掉（不会的题先做会的）".into(),
                    minutes: 20,
                },
                PlanDay {
                    day: "周二".into(),
                    task: "对答案，把做错的题抄进错题本".into(),
                    minutes: 15,
                },
                PlanDay {
                    day: "周三".into(),
                    task: "错题重做一遍，标出卡住的那一步".into(),
                    minutes: 15,
                },
                PlanDay {
                    day: "周四".into(),
                    task: "挑 1 个最没底的知识点，看 1 个例题 + 做 3 道同类题".into(),
                    minutes: 20,
                },
                PlanDay {
                    day: "周五".into(),
                    task: "小测 10 道（覆盖本周做过的题）".into(),
                    minutes: 15,
                },
                PlanDay {
                    day: "周末".into(),
                    task: "回看错题本，把已经会了的标「已掌握」".into(),
                    minutes: 15,
                },
            ],
        };
    }

    let focus: Vec<String> = if facts.weak.is_empty() {
        facts
            .strong
            .iter()
            .map(|(name, _, _)| name.clone())
            .collect()
    } else {
        facts.weak.iter().map(|(name, _, _)| name.clone()).collect()
    };
    let first = facts.weak.first().map(|row| row.0.clone());
    let second = facts.weak.get(1).map(|row| row.0.clone());
    let due_task = if due > 0 {
        format!(
            "错题复习：先做 {} 道到期的（做对就点「复习过」）",
            due.min(8)
        )
    } else if facts.wrong_count > 0 {
        format!(
            "错题回看：错题本里 {} 道，挑 5 道重做",
            facts.wrong_count.min(20)
        )
    } else {
        "基础题保持手感：做 5 道已掌握知识点的题".to_string()
    };

    let mut days = vec![
        PlanDay {
            day: "周一".into(),
            task: due_task,
            minutes: if due > 0 { 20 } else { 12 },
        },
        PlanDay {
            day: "周二".into(),
            task: match &first {
                Some(kp) => format!("「{kp}」专项：看 1 个例题，再做 8 道同类题"),
                None => "综合小练 8 道（限时 15 分钟）".into(),
            },
            minutes: 20,
        },
        PlanDay {
            day: "周三".into(),
            task: "错题重做：把周一的错题隔一天再做一遍，仍错的画星号".into(),
            minutes: 15,
        },
        PlanDay {
            day: "周四".into(),
            task: match &second {
                Some(kp) => format!("「{kp}」专项：看 1 个例题，再做 8 道同类题"),
                None => match &first {
                    Some(kp) => format!("「{kp}」变式 6 道（换个问法）"),
                    None => "把本周错题按知识点归类，各做 2 道".into(),
                },
            },
            minutes: 20,
        },
        PlanDay {
            day: "周五".into(),
            task: format!(
                "综合小练 10 道，优先覆盖{}",
                if focus.is_empty() {
                    "本周做错的内容".to_string()
                } else {
                    format!("「{}」", focus.join("」「"))
                }
            ),
            minutes: 25,
        },
        PlanDay {
            day: "周六".into(),
            task: "本周错题回看：只做画星号的，做对了标「已掌握」".into(),
            minutes: 15,
        },
        PlanDay {
            day: "周日".into(),
            task: "休息；愿意的话把下周要学的内容翻一遍".into(),
            minutes: 10,
        },
    ];
    if facts.days_idle.map(|days| days >= 14).unwrap_or(false) {
        // 两周以上没练：先恢复手感，别一上来就加难度。
        days[0].task = format!("恢复手感：先做 5 道简单题（{}）", facts.idle_text());
        days[0].minutes = 12;
    }
    let total: i64 = days.iter().map(|day| day.minutes).sum();
    AdvicePlan {
        daily_minutes: (total / days.len() as i64).clamp(10, 30),
        total_minutes: total,
        focus,
        days,
    }
}

fn rule_issues(facts: &Facts) -> Vec<AdviceIssue> {
    if !facts.has_data {
        return vec![AdviceIssue {
            kp: "没有作答数据".into(),
            why: "掌握度只能从已判分的作答里算出来，没有作答就没有依据".into(),
            action: "先交一次作业；老师批完后这里会自动给出薄弱点".into(),
        }];
    }
    let mut out: Vec<AdviceIssue> = facts
        .weak
        .iter()
        .map(|(name, mastery, answered)| AdviceIssue {
            kp: name.clone(),
            why: {
                let mut why = format!("掌握度 {mastery}，已判 {answered} 题；做错的题集中在这里，说明方法或概念有一处没打通");
                if let Some((count, last)) = facts
                    .wrong_by_kp
                    .iter()
                    .find(|(kp, _, _)| kp == name)
                    .map(|(_, count, last)| (count, last))
                {
                    why.push_str(&format!(
                        "；错题本里有 {} 道挂在这个点上（最近 {}）",
                        count,
                        last.get(..10).unwrap_or(last)
                    ));
                }
                why
            },
            action: if *answered < 4 {
                format!("题量太少，先做 6–8 道「{name}」的基础题，再看掌握度是否回升")
            } else {
                format!("先看 1 个例题理清步骤，再做 8 道「{name}」同类题，隔天重做错题")
            },
        })
        .collect();
    if facts.delta <= -6 {
        out.insert(
            0,
            AdviceIssue {
                kp: "近两周退步".into(),
                why: format!(
                    "掌握度比两周前低 {} 分，可能是新内容没跟上，或做题节奏断了",
                    -facts.delta
                ),
                action: "先用错题重做止损，再按本周安排恢复每天 15–20 分钟".into(),
            },
        );
    }
    if out.is_empty() {
        out.push(AdviceIssue {
            kp: "没有明显短板".into(),
            why: "所有练过的知识点都在 70 分以上".into(),
            action: "保持每周 2–3 次练习，适当加一点综合题和变式题".into(),
        });
    }
    out.truncate(3);
    out
}

fn rule_strengths(facts: &Facts) -> Vec<AdviceStrength> {
    let mut out: Vec<AdviceStrength> = facts
        .strong
        .iter()
        .map(|(name, mastery, answered)| AdviceStrength {
            point: name.clone(),
            evidence: format!("掌握度 {mastery}，已判 {answered} 题"),
        })
        .collect();
    if out.is_empty() {
        if facts.has_data && facts.overall >= 60 {
            out.push(AdviceStrength {
                point: "整体基础".into(),
                evidence: format!(
                    "掌握度 {}，得分率 {}%",
                    facts.overall,
                    (facts.score_rate * 100.0).round() as i64
                ),
            });
        }
    }
    out.truncate(2);
    out
}

fn rule_parent_note(facts: &Facts, plan: &AdvicePlan) -> String {
    if !facts.has_data {
        return "本周先不安排额外练习，重点是让孩子把作业交上来、把错题记下来；有数据之后再定计划。"
            .into();
    }
    let focus = if plan.focus.is_empty() {
        "本周做错的内容".to_string()
    } else {
        plan.focus.join("、")
    };
    let mut note = format!(
        "本周共安排约 {} 分钟，平均每天 {} 分钟，集中在{}。",
        plan.total_minutes, plan.daily_minutes, focus
    );
    if facts.due_review > 0 {
        note.push_str(&format!(
            "其中 {} 道错题已到复习时间，先复习再刷新题。",
            facts.due_review
        ));
    }
    if facts.days_idle.map(|days| days >= 14).unwrap_or(false) {
        note.push_str("已经两周以上没练，先从简单题恢复手感，不要一次加太多。");
    } else if facts.delta > 3 {
        note.push_str("近两周在进步，保持现在的节奏即可。");
    }
    note.push_str("建议固定时段完成，做完在错题本点「复习过」，下次就能按记忆节奏提醒。");
    note
}

fn rule_advice(facts: &Facts) -> Advice {
    let plan = rule_plan(facts);
    let (level, label) = level_of(facts);
    Advice {
        student_id: facts.student_id.clone(),
        level: level.into(),
        level_label: label.into(),
        headline: headline_of(facts),
        strengths: rule_strengths(facts),
        issues: rule_issues(facts),
        parent_note: rule_parent_note(facts, &plan),
        plan,
        source: "RULE".into(),
        created_at: text::now(),
        stale: false,
        facts: facts.fact_lines(),
    }
}

// ---------------------------------------------------------------------------
// AI 版
// ---------------------------------------------------------------------------

/// AI 必须按这个形状回话；少字段不算错，前端会退回规则版那一段。
fn validate(raw: &Value, base: &Advice) -> Option<Advice> {
    let object = raw.as_object()?;
    let text_of = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };

    let headline = text_of("headline").unwrap_or_else(|| base.headline.clone());
    let mut advice = base.clone();
    advice.headline = headline;
    advice.source = "AI".into();
    advice.created_at = text::now();

    if let Some(note) = text_of("parentNote") {
        advice.parent_note = note;
    }

    if let Some(items) = object.get("strengths").and_then(Value::as_array) {
        let parsed: Vec<AdviceStrength> = items
            .iter()
            .filter_map(|item| {
                let point = item.get("point")?.as_str()?.trim().to_string();
                let evidence = item
                    .get("evidence")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if point.is_empty() {
                    return None;
                }
                Some(AdviceStrength { point, evidence })
            })
            .take(3)
            .collect();
        if !parsed.is_empty() {
            advice.strengths = parsed;
        }
    }

    if let Some(items) = object.get("issues").and_then(Value::as_array) {
        let parsed: Vec<AdviceIssue> = items
            .iter()
            .filter_map(|item| {
                let kp = item.get("kp")?.as_str()?.trim().to_string();
                let why = item.get("why")?.as_str()?.trim().to_string();
                let action = item.get("action")?.as_str()?.trim().to_string();
                if kp.is_empty() || action.is_empty() {
                    return None;
                }
                Some(AdviceIssue { kp, why, action })
            })
            .take(4)
            .collect();
        if !parsed.is_empty() {
            advice.issues = parsed;
        }
    }

    if let Some(plan) = object.get("plan") {
        let days: Vec<PlanDay> = plan
            .get("days")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let day = item.get("day")?.as_str()?.trim().to_string();
                        let task = item.get("task")?.as_str()?.trim().to_string();
                        if day.is_empty() || task.is_empty() {
                            return None;
                        }
                        let minutes = item
                            .get("minutes")
                            .and_then(Value::as_i64)
                            .unwrap_or(15)
                            .clamp(5, 60);
                        Some(PlanDay { day, task, minutes })
                    })
                    .take(7)
                    .collect()
            })
            .unwrap_or_default();
        // 少于三天就不像「安排」了，直接保留规则版。
        if days.len() >= 3 {
            let total: i64 = days.iter().map(|day| day.minutes).sum();
            advice.plan = AdvicePlan {
                daily_minutes: (total / days.len() as i64).clamp(10, 40),
                total_minutes: total,
                focus: plan
                    .get("focus")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str())
                            .map(|item| item.trim().to_string())
                            .filter(|item| !item.is_empty())
                            .collect()
                    })
                    .filter(|items: &Vec<String>| !items.is_empty())
                    .unwrap_or_else(|| base.plan.focus.clone()),
                days,
            };
        }
    }
    Some(advice)
}

pub fn generate(
    conn: &Connection,
    http: &reqwest::blocking::Client,
    student_id: &str,
    force: bool,
) -> Result<Advice, String> {
    let facts = facts_of(conn, student_id)?;
    let fingerprint = fingerprint_of(&facts);
    if !force {
        if let Some((cached, stale)) = stored(conn, student_id, &fingerprint) {
            // 缓存里是规则版时仍然重试一次 AI：用户点「AI 诊断」就是想试试模型。
            if !stale && cached.source == "AI" {
                return Ok(cached);
            }
        }
    }

    let base = rule_advice(&facts);
    let messages = llm::prompts::advice(&facts.json());
    let mut cooldowns = std::collections::HashMap::new();
    let outcome = llm::complete(
        conn,
        http,
        "advice",
        &messages,
        None,
        true,
        2000,
        &mut cooldowns,
    );
    let advice = match outcome {
        Ok(raw) => match llm::parse_json(&raw, "学情建议") {
            Ok(parsed) => validate(&parsed, &base).unwrap_or(base),
            Err(_) => base,
        },
        // 没配模型 / 调用失败：规则版照样能用，不把错误丢给用户。
        Err(_) => base,
    };
    save(conn, student_id, &fingerprint, &advice)?;
    Ok(advice)
}

/// 读已存的报告；`stale` 表示生成后数据又变了。
pub fn get(conn: &Connection, student_id: &str) -> Result<Advice, String> {
    let facts = facts_of(conn, student_id)?;
    let fingerprint = fingerprint_of(&facts);
    if let Some((mut advice, stale)) = stored(conn, student_id, &fingerprint) {
        advice.stale = stale;
        return Ok(advice);
    }
    Ok(rule_advice(&facts))
}

fn fingerprint_of(facts: &Facts) -> String {
    let mut hasher = DefaultHasher::new();
    facts.json().to_string().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn stored(conn: &Connection, student_id: &str, fingerprint: &str) -> Option<(Advice, bool)> {
    let row: Option<(String, String, String)> = conn
        .query_row(
            "select fingerprint, payload, created_at from local_insight_report where student_id = ?1",
            [student_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .ok()
        .flatten();
    let (saved_fp, payload, created_at) = row?;
    let mut advice: Advice = serde_json::from_str(&payload).ok()?;
    advice.created_at = created_at;
    advice.stale = saved_fp != fingerprint;
    let stale = advice.stale;
    Some((advice, stale))
}

fn save(
    conn: &Connection,
    student_id: &str,
    fingerprint: &str,
    advice: &Advice,
) -> Result<(), String> {
    let payload = serde_json::to_string(advice).map_err(|e| e.to_string())?;
    conn.execute(
        "insert into local_insight_report (student_id, fingerprint, source, model, payload, created_at)
         values (?1, ?2, ?3, '', ?4, ?5)
         on conflict (student_id) do update
            set fingerprint = excluded.fingerprint,
                source = excluded.source,
                payload = excluded.payload,
                created_at = excluded.created_at",
        params![student_id, fingerprint, advice.source, payload, advice.created_at],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
