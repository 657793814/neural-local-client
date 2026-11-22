//! 听写台仓储：只存当前批次内容与拼音，不写判分 / 会话 / 学情。

use chrono::Local;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn now() -> String {
    Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

fn str_field<'a>(value: &'a Value, key: &str, default: &'a str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or(default)
}

fn array_field(value: &Value, key: &str) -> Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// 开播前的硬门槛：多音字没定音的不许开始。
pub fn validate_draft(input: &Value) -> Result<Vec<Value>, String> {
    let items = array_field(input, "items");
    if items.is_empty() {
        return Err("还没有词条，先识别或手工添加".into());
    }
    let mut pending = Vec::new();
    for item in &items {
        let text = str_field(item, "text", "").trim();
        if text.is_empty() {
            return Err("有条词条是空的，删掉或补全再开始".into());
        }
        let candidates = array_field(item, "pinyinCandidates");
        let selected = str_field(item, "selectedPinyin", "").trim();
        let all_chosen = candidates.iter().all(|candidate| {
            candidate.get("selected").and_then(Value::as_u64).is_some()
        });
        if str_field(item, "lang", "ZH") == "ZH"
            && (!candidates.is_empty() && !all_chosen || selected.is_empty() && !candidates.is_empty())
        {
            pending.push(text.to_string());
        }
    }
    if !pending.is_empty() {
        let names = pending.iter().take(4).cloned().collect::<Vec<_>>().join("、");
        let more = if pending.len() > 4 {
            format!(" 等 {} 条", pending.len())
        } else {
            String::new()
        };
        return Err(format!("还有多音字没定音：{names}{more}。点开候选选一个读音再开始。"));
    }
    Ok(items)
}

/// 把 tmp 里的原图归档到 assets/dictation/<setId>/，返回相对 assets/ 的路径。
fn archive_image(data_dir: &Path, set_id: &str, source_path: &str) -> Result<String, String> {
    if source_path.trim().is_empty() {
        return Ok(String::new());
    }
    let from = PathBuf::from(source_path);
    if !from.exists() {
        return Err(format!("原图找不到了：{source_path}"));
    }
    let ext = from
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && value.len() <= 5)
        .unwrap_or("png");
    let dir = data_dir.join("assets").join("dictation").join(set_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建听写资产目录失败：{e}"))?;
    let to = dir.join(format!("source.{ext}"));
    std::fs::copy(&from, &to).map_err(|e| format!("归档听写原图失败：{e}"))?;
    Ok(format!("dictation/{set_id}/source.{ext}"))
}

pub fn save_set(conn: &mut Connection, data_dir: &Path, input: &Value) -> Result<String, String> {
    let items = validate_draft(input)?;
    let id = uuid::Uuid::new_v4().to_string();
    let stamp = now();
    let title = {
        let value = str_field(input, "title", "").trim();
        if value.is_empty() {
            "今日听写".to_string()
        } else {
            value.to_string()
        }
    };
    let date = str_field(input, "dictationDate", "").to_string();
    let lang_mode = str_field(input, "langMode", "MIXED").to_string();
    let settings = input.get("settings").cloned().unwrap_or_else(|| json!({}));
    let image_rel = archive_image(data_dir, &id, str_field(input, "sourcePath", ""))?;

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "insert into local_dictation_set
            (id, title, dictation_date, source_image_path, lang_mode, settings_json, created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            title,
            date,
            image_rel,
            lang_mode,
            settings.to_string(),
            stamp,
            stamp
        ],
    )
    .map_err(|e| format!("保存听写集失败：{e}"))?;

    for (index, item) in items.iter().enumerate() {
        let item_id = uuid::Uuid::new_v4().to_string();
        let ord = item.get("ord").and_then(Value::as_i64).unwrap_or(index as i64);
        let bbox = item.get("bbox").cloned().unwrap_or_else(|| json!([]));
        let candidates = item.get("pinyinCandidates").cloned().unwrap_or_else(|| json!([]));
        tx.execute(
            "insert into local_dictation_item
                (id, set_id, ord, item_text, lang, pinyin, pinyin_candidates_json,
                 selected_pinyin, speak_as, meaning, bbox_json, confidence, needs_review, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                item_id,
                id,
                ord,
                str_field(item, "text", ""),
                str_field(item, "lang", "ZH"),
                str_field(item, "pinyin", ""),
                candidates.to_string(),
                str_field(item, "selectedPinyin", ""),
                str_field(item, "speakAs", str_field(item, "text", "")),
                str_field(item, "meaning", ""),
                bbox.to_string(),
                item.get("confidence").and_then(Value::as_f64).unwrap_or(0.0),
                item.get("needsReview").and_then(Value::as_bool).unwrap_or(false) as i64,
                stamp,
            ],
        )
        .map_err(|e| format!("保存词条失败：{e}"))?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn list_sets(conn: &Connection) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(
            "select s.id, s.title, s.dictation_date, s.lang_mode, s.updated_at,
                    (select count(*) from local_dictation_item i where i.set_id = s.id) as item_count
             from local_dictation_set s
             where s.deleted_at is null
             order by coalesce(s.dictation_date, '') desc, s.updated_at desc",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "title": row.get::<_, String>(1)?,
                "date": row.get::<_, String>(2)?,
                "langMode": row.get::<_, String>(3)?,
                "updatedAt": row.get::<_, String>(4)?,
                "itemCount": row.get::<_, i64>(5)?,
            }))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

pub fn get_set(conn: &Connection, id: &str) -> Result<Value, String> {
    let set: Option<Value> = conn
        .query_row(
            "select id, title, dictation_date, source_image_path, lang_mode, settings_json, updated_at
             from local_dictation_set where id = ?1 and deleted_at is null",
            [id],
            |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "date": row.get::<_, String>(2)?,
                    "sourcePath": row.get::<_, String>(3)?,
                    "langMode": row.get::<_, String>(4)?,
                    "settings": row.get::<_, String>(5)?,
                    "updatedAt": row.get::<_, String>(6)?,
                }))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let mut set = set.ok_or_else(|| "这份听写集不存在或已删除".to_string())?;
    let mut stmt = conn
        .prepare(
            "select id, ord, item_text, lang, pinyin, pinyin_candidates_json,
                    selected_pinyin, speak_as, meaning, bbox_json, confidence, needs_review
             from local_dictation_item where set_id = ?1 order by ord",
        )
        .map_err(|e| e.to_string())?;
    let items: Vec<Value> = stmt
        .query_map([id], |row| {
            let lang: String = row.get(3)?;
            let text: String = row.get(2)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "ord": row.get::<_, i64>(1)?,
                "text": text,
                "lang": lang,
                "pinyin": row.get::<_, String>(4)?,
                "pinyinCandidates": row.get::<_, String>(5)?,
                "selectedPinyin": row.get::<_, String>(6)?,
                "speakAs": row.get::<_, String>(7)?,
                "meaning": row.get::<_, String>(8)?,
                "bbox": row.get::<_, String>(9)?,
                "confidence": row.get::<_, f64>(10)?,
                "needsReview": row.get::<_, i64>(11)? != 0,
            }))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    set["items"] = json!(items);
    Ok(set)
}

pub fn delete_set(conn: &Connection, id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "update local_dictation_set set deleted_at = ?1, updated_at = ?1 where id = ?2",
            params![now(), id],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("这份听写集已经不在了".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polyphone_without_selection_blocks_start() {
        let draft = json!({
            "items": [{
                "text": "重",
                "lang": "ZH",
                "pinyinCandidates": [{"char": "重", "index": 0, "selected": 0, "options": [{"pinyin": "zhòng"}, {"pinyin": "chóng"}]}],
                "selectedPinyin": ""
            }]
        });
        let error = validate_draft(&draft).unwrap_err();
        assert!(error.contains("重"), "{error}");
        let mut fixed = draft;
        fixed["items"][0]["selectedPinyin"] = json!("zhòng");
        assert!(validate_draft(&fixed).is_ok());
    }

    #[test]
    fn saved_set_round_trips_with_items() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        conn.execute_batch("create table app_setting (key text primary key, value_json text not null);")
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0017_dictation.sql")).unwrap();
        let dir = std::env::temp_dir().join(format!("nb-dictation-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let draft = json!({
            "title": "今天的听写",
            "dictationDate": "2026-11-22",
            "items": [{
                "text": "银行", "lang": "ZH", "pinyin": "yín háng",
                "pinyinCandidates": [], "selectedPinyin": "yín háng", "speakAs": "银行"
            }]
        });
        let id = save_set(&mut conn, &dir, &draft).unwrap();
        let got = get_set(&conn, &id).unwrap();
        assert_eq!(got["title"], "今天的听写");
        assert_eq!(got["items"][0]["text"], "银行");
        assert_eq!(got["items"][0]["selectedPinyin"], "yín háng");
        let list = list_sets(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["itemCount"], 1);
        std::fs::remove_dir_all(dir).ok();
    }
}
