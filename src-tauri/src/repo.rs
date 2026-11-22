//! 题目仓储：列表、详情、保存、软删除、统计。
//!
//! 所有「选题 / 列表 / 统计」查询都带 `deleted_at is null`，与中心系统一致；
//! 只有按 id 回看历史引用时才允许取到已删除的题。

use crate::db;
use crate::models::*;
use crate::sync;
use crate::text;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, ToSql};

const SELECT: &str = "select q.id, q.subject_id, ifnull(s.name, '未分类'), q.type, q.stem,
        q.options_json, q.answer, q.analysis, q.difficulty, q.source, q.year,
        q.status, q.origin, q.needs_review, q.ai_answer, q.ai_analysis, q.confidence,
        q.review_note, q.created_at, q.updated_at, q.deleted_at, q.sync_state, q.server_id
 from question q left join subject s on s.id = q.subject_id";

fn map_question(row: &rusqlite::Row<'_>) -> rusqlite::Result<Question> {
    let stem: String = row.get(4)?;
    let options_json: String = row.get(5)?;
    let kind: String = row.get(3)?;
    Ok(Question {
        id: row.get(0)?,
        subject_id: row.get(1)?,
        subject_name: row.get(2)?,
        type_label: text::type_label(&kind).to_string(),
        kind,
        excerpt: text::excerpt(&stem, 120),
        stem,
        options: serde_json::from_str(&options_json).unwrap_or_default(),
        answer: row.get(6)?,
        analysis: row.get(7)?,
        difficulty: row.get(8)?,
        source: row.get(9)?,
        year: row.get(10)?,
        status: row.get(11)?,
        origin: row.get(12)?,
        needs_review: row.get::<_, i64>(13)? != 0,
        ai_answer: row.get::<_, i64>(14)? != 0,
        ai_analysis: row.get::<_, i64>(15)? != 0,
        confidence: row.get(16)?,
        review_note: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
        deleted_at: row.get(20)?,
        sync_state: row.get(21)?,
        server_id: row.get(22)?,
        tags: Vec::new(),
        knowledge: Vec::new(),
        issues: Vec::new(),
        assets: Vec::new(),
        thumb: None,
    })
}

/// hydrate 用的三条语句：列表里一次 prepare、逐题复用，省掉每题三次编译。
struct HydrateStatements<'a> {
    tags: rusqlite::Statement<'a>,
    knowledge: rusqlite::Statement<'a>,
    assets: rusqlite::Statement<'a>,
}

impl<'a> HydrateStatements<'a> {
    fn prepare(conn: &'a Connection) -> Result<Self, String> {
        Ok(Self {
            // 只把自由标签算进 tags；旧库可能残留其它 kind 的标签，不混进来。
            tags: conn
                .prepare("select tag from question_tag where question_id = ?1 and kind = 'FREE' order by tag")
                .map_err(|e| e.to_string())?,
            knowledge: conn
                .prepare(
                    "select kp.id, kp.name, ifnull(ch.name, '')
                     from question_knowledge qk join knowledge_point kp on kp.id = qk.knowledge_point_id
                     left join chapter ch on ch.id = kp.chapter_id
                     where qk.question_id = ?1 order by kp.name",
                )
                .map_err(|e| e.to_string())?,
            assets: conn
                .prepare(
                    "select id, kind, content_type, width, height from question_asset
                     where question_id = ?1 order by sort, created_at",
                )
                .map_err(|e| e.to_string())?,
        })
    }
}

fn hydrate_with(stmts: &mut HydrateStatements<'_>, question: &mut Question) -> Result<(), String> {
    let tags: Vec<String> = stmts
        .tags
        .query_map([&question.id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    question.tags = tags;

    let points: Vec<(String, String, String)> = stmts
        .knowledge
        .query_map([&question.id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    question.knowledge = points
        .into_iter()
        .map(|(id, name, chapter)| KnowledgeRef {
            id,
            name,
            path: chapter,
        })
        .collect();

    let assets: Vec<Asset> = stmts
        .assets
        .query_map([&question.id], |row| {
            let id: String = row.get(0)?;
            Ok(Asset {
                url: format!("nbasset:{id}"),
                id,
                kind: row.get(1)?,
                content_type: row.get(2)?,
                width: row.get(3)?,
                height: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    // 缩略图只认题干 / 选项里用到的图；截图不进题库，也不当封面。
    question.thumb = assets
        .iter()
        .find(|asset| asset.kind == "FIGURE")
        .map(|asset| asset.id.clone());
    question.assets = assets;
    Ok(())
}

fn hydrate(conn: &Connection, question: &mut Question) -> Result<(), String> {
    let mut stmts = HydrateStatements::prepare(conn)?;
    hydrate_with(&mut stmts, question)
}

pub fn get(conn: &Connection, id: &str) -> Result<Question, String> {
    let mut question = conn
        .query_row(&format!("{SELECT} where q.id = ?1"), [id], map_question)
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "这道题不在了，可能已被删除".to_string())?;
    hydrate(conn, &mut question)?;
    attach_issues(conn, &mut question)?;
    Ok(question)
}

/// 草稿才需要「还缺什么」：正式入库要求答案、解析、知识点三项齐全。
/// 只给草稿算，正式题与回收站不显示，省掉列表里的重复查询。
fn attach_issues(conn: &Connection, question: &mut Question) -> Result<(), String> {
    if !question.needs_review && question.status != "DRAFT" {
        return Ok(());
    }
    if let Some(issues) = publication_issues(conn, &question.id)? {
        question.issues = issues;
    }
    Ok(())
}

pub fn list(conn: &Connection, query: &QuestionQuery) -> Result<Page<Question>, String> {
    let trashed = query.trashed.unwrap_or(false);
    let size = query.size.unwrap_or(30).clamp(1, 200);
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * size;

    let mut where_sql = String::from(" where ");
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();
    if trashed {
        // 回收站只看已删除的题，忽略其它列表标记。
        where_sql.push_str("q.deleted_at is not null");
    } else {
        // 四个列表互斥：草稿箱 / 驳回列表 / 题库列表，默认题库列表不掺草稿与驳回。
        where_sql.push_str("q.deleted_at is null and ");
        if query.needs_review == Some(true) {
            where_sql.push_str("q.needs_review = 1");
        } else if query.rejected == Some(true) {
            where_sql.push_str(
                "q.needs_review = 0 and q.review_note is not null and trim(q.review_note) <> ''",
            );
        } else {
            where_sql.push_str("q.status = 'PUBLISHED' and q.needs_review = 0 and (q.review_note is null or trim(q.review_note) = '')");
        }
    }

    if let Some(subject) = query
        .subject_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        where_sql.push_str(" and q.subject_id = ?");
        params.push(Box::new(subject.to_string()));
    }
    if let Some(kind) = query.kind.as_deref().filter(|value| !value.is_empty()) {
        where_sql.push_str(" and q.type = ?");
        params.push(Box::new(kind.to_string()));
    }
    if let Some(difficulty) = query.difficulty {
        where_sql.push_str(" and q.difficulty = ?");
        params.push(Box::new(difficulty));
    }
    if let Some(kp) = query
        .knowledge_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        where_sql.push_str(
            " and exists (select 1 from question_knowledge qk where qk.question_id = q.id and qk.knowledge_point_id = ?)",
        );
        params.push(Box::new(kp.to_string()));
    }
    if let Some(tag) = query.tag.as_deref().filter(|value| !value.is_empty()) {
        where_sql.push_str(
            " and exists (select 1 from question_tag t where t.question_id = q.id and t.tag = ?)",
        );
        params.push(Box::new(tag.to_string()));
    }
    if let Some(source) = query.source.as_deref().filter(|value| !value.is_empty()) {
        where_sql.push_str(" and q.source = ?");
        params.push(Box::new(source.to_string()));
    }
    if !trashed && query.uncategorized == Some(true) {
        // 批量归类：只挑没挂任何知识点的题，和上面的草稿/驳回条件叠加。
        where_sql.push_str(
            " and not exists (select 1 from question_knowledge qk where qk.question_id = q.id)",
        );
    }
    if let Some(keyword) = query.keyword.as_deref() {
        let trimmed = keyword.trim();
        if !trimmed.is_empty() {
            if trimmed.chars().count() >= 3 && db::search_mode(conn) == "trigram" {
                where_sql.push_str(
                    " and q.id in (select question_id from question_fts where question_fts match ?)",
                );
                params.push(Box::new(format!("\"{}\"", trimmed.replace('"', "\"\""))));
            } else {
                where_sql.push_str(
                    " and (q.stem like ? or q.answer like ? or q.analysis like ? or exists (
                        select 1 from question_tag t where t.question_id = q.id and t.tag like ?))",
                );
                let pattern = format!("%{trimmed}%");
                for _ in 0..4 {
                    params.push(Box::new(pattern.clone()));
                }
            }
        }
    }

    let order = match query.sort.as_deref().unwrap_or("updated_desc") {
        "created_desc" => "q.created_at desc",
        "difficulty_asc" => "q.difficulty asc, q.updated_at desc",
        "difficulty_desc" => "q.difficulty desc, q.updated_at desc",
        _ => "q.updated_at desc",
    };

    let total: i64 = conn
        .query_row(
            &format!("select count(*) from question q{where_sql}"),
            params_from_iter(params.iter().map(|value| &**value as &dyn ToSql)),
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let sql = format!("{SELECT}{where_sql} order by {order} limit ? offset ?");
    let mut all = params;
    all.push(Box::new(size));
    all.push(Box::new(offset));
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut items: Vec<Question> = stmt
        .query_map(
            params_from_iter(all.iter().map(|value| &**value as &dyn ToSql)),
            map_question,
        )
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    let mut stmts = HydrateStatements::prepare(conn)?;
    for item in &mut items {
        hydrate_with(&mut stmts, item)?;
        // attach_issues 内部已经提前返回；这里连调用都省掉，别让正式题白查一遍库。
        if item.needs_review || item.status == "DRAFT" {
            attach_issues(conn, item)?;
        }
    }
    Ok(Page {
        items,
        total,
        page,
        size,
    })
}

/// 题库里出现过的来源网址及题量：给题库的「来源」筛选下拉用。
pub fn sources(conn: &Connection) -> Result<Vec<SourceCount>, String> {
    let mut stmt = conn
        .prepare(
            "select ifnull(source, '') as s, count(*) as n from question
             where deleted_at is null and ifnull(source, '') <> ''
             group by s order by n desc, s",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(SourceCount {
                source: row.get(0)?,
                count: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn stats(conn: &Connection) -> Result<Stats, String> {
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap_or(0) };
    let total = count("select count(*) from question where deleted_at is null");
    let draft =
        count("select count(*) from question where deleted_at is null and status = 'DRAFT'");
    let needs_review =
        count("select count(*) from question where deleted_at is null and needs_review = 1");
    let rejected = count(
        "select count(*) from question where deleted_at is null and needs_review = 0
         and review_note is not null and trim(review_note) <> ''",
    );
    let trashed = count("select count(*) from question where deleted_at is not null");
    let with_images = count(
        "select count(distinct question_id) from question_asset where question_id in
         (select id from question where deleted_at is null)",
    );
    let updated_at: Option<String> = conn
        .query_row(
            "select max(updated_at) from question where deleted_at is null",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();

    let mut stmt = conn
        .prepare(
            "select s.id, s.name,
                    (select count(*) from question q where q.subject_id = s.id and q.deleted_at is null)
             from subject s order by s.sort, s.name",
        )
        .map_err(|e| e.to_string())?;
    let subjects: Vec<SubjectStat> = stmt
        .query_map([], |row| {
            Ok(SubjectStat {
                id: row.get(0)?,
                name: row.get(1)?,
                count: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    Ok(Stats {
        total,
        draft,
        needs_review,
        rejected,
        trashed,
        with_images,
        subjects,
        updated_at,
    })
}

fn replace_links(conn: &Connection, id: &str, input: &QuestionInput) -> Result<(), String> {
    conn.execute("delete from question_tag where question_id = ?1", [id])
        .map_err(|e| e.to_string())?;
    for tag in input.tags.iter().take(12) {
        let tag = tag.trim();
        if tag.is_empty() {
            continue;
        }
        conn.execute(
            "insert or ignore into question_tag (question_id, tag, kind) values (?1, ?2, 'FREE')",
            params![id, tag],
        )
        .map_err(|e| e.to_string())?;
    }

    conn.execute(
        "delete from question_knowledge where question_id = ?1",
        [id],
    )
    .map_err(|e| e.to_string())?;
    for point in input.knowledge_ids.iter().take(6) {
        let exists: Option<i64> = conn
            .query_row(
                "select 1 from knowledge_point where id = ?1",
                [point],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if exists.is_some() {
            conn.execute(
                "insert or ignore into question_knowledge (question_id, knowledge_point_id) values (?1, ?2)",
                params![id, point],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn validate(input: &QuestionInput) -> Result<(), String> {
    if input.stem.trim().is_empty() {
        return Err("题干不能为空".into());
    }
    let kind = text::normalize_type(&input.kind)
        .ok_or_else(|| "题型需要是单选 / 多选 / 填空 / 解答之一".to_string())?;
    if (kind == "SINGLE" || kind == "MULTI") && input.options.len() < 2 {
        return Err("选择题至少要两个选项".into());
    }
    if input.options.len() > 7 {
        return Err("选项最多 7 个".into());
    }
    // 草稿箱是半成品的落脚处：识别缺答案、老师还没补，都得能存下来。
    // 只有正式题目（NORMAL 等）才强制要答案。
    let draft = matches!(input.status.as_deref(), Some("DRAFT"));
    if input.answer.trim().is_empty() && !draft {
        return Err("答案不能为空".into());
    }
    if !(1..=5).contains(&input.difficulty) {
        return Err("难度需要是 1–5".into());
    }
    Ok(())
}

/// 选择题答案只接受实际选项字母及常见分隔符；空答案由正式题门槛单独报告。
pub(crate) fn choice_answer_issue(
    kind: &str,
    options: &[OptionItem],
    answer: &str,
) -> Option<String> {
    let raw = answer.trim();
    let kind = text::normalize_type(kind).unwrap_or(kind);
    if raw.is_empty() || (kind != "SINGLE" && kind != "MULTI") {
        return None;
    }
    let valid_syntax = raw.chars().all(|ch| {
        ch.is_ascii_alphabetic()
            || ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，'
                    | '、'
                    | ';'
                    | '；'
                    | '/'
                    | '|'
                    | '.'
                    | '．'
                    | '。'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '['
                    | ']'
                    | '【'
                    | '】'
            )
    });
    let letters: Vec<String> = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .map(|ch| ch.to_ascii_uppercase().to_string())
        .collect();
    let keys: std::collections::HashSet<String> = options
        .iter()
        .map(|option| option.key.trim().to_ascii_uppercase())
        .collect();
    if !valid_syntax || letters.is_empty() || letters.iter().any(|letter| !keys.contains(letter)) {
        return Some("选择题答案只能填实际选项的字母".into());
    }
    if kind == "SINGLE" && letters.len() != 1 {
        return Some("单选题答案只能是一个选项字母".into());
    }
    None
}

/// 从数据库读回实际字段与关联，避免“请求里带了知识点 id、落库时却被过滤”也发布成功。
/// 回收站或不存在的 id 返回 None，复核时继续保持跳过语义。
fn publication_issues(conn: &Connection, id: &str) -> Result<Option<Vec<String>>, String> {
    let row = conn
        .query_row(
            "select type, answer, analysis, options_json from question where id = ?1 and deleted_at is null",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((kind, answer, analysis, options_json)) = row else {
        return Ok(None);
    };
    let options: Vec<OptionItem> = serde_json::from_str(&options_json).unwrap_or_default();
    let knowledge: i64 = conn
        .query_row(
            "select count(*) from question_knowledge qk join knowledge_point kp on kp.id = qk.knowledge_point_id where qk.question_id = ?1",
            [id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let mut issues = Vec::new();
    if answer.trim().is_empty() {
        issues.push("缺答案".to_string());
    }
    if analysis.trim().is_empty() {
        issues.push("缺解析".to_string());
    }
    if knowledge == 0 {
        issues.push("缺知识点".to_string());
    }
    if let Some(issue) = choice_answer_issue(&kind, &options, &answer) {
        issues.push(issue);
    }
    Ok(Some(issues))
}

pub fn save(conn: &mut Connection, input: &QuestionInput) -> Result<String, String> {
    validate(input)?;
    let kind = text::normalize_type(&input.kind).unwrap_or("SOLVED");
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = text::now();
    let normalized = text::normalize_stem(&input.stem);
    let options_json = serde_json::to_string(&input.options).map_err(|e| e.to_string())?;
    let status = input.status.clone().unwrap_or_else(|| "DRAFT".into());
    let origin = input.origin.clone().unwrap_or_else(|| "MANUAL".into());
    let needs_review = input.needs_review.unwrap_or(false);

    let exists: Option<String> = conn
        .query_row("select id from question where id = ?1", [&id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if exists.is_some() {
        tx.execute(
            "update question set subject_id = ?2, type = ?3, stem = ?4, options_json = ?5,
                answer = ?6, analysis = ?7, difficulty = ?8, stem_normalized = ?9,
                source = ?10, year = ?11, status = ?12, needs_review = ?13,
                review_note = ?14, updated_at = ?15,
                sync_state = case when server_id is null then 'LOCAL' else 'PENDING' end
             where id = ?1",
            params![
                id,
                input.subject_id,
                kind,
                input.stem,
                options_json,
                input.answer,
                input.analysis,
                input.difficulty,
                normalized,
                input.source,
                input.year,
                status,
                needs_review as i64,
                input.review_note,
                now
            ],
        )
        .map_err(|e| e.to_string())?;
    } else {
        tx.execute(
            "insert into question (id, subject_id, type, stem, options_json, answer, analysis,
                difficulty, stem_normalized, source, year, status, origin, needs_review,
                review_note, created_at, updated_at, sync_state)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16, 'LOCAL')",
            params![
                id,
                input.subject_id,
                kind,
                input.stem,
                options_json,
                input.answer,
                input.analysis,
                input.difficulty,
                normalized,
                input.source,
                input.year,
                status,
                origin,
                needs_review as i64,
                input.review_note,
                now
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    replace_links(&tx, &id, input)?;
    if status != "DRAFT" {
        if let Some(issues) = publication_issues(&tx, &id)? {
            if !issues.is_empty() {
                return Err(format!("不能正式入库：{}", issues.join("、")));
            }
        }
    }
    db::index_question(&tx, &id)?;
    let op = if exists.is_some() { "UPDATE" } else { "CREATE" };
    sync::enqueue(&tx, "question", &id, op)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn soft_delete(conn: &mut Connection, ids: &[String]) -> Result<usize, String> {
    let now = text::now();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut changed = 0;
    for id in ids {
        changed += tx
            .execute(
                "update question set deleted_at = ?2, updated_at = ?2,
                    sync_state = case when server_id is null then 'LOCAL' else 'PENDING' end
                 where id = ?1 and deleted_at is null",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
        sync::enqueue(&tx, "question", id, "DELETE")?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

pub fn restore(conn: &mut Connection, ids: &[String]) -> Result<usize, String> {
    let now = text::now();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut changed = 0;
    for id in ids {
        changed += tx
            .execute(
                "update question set deleted_at = null, updated_at = ?2,
                    sync_state = case when server_id is null then 'LOCAL' else 'PENDING' end
                 where id = ?1 and deleted_at is not null",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
        sync::enqueue(&tx, "question", id, "UPDATE")?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

/// 复核一批题：approve 通过并发布，reject 驳回（写明原因），draft 把驳回的题转回草稿。
/// 前两者清掉 needs_review 标记；驳回时把原因写进 review_note，转回草稿时清空。
/// 已删除（回收站）的题跳过，不参与复核。
pub fn review(
    conn: &mut Connection,
    ids: &[String],
    action: &str,
    note: Option<&str>,
) -> Result<usize, String> {
    let (status, needs_review, keep_note) = match action {
        "approve" => ("PUBLISHED", 0_i64, false),
        "reject" => ("DRAFT", 0, true),
        "draft" => ("DRAFT", 1, false),
        other => return Err(format!("未知的复核动作：{other}")),
    };
    if action == "reject"
        && note
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Err("驳回需要填写原因".into());
    }
    let now = text::now();
    let review_note = if keep_note { note } else { None };
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut changed = 0;
    for id in ids {
        if action == "approve" {
            let Some(issues) = publication_issues(&tx, id)? else {
                continue;
            };
            if !issues.is_empty() {
                return Err(format!(
                    "题目 #{} 不能入库：{}",
                    short_id(id),
                    issues.join("、")
                ));
            }
        }
        let n = tx
            .execute(
                "update question set status = ?2, needs_review = ?3, review_note = ?4,
                    updated_at = ?5,
                    sync_state = case when server_id is null then 'LOCAL' else 'PENDING' end
                 where id = ?1 and deleted_at is null",
                params![id, status, needs_review, review_note, now],
            )
            .map_err(|e| e.to_string())?;
        changed += n;
        if n > 0 {
            sync::enqueue(&tx, "question", id, "UPDATE")?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

/// 报错和列表里显示用的短编号：uuid 前 8 位，够对上题目又好念。
pub fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// 彻底删除：只会删本地文件与行，前提是这道题已经在回收站。
/// 同步过的题不允许硬删，否则服务端会再拉回来，用户会看到「删了又回来」。
pub fn purge(conn: &mut Connection, id: &str, assets_dir: &std::path::Path) -> Result<(), String> {
    let row: Option<(Option<String>, String)> = conn
        .query_row(
            "select deleted_at, sync_state from question where id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (deleted_at, sync_state) = row.ok_or_else(|| "这道题不在了".to_string())?;
    if deleted_at.is_none() {
        return Err("请先把题目移到回收站，再彻底删除".into());
    }
    if sync_state == "SYNCED" {
        return Err("这道题同步过中心题库，彻底删除会把它重新拉回来；请先在同步页停用同步".into());
    }
    conn.execute("delete from question where id = ?1", [id])
        .map_err(|e| e.to_string())?;
    db::unindex_question(conn, id);
    let dir = assets_dir.join(id);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(dir);
    }
    Ok(())
}

/// 已有指纹，导入预览时一次性取出，避免逐题查库。
pub fn fingerprints(conn: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare("select id, stem_normalized from question where deleted_at is null")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

pub fn tags(conn: &Connection) -> Result<Vec<(String, i64)>, String> {
    // 标签云要能搜、能展开，所以全量返回（按题量降序），不再截断在前 80 个。
    let mut stmt = conn
        .prepare(
            "select tag, count(*) from question_tag t
             where t.question_id in (select id from question where deleted_at is null)
             group by tag order by count(*) desc, tag asc",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

/// 记录一次「AI 自动补写答案 / 解析」，供列表徽章显示。
pub fn mark_ai_fields(
    conn: &Connection,
    id: &str,
    answer: bool,
    analysis: bool,
) -> Result<(), String> {
    conn.execute(
        "update question set ai_answer = max(ai_answer, ?2), ai_analysis = max(ai_analysis, ?3) where id = ?1",
        params![id, answer as i64, analysis as i64],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
