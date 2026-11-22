//! SQLite 连接、迁移、内置教材目录与全文检索。

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_init.sql"),
    include_str!("../migrations/0002_local_user.sql"),
    include_str!("../migrations/0003_capture.sql"),
    include_str!("../migrations/0004_teaching.sql"),
    include_str!("../migrations/0005_insights.sql"),
    include_str!("../migrations/0006_courseware.sql"),
    include_str!("../migrations/0007_grading_photos.sql"),
    include_str!("../migrations/0008_llm_model_has_key.sql"),
    include_str!("../migrations/0009_drop_capture_assets.sql"),
    include_str!("../migrations/0010_merge_exam_points.sql"),
    include_str!("../migrations/0011_center_submission.sql"),
    include_str!("../migrations/0012_papers.sql"),
    include_str!("../migrations/0013_scores.sql"),
    include_str!("../migrations/0014_courseware_import.sql"),
    include_str!("../migrations/0015_diagram_purpose.sql"),
    include_str!("../migrations/0016_knowledge_expand.sql"),
];
const SEED_CURRICULUM: &str = include_str!("../seed/curriculum.json");

/// 当前库结构版本 = 迁移文件的条数，与 `pragma user_version` 一致。
/// 测试断言「迁移全部跑完」时用它，新增迁移就不用再回头改一堆写死的数字。
// 只在 #[cfg(test)] 的 flow_tests 里被引用；release 构建里抑制 dead_code 噪音。
#[cfg_attr(not(test), allow(dead_code))]
pub const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

pub fn open(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("打不开本地数据库：{e}"))?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

pub fn migrate(conn: &mut Connection) -> Result<(), String> {
    let version: i64 = conn
        .query_row("pragma user_version", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let target = (index + 1) as i64;
        if version >= target {
            continue;
        }
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute_batch(sql)
            .map_err(|e| format!("数据库迁移失败：{e}"))?;
        tx.pragma_update(None, "user_version", target)
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn meta_get(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("select value from meta where key = ?1", [key], |row| {
        row.get(0)
    })
    .optional()
    .ok()
    .flatten()
}

pub fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "insert into meta (key, value) values (?1, ?2)
         on conflict (key) do update set value = excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 内置教材目录只在库为空时装一次；用户之后自建的章节不会被覆盖。
pub fn seed_curriculum(conn: &mut Connection) -> Result<(), String> {
    let count: i64 = conn
        .query_row("select count(*) from subject", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if count > 0 {
        return Ok(());
    }
    let seed: serde_json::Value =
        serde_json::from_str(SEED_CURRICULUM).map_err(|e| format!("内置教材目录损坏：{e}"))?;
    let version = seed
        .get("version")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if let Some(subjects) = seed.get("subjects").and_then(|value| value.as_array()) {
        for (si, subject) in subjects.iter().enumerate() {
            let code = subject
                .get("code")
                .and_then(|value| value.as_str())
                .unwrap_or("SUBJ");
            let name = subject
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("未命名学科");
            let subject_id = format!("subj-{code}");
            tx.execute(
                "insert or ignore into subject (id, code, name, sort) values (?1, ?2, ?3, ?4)",
                params![subject_id, code, name, si as i64],
            )
            .map_err(|e| e.to_string())?;

            let textbooks = subject
                .get("textbooks")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            for (ti, textbook) in textbooks.iter().enumerate() {
                let tb_name = textbook
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("未命名教材");
                let grade = textbook.get("grade").and_then(|value| value.as_str());
                let textbook_id = format!("tb-{code}-{}", ti + 1);
                tx.execute(
                    "insert or ignore into textbook (id, subject_id, name, grade, sort)
                     values (?1, ?2, ?3, ?4, ?5)",
                    params![textbook_id, subject_id, tb_name, grade, ti as i64],
                )
                .map_err(|e| e.to_string())?;

                let chapters = textbook
                    .get("chapters")
                    .and_then(|value| value.as_array())
                    .cloned()
                    .unwrap_or_default();
                for (ci, chapter) in chapters.iter().enumerate() {
                    let ch_name = chapter
                        .get("name")
                        .and_then(|value| value.as_str())
                        .unwrap_or("未命名章节");
                    let chapter_id = format!("ch-{code}-{}-{}", ti + 1, ci + 1);
                    tx.execute(
                        "insert or ignore into chapter (id, textbook_id, name, sort) values (?1, ?2, ?3, ?4)",
                        params![chapter_id, textbook_id, ch_name, ci as i64],
                    )
                    .map_err(|e| e.to_string())?;

                    let points = chapter
                        .get("points")
                        .and_then(|value| value.as_array())
                        .cloned()
                        .unwrap_or_default();
                    for (pi, point) in points.iter().enumerate() {
                        let Some(point_name) = point.as_str() else {
                            continue;
                        };
                        let point_id = format!("kp-{code}-{}-{}-{}", ti + 1, ci + 1, pi + 1);
                        let point_code = format!("{code}-{}-{}-{:02}", ti + 1, ci + 1, pi + 1);
                        tx.execute(
                            "insert or ignore into knowledge_point
                                (id, subject_id, chapter_id, code, name, sort)
                             values (?1, ?2, ?3, ?4, ?5, ?6)",
                            params![
                                point_id, subject_id, chapter_id, point_code, point_name, pi as i64
                            ],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                }
            }
        }
    }
    tx.execute(
        "insert or replace into meta (key, value) values ('curriculum.version', ?1)",
        params![version],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// FTS5 优先用 trigram（中文按字切，能命中任意子串）；建不出来就退回 LIKE。
pub fn ensure_search(conn: &Connection) -> Result<String, String> {
    if let Some(mode) = meta_get(conn, "search.mode") {
        if mode == "trigram" || mode == "like" {
            return Ok(mode);
        }
    }
    let created = conn
        .execute_batch(
            "create virtual table if not exists question_fts
             using fts5(question_id UNINDEXED, body, tokenize='trigram')",
        )
        .is_ok();
    let mode = if created { "trigram" } else { "like" };
    meta_set(conn, "search.mode", mode)?;
    if created {
        reindex(conn)?;
    }
    Ok(mode.to_string())
}

pub fn search_mode(conn: &Connection) -> String {
    meta_get(conn, "search.mode").unwrap_or_else(|| "like".to_string())
}

pub fn reindex(conn: &Connection) -> Result<(), String> {
    if search_mode(conn) != "trigram" {
        return Ok(());
    }
    conn.execute("delete from question_fts", [])
        .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "select q.id,
                    q.stem || ' ' || q.answer || ' ' || q.analysis || ' ' ||
                    ifnull((select group_concat(t.tag, ' ') from question_tag t where t.question_id = q.id), '')
             from question q",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    for (id, body) in rows {
        conn.execute(
            "insert into question_fts (question_id, body) values (?1, ?2)",
            params![id, body],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn index_question(conn: &Connection, id: &str) -> Result<(), String> {
    if search_mode(conn) != "trigram" {
        return Ok(());
    }
    conn.execute("delete from question_fts where question_id = ?1", [id])
        .map_err(|e| e.to_string())?;
    let body: Option<String> = conn
        .query_row(
            "select q.stem || ' ' || q.answer || ' ' || q.analysis || ' ' ||
                    ifnull((select group_concat(t.tag, ' ') from question_tag t where t.question_id = q.id), '')
             from question q where q.id = ?1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(body) = body {
        conn.execute(
            "insert into question_fts (question_id, body) values (?1, ?2)",
            params![id, body],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn unindex_question(conn: &Connection, id: &str) {
    if search_mode(conn) == "trigram" {
        let _ = conn.execute("delete from question_fts where question_id = ?1", [id]);
    }
}

/// 学科 / 教材 / 章 / 知识点树，一次给前端做选择器。
pub fn curriculum_tree(
    conn: &Connection,
    subject_id: Option<&str>,
) -> Result<serde_json::Value, String> {
    let mut subjects_stmt = conn
        .prepare("select id, code, name from subject order by sort, name")
        .map_err(|e| e.to_string())?;
    let subjects: Vec<(String, Option<String>, String)> = subjects_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let mut out = Vec::new();
    for (sid, code, sname) in subjects {
        if let Some(filter) = subject_id {
            if filter != sid {
                continue;
            }
        }
        let mut tb_stmt = conn
            .prepare(
                "select id, name, grade from textbook where subject_id = ?1 order by sort, name",
            )
            .map_err(|e| e.to_string())?;
        let textbooks: Vec<(String, String, Option<String>)> = tb_stmt
            .query_map([&sid], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();

        let mut tb_json = Vec::new();
        for (tid, tname, grade) in textbooks {
            let mut ch_stmt = conn
                .prepare("select id, name from chapter where textbook_id = ?1 and parent_id is null order by sort, name")
                .map_err(|e| e.to_string())?;
            let chapters: Vec<(String, String)> = ch_stmt
                .query_map([&tid], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .collect();

            let mut ch_json = Vec::new();
            for (cid, cname) in chapters {
                let mut kp_stmt = conn
                    .prepare("select id, name, code from knowledge_point where chapter_id = ?1 order by sort, name")
                    .map_err(|e| e.to_string())?;
                let points: Vec<(String, String, Option<String>)> = kp_stmt
                    .query_map([&cid], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                    .map_err(|e| e.to_string())?
                    .filter_map(Result::ok)
                    .collect();
                ch_json.push(serde_json::json!({
                    "id": cid,
                    "name": cname,
                    "points": points.iter().map(|(id, name, code)| serde_json::json!({
                        "id": id, "name": name, "code": code,
                        "path": format!("{sname} / {tname} / {cname} / {name}")
                    })).collect::<Vec<_>>()
                }));
            }
            tb_json.push(serde_json::json!({
                "id": tid, "name": tname, "grade": grade, "chapters": ch_json
            }));
        }
        out.push(serde_json::json!({
            "id": sid, "code": code, "name": sname, "textbooks": tb_json
        }));
    }
    Ok(serde_json::Value::Array(out))
}

/// 新建 / 复用「词表里没有」的知识点。
///
/// 内置词表再全也有覆盖不到的时候（新题型、校本专题、跨章节综合），所以用户
/// 必须能直接敲一个名字进去。自建点挂在每个学科自动建的「自建知识点 / 自定义」
/// 目录下，跟教材目录一样出现在知识树里——归类、统计、导出都不用特判。
///
/// 同名（忽略首尾空格）已存在时直接复用，避免同一个知识点被反复建出来。
pub fn ensure_knowledge_point(
    conn: &Connection,
    subject_id: &str,
    name: &str,
) -> Result<String, String> {
    ensure_point_with_origin(conn, subject_id, name, "custom")
}

/// 归类 / 识别链路里「词表没有但像个知识点」的名字：按更严的规则自动补一条，
/// 记为 ai，方便以后在自建词里复核清理。不像知识点的一律不收，避免污染词表。
pub fn ensure_ai_point(conn: &Connection, subject_id: &str, name: &str) -> Option<String> {
    let name = name.trim();
    if !acceptable_new_point(name) {
        return None;
    }
    ensure_point_with_origin(conn, subject_id, name, "ai").ok()
}

fn acceptable_new_point(name: &str) -> bool {
    let length = name.chars().count();
    if !(2..=20).contains(&length) {
        return false;
    }
    if !name.chars().any(|ch| ch.is_alphabetic()) {
        return false;
    }
    if name.contains("http") || name.contains('$') || name.contains('=') {
        return false;
    }
    // 「求…」「计算…」是任务不是概念；带标点的多半是把题干句子切出来了。
    const TASK_PREFIX: [&str; 6] = ["求", "计算", "证明", "解方", "画出", "判断"];
    if TASK_PREFIX.iter().any(|prefix| name.starts_with(prefix)) {
        return false;
    }
    const BAD: [char; 12] = ['？', '?', '！', '!', '，', ',', '。', '；', ';', '：', ':', '\n'];
    !name.chars().any(|ch| BAD.contains(&ch))
}

fn ensure_point_with_origin(
    conn: &Connection,
    subject_id: &str,
    name: &str,
    origin: &str,
) -> Result<String, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("知识点名称不能为空".into());
    }
    if name.chars().count() > 30 {
        return Err("知识点名称太长，控制在 30 字以内".into());
    }
    if let Some(id) = find_point_by_name(conn, Some(subject_id), &name) {
        return Ok(id);
    }
    // 跨学科同名也算同一个知识点，不再造一份，免得统计时一个概念分裂成两条。
    if let Some(id) = find_point_by_name(conn, None, &name) {
        return Ok(id);
    }

    let exists: Option<String> = conn
        .query_row("select name from subject where id = ?1", [subject_id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err("这个学科不存在，先选好学科再建知识点".into());
    }

    let textbook_id = match conn
        .query_row(
            "select id from textbook where subject_id = ?1 and name = '自建知识点' limit 1",
            [subject_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    {
        Some(id) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "insert into textbook (id, subject_id, name, grade, sort) values (?1, ?2, '自建知识点', null, 900)",
                params![id, subject_id],
            )
            .map_err(|e| e.to_string())?;
            id
        }
    };

    let chapter_id = match conn
        .query_row(
            "select id from chapter where textbook_id = ?1 and name = '自定义' limit 1",
            [&textbook_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    {
        Some(id) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "insert into chapter (id, textbook_id, parent_id, name, sort) values (?1, ?2, null, '自定义', 900)",
                params![id, textbook_id],
            )
            .map_err(|e| e.to_string())?;
            id
        }
    };

    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "insert into knowledge_point (id, subject_id, chapter_id, name, sort, origin)
         values (?1, ?2, ?3, ?4, 900, ?5)",
        params![id, subject_id, chapter_id, name, origin],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// 按名字找知识点：给了学科就学科内找，没给就全局找（用于跨学科复用）。
fn find_point_by_name(conn: &Connection, subject_id: Option<&str>, name: &str) -> Option<String> {
    let sql = match subject_id {
        Some(_) => "select id from knowledge_point where subject_id = ?1 and trim(name) = ?2 limit 1",
        None => "select id from knowledge_point where trim(name) = ?1 limit 1",
    };
    let id: Option<String> = match subject_id {
        Some(subject) => conn
            .query_row(sql, params![subject, name], |row| row.get(0))
            .optional(),
        None => conn.query_row(sql, [name], |row| row.get(0)).optional(),
    }
    .ok()
    .flatten();
    id
}

/// `knowledgePath` 归一化后逐级比对：允许只写知识点名，也允许写完整路径。
pub fn resolve_knowledge_path(
    conn: &Connection,
    subject_id: &str,
    path: &[String],
) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let target = path.last()?.trim().to_string();
    let mut stmt = conn
        .prepare(
            "select kp.id, kp.name, ch.name, tb.name
             from knowledge_point kp
             left join chapter ch on ch.id = kp.chapter_id
             left join textbook tb on tb.id = ch.textbook_id
             where kp.subject_id = ?1",
        )
        .ok()?;
    let rows: Vec<(String, String, Option<String>, Option<String>)> = stmt
        .query_map([subject_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .ok()?
        .filter_map(Result::ok)
        .collect();

    let loose = |a: &str, b: &str| a.trim() == b.trim();
    // 完整路径优先：知识点名相同但章节不同时不会串。
    if path.len() >= 3 {
        let chapter = &path[path.len() - 2];
        let textbook = &path[path.len() - 3];
        for (id, name, ch, tb) in &rows {
            if loose(name, &target)
                && ch
                    .as_deref()
                    .map(|value| loose(value, chapter))
                    .unwrap_or(false)
            {
                if path.len() == 3
                    || tb
                        .as_deref()
                        .map(|value| loose(value, textbook))
                        .unwrap_or(false)
                {
                    return Some(id.clone());
                }
            }
        }
    }
    rows.into_iter()
        .find(|(_, name, _, _)| loose(name, &target))
        .map(|(id, _, _, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_and_seed() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&mut conn).unwrap();
        seed_curriculum(&mut conn).unwrap();
        let subjects: i64 = conn
            .query_row("select count(*) from subject", [], |r| r.get(0))
            .unwrap();
        assert!(subjects >= 5);
        let points: i64 = conn
            .query_row("select count(*) from knowledge_point", [], |r| r.get(0))
            .unwrap();
        assert!(points > 50);
        assert_eq!(ensure_search(&conn).unwrap(), "trigram");
    }

    /// 用户可以随手建一个词表里没有的知识点，且同名不会建第二份。
    #[test]
    fn custom_point_is_created_once_and_shows_up_in_tree() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&mut conn).unwrap();
        seed_curriculum(&mut conn).unwrap();
        let subject: String = conn
            .query_row("select id from subject order by sort limit 1", [], |row| {
                row.get(0)
            })
            .unwrap();

        let first = ensure_knowledge_point(&conn, &subject, " 校本专题·函数建模 ").unwrap();
        let again = ensure_knowledge_point(&conn, &subject, "校本专题·函数建模").unwrap();
        assert_eq!(first, again, "同名知识点必须复用，不能重复建");

        let (name, origin): (String, String) = conn
            .query_row(
                "select name, origin from knowledge_point where id = ?1",
                [&first],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(name, "校本专题·函数建模");
        assert_eq!(origin, "custom");

        let tree = curriculum_tree(&conn, Some(&subject)).unwrap().to_string();
        assert!(tree.contains("自建知识点"), "自建目录要出现在知识树里");
        assert!(tree.contains("校本专题·函数建模"));
    }

    /// AI 归类时补词要克制：任务式、句子式的名字不收。
    #[test]
    fn ai_point_creation_rejects_task_like_names() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&mut conn).unwrap();
        seed_curriculum(&mut conn).unwrap();
        let subject: String = conn
            .query_row("select id from subject order by sort limit 1", [], |row| {
                row.get(0)
            })
            .unwrap();

        assert!(ensure_ai_point(&conn, &subject, "求函数的解析式").is_none());
        assert!(ensure_ai_point(&conn, &subject, "如图，求阴影面积").is_none());
        assert!(ensure_ai_point(&conn, &subject, "A").is_none());
        let created = ensure_ai_point(&conn, &subject, "抽象函数与对称性").unwrap();
        let origin: String = conn
            .query_row("select origin from knowledge_point where id = ?1", [&created], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(origin, "ai");
    }

    #[test]
    fn resolve_path_by_name() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        seed_curriculum(&mut conn).unwrap();
        let id = resolve_knowledge_path(
            &conn,
            "subj-MATH",
            &[
                "初中数学 · 七年级上册".into(),
                "第一章 有理数".into(),
                "正数和负数".into(),
            ],
        );
        assert_eq!(id.as_deref(), Some("kp-MATH-1-1-1"));
    }
}
