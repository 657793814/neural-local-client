//! 采集来源记忆与连拍队列的仓储层。
//! 只操作 capture_source / capture_queue 两张表；不碰题库，不调模型。

use crate::models::{CaptureSource, QueueItem};
use crate::text;
use rusqlite::{params, Connection, OptionalExtension};

fn now() -> String {
    text::now()
}

/// 记录一次来源：同网址累加采集次数并更新 label / last_at；返回来源 id。
pub fn remember(conn: &Connection, url: &str, label: &str) -> Result<String, String> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(String::new());
    }
    let id: Option<String> = conn
        .query_row(
            "select id from capture_source where url = ?1",
            [url],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let at = now();
    if let Some(id) = id {
        conn.execute(
            "update capture_source set label = case when ?2 = '' then label else ?2 end,
                capture_count = capture_count + 1, last_at = ?3 where id = ?1",
            params![id, label.trim(), at],
        )
        .map_err(|e| e.to_string())?;
        Ok(id)
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "insert into capture_source (id, url, label, capture_count, last_at, created_at)
             values (?1, ?2, ?3, 1, ?4, ?4)",
            params![id, url, label.trim(), at],
        )
        .map_err(|e| e.to_string())?;
        Ok(id)
    }
}

pub fn sources(conn: &Connection) -> Result<Vec<CaptureSource>, String> {
    let mut stmt = conn
        .prepare("select id, url, label, starred, capture_count, last_at from capture_source order by starred desc, last_at desc")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(CaptureSource {
                id: row.get(0)?,
                url: row.get(1)?,
                label: row.get(2)?,
                starred: row.get::<_, i64>(3)? != 0,
                capture_count: row.get(4)?,
                last_at: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn star(conn: &Connection, id: &str, on: bool) -> Result<(), String> {
    conn.execute(
        "update capture_source set starred = ?2 where id = ?1",
        params![id, if on { 1 } else { 0 }],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn forget(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("delete from capture_source where id = ?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 入队一项。来源 id / url 可为空（手工截图没有浏览来源）。
pub fn enqueue(
    conn: &Connection,
    image_path: &str,
    source_id: Option<&str>,
    source_url: &str,
) -> Result<QueueItem, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let at = now();
    conn.execute(
        "insert into capture_queue (id, image_path, source_id, source_url, status, created_at, updated_at)
         values (?1, ?2, ?3, ?4, 'PENDING', ?5, ?5)",
        params![id, image_path, source_id, source_url, at],
    )
    .map_err(|e| e.to_string())?;
    get(conn, &id)
}

pub fn get(conn: &Connection, id: &str) -> Result<QueueItem, String> {
    conn.query_row(
        "select id, image_path, ifnull(source_id,''), source_url, status, ifnull(job_id,''), error, item_count, created_at
         from capture_queue where id = ?1",
        [id],
        |row| {
            let sid: String = row.get(2)?;
            let jid: String = row.get(5)?;
            Ok(QueueItem {
                id: row.get(0)?,
                image_path: row.get(1)?,
                source_id: if sid.is_empty() { None } else { Some(sid) },
                source_url: row.get(3)?,
                status: row.get(4)?,
                job_id: if jid.is_empty() { None } else { Some(jid) },
                error: row.get(6)?,
                item_count: row.get(7)?,
                created_at: row.get(8)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

pub fn list(conn: &Connection) -> Result<Vec<QueueItem>, String> {
    let mut stmt = conn
        .prepare("select id from capture_queue order by created_at desc, rowid desc")
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(ids.iter().filter_map(|id| get(conn, id).ok()).collect())
}

pub fn mark(
    conn: &Connection,
    id: &str,
    status: &str,
    job_id: Option<&str>,
    error: &str,
) -> Result<(), String> {
    conn.execute(
        "update capture_queue set status = ?2, job_id = ?3, error = ?4, updated_at = ?5 where id = ?1",
        params![id, status, job_id.unwrap_or(""), error, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn set_item_count(conn: &Connection, id: &str, count: i64) -> Result<(), String> {
    conn.execute(
        "update capture_queue set item_count = ?2, status = 'DONE', updated_at = ?3 where id = ?1",
        params![id, count, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn retry(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute(
        "update capture_queue set status = 'PENDING', error = '', updated_at = ?2 where id = ?1",
        params![id, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute("delete from capture_queue where id = ?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn clear_done(conn: &Connection) -> Result<usize, String> {
    Ok(conn
        .execute(
            "delete from capture_queue where status in ('DONE','FAILED')",
            [],
        )
        .map_err(|e| e.to_string())?)
}
