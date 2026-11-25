//! 题库统计：回答「我有多少题、都长什么样、还缺什么」。
//!
//! 统计本身都是很直接的 group by，真正有价值的是最后那段**补题建议**：
//! 零题的学科、某个学科缺整类题型、难度全挤在中档、有题没挂知识点——
//! 这些在组卷和学情统计时都会变成实际问题，但单看题库列表是发现不了的。
//!
//! 口径按**用户选择的学科范围**计算：`subjects` 为空看全库；选一个学科时
//! 单学科模式（看章节覆盖、章节分布，建议不再按学科点名）；选多个学科时
//! 所有数字与建议都只落在选中的学科上。建议全部由本地规则算出来，不叫模型。
//!
//! `bySubject` 永远按选中范围出（全库选择时就是全库），前端的选择器
//! 用首次加载的全量数字缓存，不跟随过滤变化。

use rusqlite::{params_from_iter, Connection};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub key: String,
    pub label: String,
    pub count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterGap {
    pub subject_id: String,
    pub subject: String,
    pub chapter: String,
    pub count: i64,
}

/// 一条补题建议。`action` 是前端要跳去的页面，`subject_id` 用来预选学科；
/// `types` / `difficulty` 能带时一起带给出题页做预选，科目是必带的。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// high / mid / low：high 会直接影响组卷可用性，low 只是整理建议。
    pub level: String,
    pub title: String,
    pub detail: String,
    pub action: String,
    pub subject_id: Option<String>,
    /// 建议补的题型代号（SINGLE/MULTI/FILL/SOLVED），出题页直接预选。
    pub types: Vec<String>,
    /// 建议的难度目标（1–5），仅难度结构类建议携带。
    pub difficulty: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub total: i64,
    pub published: i64,
    pub draft: i64,
    pub needs_review: i64,
    pub linked: i64,
    pub unlinked: i64,
    pub points_total: i64,
    pub points_covered: i64,
    pub subjects_covered: i64,
    pub subjects_total: i64,
    /// 选中范围内的章节数 / 有题的章节数：单学科模式下顶替「学科覆盖」。
    pub chapters_total: i64,
    pub chapters_covered: i64,
    /// true = 只选了一个学科，前端按单学科口径排版。
    pub single_subject: bool,
    pub recent30: i64,
    pub by_subject: Vec<Bucket>,
    pub by_chapter: Vec<Bucket>,
    /// 单学科口径下的知识点分布：按挂题量降序，只留有题的考点。
    /// 章节名（"第一章 …"）来自教材目录快照，跟实际挂题的考点对不齐；
    /// 用户看结构时真正关心的是"哪些考点有题"，所以主图改用知识点。
    pub by_point: Vec<Bucket>,
    pub by_type: Vec<Bucket>,
    pub by_difficulty: Vec<Bucket>,
    pub months: Vec<Bucket>,
    pub empty_chapters: Vec<ChapterGap>,
    /// 没题的考点：补题时按这个挑，比空章节更贴近实际编排。
    pub empty_points: Vec<Bucket>,
    pub suggestions: Vec<Suggestion>,
}

const TYPE_LABELS: [(&str, &str); 4] = [
    ("SINGLE", "单选题"),
    ("MULTI", "多选题"),
    ("FILL", "填空题"),
    ("SOLVED", "解答题"),
];

const DIFFICULTY_LABELS: [&str; 5] = ["1 · 很容易", "2 · 容易", "3 · 中等", "4 · 较难", "5 · 很难"];

/// 选中学科的过滤片段：`{F}` 处展开成 ` and <列> in (?,…)`；全库时是空串。
/// 参数在每条查询里只出现一次、按文本顺序绑定，配合 `scalar` / `rows` 使用。
fn filter_clause(subjects: &[String], column: &str) -> String {
    if subjects.is_empty() {
        return String::new();
    }
    let slots: Vec<&str> = subjects.iter().map(|_| "?").collect();
    format!(" and {} in ({})", column, slots.join(", "))
}

fn scalar(conn: &Connection, sql: &str, subjects: &[String]) -> Result<i64, String> {
    let mut statement = conn.prepare(sql).map_err(|error| error.to_string())?;
    let value: i64 = if subjects.is_empty() {
        statement.query_row([], |row| row.get(0))
    } else {
        statement.query_row(params_from_iter(subjects.iter()), |row| row.get(0))
    }
    .map_err(|error| error.to_string())?;
    Ok(value)
}

fn rows<T, F>(conn: &Connection, sql: &str, subjects: &[String], map: F) -> Result<Vec<T>, String>
where
    F: Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = conn.prepare(sql).map_err(|error| error.to_string())?;
    let rows = if subjects.is_empty() {
        statement.query_map([], map)
    } else {
        statement.query_map(params_from_iter(subjects.iter()), map)
    }
    .map_err(|error| error.to_string())?;
    Ok(rows.filter_map(|row| row.ok()).collect())
}

pub fn collect(conn: &Connection, subjects: &[String]) -> Result<Stats, String> {
    let single = subjects.len() == 1;

    let total = scalar(
        conn,
        &format!(
            "select count(*) from question where deleted_at is null{}",
            filter_clause(subjects, "subject_id")
        ),
        subjects,
    )?;
    let published = scalar(
        conn,
        &format!(
            "select count(*) from question where deleted_at is null and status = 'PUBLISHED'{}",
            filter_clause(subjects, "subject_id")
        ),
        subjects,
    )?;
    let draft = scalar(
        conn,
        &format!(
            "select count(*) from question where deleted_at is null and status <> 'PUBLISHED'{}",
            filter_clause(subjects, "subject_id")
        ),
        subjects,
    )?;
    let needs_review = scalar(
        conn,
        &format!(
            "select count(*) from question where deleted_at is null and needs_review = 1{}",
            filter_clause(subjects, "subject_id")
        ),
        subjects,
    )?;
    let unlinked = scalar(
        conn,
        &format!(
            "select count(*) from question q where q.deleted_at is null \
             and not exists (select 1 from question_knowledge k where k.question_id = q.id){}",
            filter_clause(subjects, "q.subject_id")
        ),
        subjects,
    )?;
    let linked = (total - unlinked).max(0);
    // 知识点总量也要跟着范围走：单学科时只数这个学科的考点。
    let points_total = scalar(
        conn,
        &format!(
            "select count(*) from knowledge_point kp \
             join chapter c on c.id = kp.chapter_id \
             join textbook t on t.id = c.textbook_id \
             where 1=1{}",
            filter_clause(subjects, "t.subject_id")
        ),
        subjects,
    )?;
    let points_covered = scalar(
        conn,
        &format!(
            "select count(distinct kp.id) from knowledge_point kp \
             join question_knowledge qk on qk.knowledge_point_id = kp.id \
             join question q on q.id = qk.question_id and q.deleted_at is null \
             where 1=1{}",
            filter_clause(subjects, "q.subject_id")
        ),
        subjects,
    )?;
    let recent30 = scalar(
        conn,
        &format!(
            "select count(*) from question where deleted_at is null \
             and created_at >= datetime('now', '-30 days'){}",
            filter_clause(subjects, "subject_id")
        ),
        subjects,
    )?;

    let by_subject = subject_buckets(conn, subjects)?;
    let subjects_covered = by_subject.iter().filter(|bucket| bucket.count > 0).count() as i64;
    let subjects_total = if subjects.is_empty() {
        by_subject.len() as i64
    } else {
        subjects.len() as i64
    };
    let by_type = type_buckets(conn, subjects)?;
    let by_difficulty = difficulty_buckets(conn, subjects)?;
    let months = month_buckets(conn, subjects)?;
    let empty_chapters = empty_chapters(conn, subjects)?;
    let chapters_total = chapter_total(conn, subjects)?;
    let chapters_covered = (chapters_total - empty_chapters.len() as i64).max(0);
    let by_chapter = chapter_buckets(conn, subjects)?;
    let by_point = point_buckets(conn, subjects)?;
    let empty_points = empty_points(conn, subjects)?;
    let subject_types = subject_type_matrix(conn, subjects)?;

    let suggestions = suggest(&Context {
        total,
        draft,
        needs_review,
        unlinked,
        recent30,
        single,
        by_subject: &by_subject,
        by_difficulty: &by_difficulty,
        empty_chapters: &empty_chapters,
        subject_types: &subject_types,
    });

    Ok(Stats {
        total,
        published,
        draft,
        needs_review,
        linked,
        unlinked,
        points_total,
        points_covered,
        subjects_covered,
        subjects_total,
        chapters_total,
        chapters_covered,
        single_subject: single,
        recent30,
        by_subject,
        by_chapter,
        by_point,
        by_type,
        by_difficulty,
        months,
        empty_chapters,
        empty_points,
        suggestions,
    })
}

fn subject_buckets(conn: &Connection, subjects: &[String]) -> Result<Vec<Bucket>, String> {
    let sql = format!(
        "select s.id, s.name, count(q.id) \
         from subject s \
         left join question q on q.subject_id = s.id and q.deleted_at is null \
         where 1=1{} \
         group by s.id, s.name, s.sort order by s.sort",
        filter_clause(subjects, "s.id")
    );
    let buckets = rows(conn, &sql, subjects, |row| {
        Ok(Bucket {
            key: row.get(0)?,
            label: row.get(1)?,
            count: row.get(2)?,
        })
    })?;
    // 题多的学科排前面；同数量保持学科顺序（SQL 已按 sort 出）。
    let mut buckets = buckets;
    buckets.sort_by(|a, b| b.count.cmp(&a.count));
    Ok(buckets)
}

fn type_buckets(conn: &Connection, subjects: &[String]) -> Result<Vec<Bucket>, String> {
    let sql = format!(
        "select type, count(*) from question where deleted_at is null{} group by type",
        filter_clause(subjects, "subject_id")
    );
    let mut counts: HashMap<String, i64> = HashMap::new();
    for (kind, count) in rows(conn, &sql, subjects, |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })? {
        counts.insert(kind, count);
    }
    Ok(TYPE_LABELS
        .iter()
        .map(|(key, label)| Bucket {
            key: (*key).to_string(),
            label: (*label).to_string(),
            count: counts.get(*key).copied().unwrap_or(0),
        })
        .collect())
}

fn difficulty_buckets(conn: &Connection, subjects: &[String]) -> Result<Vec<Bucket>, String> {
    let sql = format!(
        "select difficulty, count(*) from question where deleted_at is null{} group by difficulty",
        filter_clause(subjects, "subject_id")
    );
    let mut counts: HashMap<i64, i64> = HashMap::new();
    for (level, count) in rows(conn, &sql, subjects, |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })? {
        counts.insert(level, count);
    }
    Ok((1..=5)
        .map(|level| Bucket {
            key: level.to_string(),
            label: DIFFICULTY_LABELS[(level - 1) as usize].to_string(),
            count: counts.get(&level).copied().unwrap_or(0),
        })
        .collect())
}

/// 最近 6 个月的新增题量；没有题的月份补 0，趋势才不会断。
fn month_buckets(conn: &Connection, subjects: &[String]) -> Result<Vec<Bucket>, String> {
    let sql = format!(
        "with recursive months(index_of, key) as ( \
             select 0, strftime('%Y-%m', 'now') \
             union all \
             select index_of + 1, strftime('%Y-%m', 'now', printf('-%d months', index_of + 1)) \
             from months where index_of < 5 \
         ) \
         select m.key, (select count(*) from question q \
             where q.deleted_at is null and substr(q.created_at, 1, 7) = m.key{}) \
         from months m order by m.key",
        filter_clause(subjects, "q.subject_id")
    );
    let buckets = rows(conn, &sql, subjects, |row| {
        let key: String = row.get(0)?;
        let count: i64 = row.get(1)?;
        Ok(Bucket {
            // 图上只要「11月」这种短标签。
            label: key
                .get(5..)
                .map(|month| format!("{month}月"))
                .unwrap_or_else(|| key.clone()),
            key,
            count,
        })
    })?;
    Ok(buckets)
}

/// 一道题都没有的章节：补题时最该先看这个，不然组卷会一直绕开那几章。
fn empty_chapters(conn: &Connection, subjects: &[String]) -> Result<Vec<ChapterGap>, String> {
    let sql = format!(
        "select s.id, s.name, c.name, count(qk.question_id) \
         from chapter c \
         join textbook t on t.id = c.textbook_id \
         join subject s on s.id = t.subject_id \
         left join knowledge_point kp on kp.chapter_id = c.id \
         left join question_knowledge qk on qk.knowledge_point_id = kp.id \
         where 1=1{} \
         group by c.id having count(qk.question_id) = 0 \
         order by s.sort, t.sort, c.sort",
        filter_clause(subjects, "s.id")
    );
    let gaps = rows(conn, &sql, subjects, |row| {
        Ok(ChapterGap {
            subject_id: row.get(0)?,
            subject: row.get(1)?,
            chapter: row.get(2)?,
            count: 0,
        })
    })?;
    Ok(gaps)
}

/// 选中范围内的章节数（单学科 KPI 用）。
fn chapter_total(conn: &Connection, subjects: &[String]) -> Result<i64, String> {
    let sql = format!(
        "select count(*) from chapter c \
         join textbook t on t.id = c.textbook_id \
         where 1=1{}",
        filter_clause(subjects, "t.subject_id")
    );
    scalar(conn, &sql, subjects)
}

/// 选中范围内每章的题量（按挂知识点归章）；空章也留在列表里，缺口一眼可见。
fn chapter_buckets(conn: &Connection, subjects: &[String]) -> Result<Vec<Bucket>, String> {
    let sql = format!(
        "select t.name, c.name, count(distinct q.id) \
         from chapter c \
         join textbook t on t.id = c.textbook_id \
         left join knowledge_point kp on kp.chapter_id = c.id \
         left join question_knowledge qk on qk.knowledge_point_id = kp.id \
         left join question q on q.id = qk.question_id and q.deleted_at is null \
         where 1=1{} \
         group by c.id, t.name, c.name, t.sort, c.sort \
         order by t.sort, c.sort",
        filter_clause(subjects, "t.subject_id")
    );
    let raw: Vec<(String, String, i64)> = rows(conn, &sql, subjects, |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    // 章名撞了（上下册同名章）才加教材前缀，平时只显示章名。
    let mut duplicated: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        let mut names: Vec<&str> = raw.iter().map(|(_, chapter, _)| chapter.as_str()).collect();
        names.sort_unstable();
        for pair in names.windows(2) {
            if pair[0] == pair[1] {
                duplicated.insert(pair[0].to_string());
            }
        }
    }
    Ok(raw
        .into_iter()
        .map(|(textbook, chapter, count)| {
            let label = if duplicated.contains(&chapter) {
                format!("{textbook} · {chapter}")
            } else {
                chapter.clone()
            };
            Bucket {
                key: format!("{textbook}/{chapter}"),
                label,
                count,
            }
        })
        .collect())
}

/// 选中范围内有题的知识点，按题量降序。
/// 只列有题的考点：教材目录里那堆"第一章 …"的空章对看结构没帮助，
/// 真正要补的缺口放到「还没题的考点」里单独说。
fn point_buckets(conn: &Connection, subjects: &[String]) -> Result<Vec<Bucket>, String> {
    let sql = format!(
        "select kp.id, kp.name, count(distinct q.id) as n \
         from knowledge_point kp \
         join question_knowledge qk on qk.knowledge_point_id = kp.id \
         join question q on q.id = qk.question_id and q.deleted_at is null \
         where 1=1{} \
         group by kp.id, kp.name \
         order by n desc, kp.name",
        filter_clause(subjects, "kp.subject_id")
    );
    rows(conn, &sql, subjects, |row| {
        Ok(Bucket {
            key: row.get(0)?,
            label: row.get(1)?,
            count: row.get(2)?,
        })
    })
}

/// 一道题都没有的考点：单学科口径下补题时的首选清单。
/// 用 not exists 而不是 count=0 的 left join，避免同一考点挂多道题时的重复计数干扰。
fn empty_points(conn: &Connection, subjects: &[String]) -> Result<Vec<Bucket>, String> {
    let sql = format!(
        "select kp.id, kp.name, 0 \
         from knowledge_point kp \
         where 1=1{} \
           and not exists (\
             select 1 from question_knowledge qk \
             join question q on q.id = qk.question_id and q.deleted_at is null \
             where qk.knowledge_point_id = kp.id) \
         order by kp.sort, kp.name",
        filter_clause(subjects, "kp.subject_id")
    );
    rows(conn, &sql, subjects, |row| {
        Ok(Bucket {
            key: row.get(0)?,
            label: row.get(1)?,
            count: 0,
        })
    })
}

/// (学科, 题型) → 题数。缺哪类题型只能按学科单独看，光看总数看不出来。
fn subject_type_matrix(
    conn: &Connection,
    subjects: &[String],
) -> Result<HashMap<(String, String), i64>, String> {
    let sql = format!(
        "select subject_id, type, count(*) from question \
         where deleted_at is null{} group by subject_id, type",
        filter_clause(subjects, "subject_id")
    );
    let pairs: Vec<(String, String, i64)> = rows(conn, &sql, subjects, |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    Ok(pairs
        .into_iter()
        .map(|(subject, kind, count)| ((subject, kind), count))
        .collect())
}

struct Context<'a> {
    total: i64,
    draft: i64,
    needs_review: i64,
    unlinked: i64,
    recent30: i64,
    /// 只选了一个学科：建议措辞按「这个学科」说，不再按学科点名。
    single: bool,
    by_subject: &'a [Bucket],
    by_difficulty: &'a [Bucket],
    empty_chapters: &'a [ChapterGap],
    subject_types: &'a HashMap<(String, String), i64>,
}

impl Context<'_> {
    /// 当前统计范围的代表学科：单学科就是它，多学科取题最多的那个。
    /// 「去出题」跳转时必带，出题页才能自动选对学科。
    fn scope_subject(&self) -> Option<String> {
        self.by_subject.first().map(|bucket| bucket.key.clone())
    }
}

fn suggest(context: &Context) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();

    // 1) 零题学科：最硬的缺口，直接影响这个学科能不能组卷。
    //    单学科模式不说「N 个学科」，直接说这个学科本身。
    let empty: Vec<&Bucket> = context
        .by_subject
        .iter()
        .filter(|bucket| bucket.count == 0)
        .collect();
    if context.single {
        if context.total == 0 {
            let name = context
                .by_subject
                .first()
                .map(|bucket| bucket.label.as_str())
                .unwrap_or("这个学科");
            out.push(Suggestion {
                level: "high".into(),
                title: format!("「{name}」还没有题目"),
                detail: "组卷、作业、学情都拿不到料。建议先补 20–30 道基础题，把流程打通。".into(),
                action: "generate".into(),
                types: Vec::new(),
                difficulty: None,
                subject_id: context.by_subject.first().map(|bucket| bucket.key.clone()),
            });
        }
    } else if !empty.is_empty() {
        let names: Vec<&str> = empty.iter().take(4).map(|bucket| bucket.label.as_str()).collect();
        out.push(Suggestion {
            level: "high".into(),
            title: format!("{} 个学科还没有题目", empty.len()),
            detail: format!(
                "{} 目前是 0 题，组卷、作业、学情都拿不到料。建议先给每个学科补 20–30 道基础题，把流程打通。",
                names.join("、")
            ),
            action: "generate".into(),
            types: Vec::new(),
            difficulty: None,
            subject_id: empty.first().map(|bucket| bucket.key.clone()),
        });
    }

    // 2) 题型缺口：不看总体，按学科逐个查，所以能点名「数学缺填空题」。
    //    单学科时那个学科就是唯一候选，措辞里不必再带学科名。
    let type_gap_candidates: Vec<&Bucket> = if context.single {
        context.by_subject.iter().collect()
    } else {
        context
            .by_subject
            .iter()
            .filter(|bucket| bucket.count >= 10)
            .collect()
    };
    for bucket in type_gap_candidates {
        let missing: Vec<&str> = TYPE_LABELS
            .iter()
            .filter(|(kind, _)| {
                context
                    .subject_types
                    .get(&(bucket.key.clone(), (*kind).to_string()))
                    .copied()
                    .unwrap_or(0)
                    == 0
            })
            .map(|(_, label)| *label)
            .collect();
        let missing_codes: Vec<String> = TYPE_LABELS
            .iter()
            .filter(|(kind, _)| {
                context
                    .subject_types
                    .get(&(bucket.key.clone(), (*kind).to_string()))
                    .copied()
                    .unwrap_or(0)
                    == 0
            })
            .map(|(kind, _)| (*kind).to_string())
            .collect();
        if missing.is_empty() {
            continue;
        }
        let title = if context.single {
            format!("还没有{}", missing.join("、"))
        } else {
            format!("「{}」没有{}", bucket.label, missing.join("、"))
        };
        out.push(Suggestion {
            level: "mid".into(),
            title,
            detail: format!(
                "已有 {} 道题，但{}是 0 道，客观题与主观题的训练面不平衡。",
                bucket.count,
                missing.join("、")
            ),
            action: "generate".into(),
            types: missing_codes,
            difficulty: None,
            subject_id: Some(bucket.key.clone()),
        });
    }

    // 3) 难度结构：全挤在中等，出卷拉不开区分度。
    if context.total >= 20 {
        let easy: i64 = context.by_difficulty.iter().take(2).map(|b| b.count).sum();
        let hard: i64 = context.by_difficulty.iter().skip(3).map(|b| b.count).sum();
        let percent = |part: i64| (part as f64 * 100.0 / context.total as f64).round() as i64;
        if percent(easy) < 15 {
            out.push(Suggestion {
                level: "mid".into(),
                title: "简单题（1–2 星）偏少".into(),
                detail: format!(
                    "只占 {}%，基础题不够覆盖课堂巩固，建议补到 25% 左右。",
                    format!("{}%", percent(easy))
                ),
                action: "generate".into(),
                types: Vec::new(),
                difficulty: Some(2),
                subject_id: context.scope_subject(),
            });
        }
        if percent(hard) < 10 {
            out.push(Suggestion {
                level: "mid".into(),
                title: "难题（4–5 星）偏少".into(),
                detail: format!(
                    "只占 {}%，出卷时压轴题不好挑，培优题建议补到 15% 上下。",
                    format!("{}%", percent(hard))
                ),
                action: "generate".into(),
                types: Vec::new(),
                difficulty: Some(4),
                subject_id: context.scope_subject(),
            });
        }
    }

    // 4) 没挂知识点的题：学情按知识点统计，这些题在报表里等于不存在。
    if context.unlinked > 0 {
        let heavy = context.unlinked * 10 > context.total.max(1);
        out.push(Suggestion {
            level: if heavy { "mid" } else { "low" }.into(),
            title: format!("{} 道题没挂知识点", context.unlinked),
            detail: "学情和知识点掌握度只统计挂过知识点的题；这些题不会出现在任何知识点报表里。"
                .into(),
            action: "library".into(),
            types: Vec::new(),
            difficulty: None,
            subject_id: context.scope_subject(),
        });
    }

    // 5) 空章节：补题最省力的定位方式。单学科时不用再带学科前缀。
    if !context.empty_chapters.is_empty() {
        let sample: Vec<String> = context
            .empty_chapters
            .iter()
            .take(3)
            .map(|gap| {
                if context.single {
                    gap.chapter.clone()
                } else {
                    format!("{} · {}", gap.subject, gap.chapter)
                }
            })
            .collect();
        out.push(Suggestion {
            level: "low".into(),
            title: format!("{} 个章节一道题都没有", context.empty_chapters.len()),
            detail: format!("例如 {}。整章空着，组卷时只能跳过去。", sample.join("；")),
            action: "library".into(),
            types: Vec::new(),
            difficulty: None,
            subject_id: context.empty_chapters.first().map(|gap| gap.subject_id.clone()),
        });
    }

    // 6) 待复核与草稿：整理欠账，不影响组卷但拖累质量。
    if context.needs_review > 0 {
        out.push(Suggestion {
            level: "low".into(),
            title: format!("{} 道题待复核", context.needs_review),
            detail: "识别或 AI 生成的题没人工确认过，答案可能有偏差。".into(),
            action: "library".into(),
            types: Vec::new(),
            difficulty: None,
            subject_id: None,
        });
    }
    if context.draft > 0 {
        out.push(Suggestion {
            level: "low".into(),
            title: format!("{} 道题还是草稿", context.draft),
            detail: "草稿默认不进组卷候选，确认无误后记得发布。".into(),
            action: "library".into(),
            types: Vec::new(),
            difficulty: None,
            subject_id: None,
        });
    }
    if context.total > 0 && context.recent30 == 0 {
        out.push(Suggestion {
            level: "low".into(),
            title: "最近 30 天没有新增题目".into(),
            detail: "题库会随教材和考纲过时，建议每月固定补一批。".into(),
            action: "generate".into(),
            types: Vec::new(),
            difficulty: None,
            subject_id: None,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::collect;
    use crate::db;
    use rusqlite::Connection;

    fn seeded() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        db::migrate(&mut conn).unwrap();
        db::seed_curriculum(&mut conn).unwrap();
        conn
    }

    fn add_question(conn: &Connection, subject: &str, kind: &str, difficulty: i64) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "insert into question (id, subject_id, type, stem, difficulty, status) \
             values (?1, ?2, ?3, ?4, ?5, 'PUBLISHED')",
            rusqlite::params![id, subject, kind, format!("题 {id}"), difficulty],
        )
        .unwrap();
        id
    }

    fn subject_id(conn: &Connection, name: &str) -> String {
        conn.query_row(
            "select id from subject where name = ?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
    }

    /// 统计口径要跟题库列表一致：软删除的题不算，题型缺口按学科单独判断。
    #[test]
    fn counts_questions_and_flags_gaps() {
        let conn = seeded();
        let math = subject_id(&conn, "数学");
        let physics = subject_id(&conn, "物理");

        for _ in 0..9 {
            add_question(&conn, &math, "SINGLE", 3);
        }
        add_question(&conn, &math, "SOLVED", 5);
        // 软删除的题不进统计。
        let gone = add_question(&conn, &math, "FILL", 2);
        conn.execute(
            "update question set deleted_at = datetime('now') where id = ?1",
            [&gone],
        )
        .unwrap();

        let stats = collect(&conn, &[]).unwrap();
        assert_eq!(stats.total, 10);
        assert_eq!(stats.published, 10);
        assert_eq!(stats.unlinked, 10, "这些题都没挂知识点");
        assert_eq!(stats.subjects_covered, 1);
        assert_eq!(stats.subjects_total, 9);
        assert!(!stats.single_subject);
        assert_eq!(
            stats.by_type.iter().find(|b| b.key == "FILL").unwrap().count,
            0
        );

        let titles: Vec<&str> = stats
            .suggestions
            .iter()
            .map(|item| item.title.as_str())
            .collect();
        assert!(
            titles.iter().any(|title| title.contains("没有题目")),
            "零题学科要提建议：{titles:?}"
        );
        assert!(
            titles.iter().any(|title| title.contains("数学") && title.contains("填空题")),
            "数学缺填空题要点名：{titles:?}"
        );
        assert!(titles.iter().any(|title| title.contains("没挂知识点")));

        // 全是中等难度 + 一道 5 星，不该报「难题偏少」。
        assert!(!titles.iter().any(|title| title.contains("难题")));

        // 物理是 0 题，建议里要能定位到它。
        let empty = stats
            .suggestions
            .iter()
            .find(|item| item.title.contains("没有题目"))
            .unwrap();
        assert!(empty.subject_id.is_some());
        assert_ne!(empty.subject_id.as_deref(), Some(math.as_str()));
        let _ = physics;
    }

    /// 题型齐全、难度分布正常时不乱提建议。
    #[test]
    fn quiet_when_library_is_healthy() {
        let conn = seeded();
        let math = subject_id(&conn, "数学");
        for kind in ["SINGLE", "MULTI", "FILL", "SOLVED"] {
            for difficulty in 1..=5 {
                add_question(&conn, &math, kind, difficulty);
            }
        }
        let stats = collect(&conn, &[]).unwrap();
        assert_eq!(stats.total, 20);
        let titles: Vec<&str> = stats
            .suggestions
            .iter()
            .map(|item| item.title.as_str())
            .collect();
        assert!(
            !titles.iter().any(|title| title.contains("数学")),
            "数学已经四类题型齐、难度均衡，不该再点它：{titles:?}"
        );
    }

    /// 单学科模式：所有口径只算选中学科，建议不再按学科点名。
    #[test]
    fn single_subject_scope_changes_dimension() {
        let conn = seeded();
        let math = subject_id(&conn, "数学");
        let physics = subject_id(&conn, "物理");
        for kind in ["SINGLE", "SOLVED"] {
            add_question(&conn, &math, kind, 3);
        }
        for _ in 0..5 {
            add_question(&conn, &physics, "SINGLE", 2);
        }

        let stats = collect(&conn, &[math.clone()]).unwrap();
        assert!(stats.single_subject);
        assert_eq!(stats.total, 2, "物理的 5 道题不进数学的统计");
        assert_eq!(stats.by_type.iter().find(|b| b.key == "SINGLE").unwrap().count, 1);
        assert_eq!(stats.subjects_total, 1);
        // 单学科才有章节口径：有章节数、有按章分布。
        assert!(stats.chapters_total > 0);
        assert_eq!(stats.chapters_covered, 0, "题没挂知识点，按章归不进去");
        assert!(stats.by_chapter.len() == stats.chapters_total as usize);
        // 题少于 10 道也照样点名题型缺口，措辞不带学科名。
        let titles: Vec<&str> = stats
            .suggestions
            .iter()
            .map(|item| item.title.as_str())
            .collect();
        assert!(
            titles.iter().any(|title| title.contains("多选题")),
            "数学缺多选/填空要直接说：{titles:?}"
        );
        assert!(
            !titles.iter().any(|title| title.contains("「数学」")),
            "单学科模式不按学科点名：{titles:?}"
        );
        assert!(
            !titles.iter().any(|title| title.contains("个学科还没有题目")),
            "单学科模式不报零题学科清单：{titles:?}"
        );

        // 全库模式下同样的题库：total 是 7，物理被零题规则之外……物理有 5 道，不命中。
        let all = collect(&conn, &[]).unwrap();
        assert_eq!(all.total, 7);
        assert!(!all.single_subject);
        assert_eq!(all.subjects_total, 9);
    }

    /// 多选学科：只落在选中范围内，没选的学科完全不进数字。
    #[test]
    fn multi_subject_scope_filters_everything() {
        let conn = seeded();
        let math = subject_id(&conn, "数学");
        let physics = subject_id(&conn, "物理");
        add_question(&conn, &math, "SINGLE", 3);
        add_question(&conn, &physics, "FILL", 4);
        add_question(&conn, &subject_id(&conn, "化学"), "SINGLE", 3);

        let picked = vec![math.clone(), physics.clone()];
        let stats = collect(&conn, &picked).unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.subjects_total, 2);
        assert_eq!(stats.subjects_covered, 2);
        assert_eq!(
            stats.by_subject.len(),
            2,
            "学科分布只出选中的两个学科"
        );
        let titles: Vec<&str> = stats
            .suggestions
            .iter()
            .map(|item| item.title.as_str())
            .collect();
        assert!(
            !titles.iter().any(|title| title.contains("化学")),
            "没选的学科不该出现在建议里：{titles:?}"
        );
    }
}

