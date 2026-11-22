//! 同步：待推送队列、账号登录、推送与拉取。
//!
//! 本地永远是 source of truth：同步失败只影响「上传」这一步，题库照常用。
//! 服务端接口见方案 §11（`/api/desktop/**`）。服务端未部署时这里会返回明确的错误，
//! 页面据此提示「中心系统还没开同步」，而不是假装成功。

use crate::models::SyncStatus;
use crate::text;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

const TOKEN_ACCOUNT: &str = "sync-token";

fn token_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(crate::llm::SERVICE, TOKEN_ACCOUNT)
        .map_err(|e| format!("打不开系统钥匙串：{e}"))
}

pub fn token() -> Option<String> {
    token_entry().ok()?.get_password().ok()
}

pub fn set_token(value: &str) -> Result<(), String> {
    token_entry()?
        .set_password(value)
        .map_err(|e| format!("令牌保存失败：{e}"))
}

pub fn clear_token() {
    if let Ok(entry) = token_entry() {
        let _ = entry.delete_credential();
    }
}

/// 写题时排队；同步没开就不进队列，避免离线用户白攒一堆待办。
pub fn enqueue(
    conn: &Connection,
    entity_type: &str,
    entity_id: &str,
    op: &str,
) -> Result<(), String> {
    let enabled: i64 = conn
        .query_row(
            "select ifnull((select enabled from sync_account where id = 1), 0)",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if enabled == 0 {
        return Ok(());
    }
    conn.execute(
        "delete from sync_outbox where entity_type = ?1 and entity_id = ?2 and pushed_at is null",
        params![entity_type, entity_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "insert into sync_outbox (id, entity_type, entity_id, op, payload_json, created_at)
         values (?1, ?2, ?3, ?4, '{}', ?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            entity_type,
            entity_id,
            op,
            text::now()
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn status(conn: &Connection) -> Result<SyncStatus, String> {
    let account = conn
        .query_row(
            "select server_url, username, enabled, device_id, expires_at from sync_account where id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? != 0,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap_or(0) };
    let pending = count("select count(*) from sync_outbox where pushed_at is null");
    let failed = count("select count(*) from sync_outbox where pushed_at is null and attempts > 0");
    let synced =
        count("select count(*) from question where sync_state = 'SYNCED' and deleted_at is null");
    let last_error: Option<String> = conn
        .query_row(
            "select last_error from sync_outbox where last_error is not null order by created_at desc limit 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();

    let (server_url, username, enabled, device_id, expires_at) =
        account.unwrap_or((String::new(), String::new(), false, String::new(), None));
    Ok(SyncStatus {
        configured: !device_id.is_empty() && !server_url.is_empty(),
        server_url,
        username,
        enabled,
        pending,
        failed,
        synced,
        last_error,
        expires_at,
    })
}

fn base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if !trimmed.starts_with("https://") && !trimmed.starts_with("http://") {
        return Err("服务器地址需要以 https:// 开头".into());
    }
    Ok(trimmed.to_string())
}

/// 中心连接信息：服务器地址 + 设备令牌。
///
/// 所有 `/api/desktop/**` 调用（同步、教学数据拉取）都用这一份，
/// 免得每个模块自己拼「没登录」的报错文案。
pub fn connection(conn: &Connection) -> Result<(String, String), String> {
    let token = token().ok_or_else(|| "还没登录中心系统".to_string())?;
    let base: String = conn
        .query_row(
            "select server_url from sync_account where id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if base.is_empty() {
        return Err("还没登录中心系统".into());
    }
    Ok((base, token))
}

pub(crate) fn error_of(status: reqwest::StatusCode, body: &str) -> String {
    match status.as_u16() {
        401 => "账号或密码不对，或设备令牌已失效".to_string(),
        403 => "这个账号没有客户端同步权限".to_string(),
        404 => {
            "服务器没有 `/api/desktop/**` 接口，说明中心系统还没升级到支持客户端的版本".to_string()
        }
        _ => format!(
            "服务器返回 {}：{}",
            status.as_u16(),
            crate::llm::json::snippet(body)
        ),
    }
}

pub fn login(
    conn: &mut Connection,
    client: &reqwest::blocking::Client,
    server: &str,
    username: &str,
    password: &str,
    device_name: &str,
) -> Result<SyncStatus, String> {
    let base = base_url(server)?;
    if username.trim().is_empty() || password.is_empty() {
        return Err("填一下账号和密码".into());
    }
    let device_id: String = {
        let stored: Option<String> = conn
            .query_row(
                "select device_id from sync_account where id = 1 and device_id <> ''",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        match stored {
            Some(id) => id,
            None => uuid::Uuid::new_v4().to_string(),
        }
    };

    let response = client
        .post(format!("{base}/api/desktop/auth/login"))
        .timeout(std::time::Duration::from_secs(20))
        .json(&json!({
            "username": username.trim(),
            "password": password,
            "deviceId": device_id,
            "deviceName": device_name,
            "platform": std::env::consts::OS,
        }))
        .send()
        .map_err(|e| format!("连不上服务器：{e}"))?;
    let status_code = response.status();
    let body = response.text().map_err(|e| e.to_string())?;
    if !status_code.is_success() {
        return Err(error_of(status_code, &body));
    }
    let value: Value = serde_json::from_str(&body).map_err(|e| format!("服务器响应不合法：{e}"))?;
    let token = value
        .get("token")
        .and_then(|node| node.as_str())
        .ok_or_else(|| "服务器没有返回设备令牌".to_string())?;
    set_token(token)?;
    let expires_at = value
        .get("expiresAt")
        .and_then(|node| node.as_str())
        .map(|text| text.to_string());

    conn.execute(
        "insert into sync_account (id, server_url, username, device_id, device_name, token_ref, enabled, expires_at, last_login_at)
         values (1, ?1, ?2, ?3, ?4, ?5, 1, ?6, ?7)
         on conflict (id) do update set server_url = excluded.server_url, username = excluded.username,
            device_id = excluded.device_id, device_name = excluded.device_name, enabled = 1,
            expires_at = excluded.expires_at, last_login_at = excluded.last_login_at",
        params![
            base,
            username.trim(),
            device_id,
            device_name,
            TOKEN_ACCOUNT,
            expires_at,
            text::now()
        ],
    )
    .map_err(|e| e.to_string())?;
    status(conn)
}

pub fn logout(conn: &mut Connection, client: &reqwest::blocking::Client) -> Result<(), String> {
    if let Some(token) = token() {
        if let Ok((base,)) = conn.query_row(
            "select server_url from sync_account where id = 1",
            [],
            |row| Ok((row.get::<_, String>(0)?,)),
        ) {
            if !base.is_empty() {
                let _ = client
                    .post(format!("{base}/api/desktop/auth/revoke"))
                    .bearer_auth(&token)
                    .timeout(std::time::Duration::from_secs(10))
                    .send();
            }
        }
    }
    clear_token();
    conn.execute(
        "update sync_account set enabled = 0, expires_at = null where id = 1",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn question_payload(conn: &Connection, id: &str) -> Result<Option<Value>, String> {
    let row = conn
        .query_row(
            "select q.type, q.stem, q.options_json, q.answer, q.analysis, q.difficulty,
                    ifnull(s.name, ''), q.updated_at, q.deleted_at
             from question q left join subject s on s.id = q.subject_id where q.id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((kind, stem, options, answer, analysis, difficulty, subject, updated_at, deleted_at)) =
        row
    else {
        return Ok(None);
    };

    let tags: Vec<String> = conn
        .prepare("select tag from question_tag where question_id = ?1 and kind = 'FREE'")
        .and_then(|mut stmt| {
            stmt.query_map([id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();
    let knowledge: Vec<String> = conn
        .prepare(
            "select ifnull(tb.name, '') || '/' || ifnull(ch.name, '') || '/' || kp.name
             from question_knowledge qk
             join knowledge_point kp on kp.id = qk.knowledge_point_id
             left join chapter ch on ch.id = kp.chapter_id
             left join textbook tb on tb.id = ch.textbook_id
             where qk.question_id = ?1",
        )
        .and_then(|mut stmt| {
            stmt.query_map([id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();
    let assets: Vec<String> = conn
        .prepare("select relative_path from question_asset where question_id = ?1 order by sort")
        .and_then(|mut stmt| {
            stmt.query_map([id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default();

    Ok(Some(json!({
        "clientId": id,
        "type": kind,
        "stem": stem,
        "options": serde_json::from_str::<Value>(&options).unwrap_or_else(|_| json!([])),
        "answer": answer,
        "analysis": analysis,
        "difficulty": difficulty,
        "tags": tags,
        "knowledgePath": knowledge,
        "subjectName": subject,
        "assets": assets,
        "share": false,
        "deleted": deleted_at.is_some(),
        "updatedAt": updated_at,
    })))
}

/// 推送待同步题目。返回成功条数；失败写回 outbox 的 last_error 与 attempts。
pub fn push(conn: &mut Connection, client: &reqwest::blocking::Client) -> Result<usize, String> {
    let token = token().ok_or_else(|| "还没登录中心系统".to_string())?;
    let base: String = conn
        .query_row(
            "select server_url from sync_account where id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if base.is_empty() {
        return Err("还没登录中心系统".into());
    }

    let mut stmt = conn
        .prepare("select id, entity_id, op from sync_outbox where pushed_at is null order by created_at limit 50")
        .map_err(|e| e.to_string())?;
    let pending: Vec<(String, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if pending.is_empty() {
        return Ok(0);
    }

    let mut questions = Vec::new();
    for (_, entity_id, _) in &pending {
        if let Some(payload) = question_payload(conn, entity_id)? {
            questions.push(payload);
        }
    }
    let batch_id = uuid::Uuid::new_v4().to_string();
    let response = client
        .post(format!("{base}/api/desktop/sync/push"))
        .bearer_auth(&token)
        .timeout(std::time::Duration::from_secs(60))
        .json(&json!({"batchId": batch_id, "questions": questions}))
        .send()
        .map_err(|e| format!("连不上服务器：{e}"))?;
    let code = response.status();
    let body = response.text().map_err(|e| e.to_string())?;
    if !code.is_success() {
        let message = error_of(code, &body);
        for (outbox_id, _, _) in &pending {
            let _ = conn.execute(
                "update sync_outbox set attempts = attempts + 1, last_error = ?2 where id = ?1",
                params![outbox_id, message],
            );
        }
        return Err(message);
    }

    let value: Value = serde_json::from_str(&body).map_err(|e| format!("服务器响应不合法：{e}"))?;
    let results = value
        .get("results")
        .and_then(|node| node.as_array())
        .cloned()
        .unwrap_or_default();
    let mut ok = 0;
    for result in results {
        let client_id = result
            .get("clientId")
            .and_then(|node| node.as_str())
            .unwrap_or("");
        let server_id = result
            .get("serverId")
            .and_then(|node| node.as_str())
            .unwrap_or("");
        let status = result
            .get("status")
            .and_then(|node| node.as_str())
            .unwrap_or("SKIPPED");
        if client_id.is_empty() {
            continue;
        }
        let state = match status {
            "CONFLICT" => "CONFLICT",
            "CREATED" | "UPDATED" => "SYNCED",
            "SKIPPED" => "SYNCED",
            _ => "FAILED",
        };
        if !server_id.is_empty() {
            let _ = conn.execute(
                "insert into sync_link (local_id, server_id, server_updated_at) values (?1, ?2, ?3)
                 on conflict (local_id) do update set server_id = excluded.server_id,
                    server_updated_at = excluded.server_updated_at",
                params![client_id, server_id, text::now()],
            );
            let _ = conn.execute(
                "update question set server_id = ?2, sync_state = ?3 where id = ?1",
                params![client_id, server_id, state],
            );
        } else {
            let _ = conn.execute(
                "update question set sync_state = ?2 where id = ?1",
                params![client_id, state],
            );
        }
        if matches!(status, "CREATED" | "UPDATED" | "SKIPPED") {
            ok += 1;
        }
    }
    for (outbox_id, _, _) in &pending {
        let _ = conn.execute(
            "update sync_outbox set pushed_at = ?2, last_error = null where id = ?1",
            params![outbox_id, text::now()],
        );
    }
    Ok(ok)
}

/// 拉取中心题库里属于自己的题。cursor 用 (updatedAt, id) 避免同一毫秒丢数据。
pub fn pull(conn: &mut Connection, client: &reqwest::blocking::Client) -> Result<usize, String> {
    let token = token().ok_or_else(|| "还没登录中心系统".to_string())?;
    let (base, cursor): (String, String) = conn
        .query_row(
            "select a.server_url, ifnull((select value from meta where key = 'sync.cursor'), '')
             from sync_account a where a.id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if base.is_empty() {
        return Err("还没登录中心系统".into());
    }
    let url = if cursor.is_empty() {
        format!("{base}/api/desktop/sync/pull?limit=200")
    } else {
        format!("{base}/api/desktop/sync/pull?limit=200&cursor={cursor}")
    };
    let response = client
        .get(url)
        .bearer_auth(&token)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .map_err(|e| format!("连不上服务器：{e}"))?;
    let code = response.status();
    let body = response.text().map_err(|e| e.to_string())?;
    if !code.is_success() {
        return Err(error_of(code, &body));
    }
    let value: Value = serde_json::from_str(&body).map_err(|e| format!("服务器响应不合法：{e}"))?;
    let items = value
        .get("items")
        .and_then(|node| node.as_array())
        .cloned()
        .unwrap_or_default();
    let applied = apply_pull_items(conn, &items)?;

    if let Some(next) = value.get("nextCursor").and_then(|node| node.as_str()) {
        crate::db::meta_set(conn, "sync.cursor", next)?;
    }
    Ok(applied)
}

/// 把一批下发项应用到本地库。抽出来是为了能脱离 HTTP 直接测（见 flow_tests）。
pub(crate) fn apply_pull_items(conn: &Connection, items: &[Value]) -> Result<usize, String> {
    let mut applied = 0;

    for item in items {
        let server_id = item
            .get("serverId")
            .and_then(|node| node.as_str())
            .unwrap_or("");
        if server_id.is_empty() {
            continue;
        }
        let local_id: Option<String> = conn
            .query_row(
                "select local_id from sync_link where server_id = ?1",
                [server_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        // 中心那边新加的题，本地还没见过：直接落一条本地题，用中心 id 当本地 id
        let local_id = match local_id {
            Some(id) => id,
            None => match adopt(conn, item, server_id)? {
                Some(id) => id,
                None => continue,
            },
        };

        // 本地有未推送的改动时不覆盖：老师刚改完还没推上去，不能被一次拉取盖掉
        let pending: i64 = conn
            .query_row(
                "select count(*) from sync_outbox where entity_id = ?1 and pushed_at is null",
                [&local_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if pending > 0 {
            continue;
        }

        let deleted = item
            .get("deleted")
            .and_then(|node| node.as_bool())
            .unwrap_or(false);
        if deleted {
            conn.execute(
                "update question set deleted_at = ?2, sync_state = 'SYNCED' where id = ?1 and deleted_at is null",
                params![local_id, text::now()],
            )
            .map_err(|e| e.to_string())?;
            applied += 1;
            continue;
        }
        let stem = item
            .get("stem")
            .and_then(|node| node.as_str())
            .unwrap_or("");
        let answer = item
            .get("answer")
            .and_then(|node| node.as_str())
            .unwrap_or("");
        let analysis = item
            .get("analysis")
            .and_then(|node| node.as_str())
            .unwrap_or("");
        let difficulty = item
            .get("difficulty")
            .and_then(|node| node.as_i64())
            .unwrap_or(3);
        let kind = item
            .get("type")
            .and_then(|node| node.as_str())
            .unwrap_or("");
        let options = item
            .get("options")
            .map(|node| serde_json::to_string(node).unwrap_or_else(|_| "[]".into()))
            .unwrap_or_else(|| "[]".into());
        conn.execute(
            "update question set stem = ?2, answer = ?3, analysis = ?4, difficulty = ?5,
                stem_normalized = ?6, updated_at = ?7, sync_state = 'SYNCED',
                type = case when ?8 <> '' then ?8 else type end,
                options_json = case when ?9 <> '[]' then ?9 else options_json end
             where id = ?1",
            params![
                local_id,
                stem,
                answer,
                analysis,
                difficulty,
                crate::text::normalize_stem(stem),
                text::now(),
                kind,
                options
            ],
        )
        .map_err(|e| e.to_string())?;
        replace_tags(conn, &local_id, item)?;
        attach_center_knowledge(conn, &local_id, item)?;
        applied += 1;
    }

    Ok(applied)
}

/// 把中心的一道题落成本地题。已软删除的不落（等它恢复）。
fn adopt(conn: &Connection, item: &Value, server_id: &str) -> Result<Option<String>, String> {
    if item
        .get("deleted")
        .and_then(|node| node.as_bool())
        .unwrap_or(false)
    {
        return Ok(None);
    }
    let stem = item
        .get("stem")
        .and_then(|node| node.as_str())
        .unwrap_or("");
    if stem.trim().is_empty() {
        return Ok(None);
    }
    let subject_id = subject_id_for(conn, item.get("subjectName").and_then(|node| node.as_str()));
    let kind = item
        .get("type")
        .and_then(|node| node.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("SINGLE");
    let options = item
        .get("options")
        .map(|node| serde_json::to_string(node).unwrap_or_else(|_| "[]".into()))
        .unwrap_or_else(|| "[]".into());
    let now = text::now();
    conn.execute(
        "insert into question (id, subject_id, type, stem, options_json, answer, analysis, difficulty,
             stem_normalized, status, origin, needs_review, created_at, updated_at, sync_state, server_id)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'PUBLISHED', 'CENTER', 0, ?10, ?10, 'SYNCED', ?11)
         on conflict (id) do nothing",
        params![
            server_id,
            subject_id,
            kind,
            stem,
            options,
            item.get("answer").and_then(|node| node.as_str()).unwrap_or(""),
            item.get("analysis").and_then(|node| node.as_str()).unwrap_or(""),
            item.get("difficulty").and_then(|node| node.as_i64()).unwrap_or(3),
            crate::text::normalize_stem(stem),
            now,
            server_id
        ],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "insert into sync_link (local_id, server_id, server_updated_at) values (?1, ?1, ?2)
         on conflict (local_id) do update set server_id = excluded.server_id,
            server_updated_at = excluded.server_updated_at",
        params![server_id, now],
    )
    .map_err(|e| e.to_string())?;
    replace_tags(conn, server_id, item)?;
    attach_center_knowledge(conn, server_id, item)?;
    Ok(Some(server_id.to_string()))
}

/// 标签以中心为准整体替换：本地是快照，不做增量合并。
fn replace_tags(conn: &Connection, question_id: &str, item: &Value) -> Result<(), String> {
    conn.execute(
        "delete from question_tag where question_id = ?1",
        [question_id],
    )
    .map_err(|e| e.to_string())?;
    let insert = |kind: &str, key: &str| -> Result<(), String> {
        let Some(list) = item.get(key).and_then(|node| node.as_array()) else {
            return Ok(());
        };
        for tag in list {
            if let Some(tag) = tag.as_str().filter(|value| !value.trim().is_empty()) {
                conn.execute(
                    "insert or ignore into question_tag (question_id, tag, kind) values (?1, ?2, ?3)",
                    params![question_id, tag.trim(), kind],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    };
    insert("FREE", "tags")?;
    Ok(())
}

/// 中心下发的 examPoints / knowledgePath 一律当知识点名解析，挂到 question_knowledge。
/// 旧中心可能仍发 examPoints 字符串：只按名字复用本地知识点，绝不重建旧标签。
fn attach_center_knowledge(
    conn: &Connection,
    question_id: &str,
    item: &Value,
) -> Result<(), String> {
    let subject_id = subject_id_for(conn, item.get("subjectName").and_then(|node| node.as_str()))
        .unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    for key in ["examPoints", "knowledgePath"] {
        if let Some(list) = item.get(key).and_then(|node| node.as_array()) {
            for value in list {
                if let Some(raw) = value.as_str() {
                    // 中心习惯发 "教材/章/知识点"，取最后一段当名字；旧 examPoints 就是名字。
                    if let Some(name) = raw.rsplit('/').map(str::trim).find(|part| !part.is_empty())
                    {
                        names.push(name.to_string());
                    }
                }
            }
        }
    }
    for name in names {
        if let Some(id) = crate::jobs::resolve_point(conn, &subject_id, &name) {
            conn.execute(
                "insert or ignore into question_knowledge (question_id, knowledge_point_id) values (?1, ?2)",
                params![question_id, id],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 学科按名字对齐；对不上就落到第一个学科，保证题能进库（与中心端策略一致）。
fn subject_id_for(conn: &Connection, name: Option<&str>) -> Option<String> {
    if let Some(name) = name.filter(|value| !value.trim().is_empty()) {
        if let Ok(id) = conn.query_row(
            "select id from subject where name = ?1 limit 1",
            [name.trim()],
            |row| row.get::<_, String>(0),
        ) {
            return Some(id);
        }
    }
    conn.query_row("select id from subject order by sort limit 1", [], |row| {
        row.get(0)
    })
    .ok()
}

pub fn retry_failed(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "update sync_outbox set attempts = 0, last_error = null where pushed_at is null",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
