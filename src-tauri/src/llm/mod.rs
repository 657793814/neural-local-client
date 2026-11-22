//! 模型池、用途路由与调用。
//!
//! 四条硬规则：
//! 1. SQLite 里只存 Key 的引用，任何返回值都不带回 Key。
//! 2. 调用一律在 Rust 侧发起，前端拿不到 Key，也拿不到 Authorization 头。
//! 3. 429 / 配额错误让该模型冷却 2 分钟，全部冷却时仍然试一次。
//! 4. 钥匙串一个进程只读一次（见 `KEY_CACHE`）：macOS 每次读都弹授权框，
//!    而一次批量归类是 N 次调用，不缓存就会连弹 N 次。
//!
//! 关于第 4 条还有一个坑：macOS 的登录钥匙串把「信任哪个程序」记在条目的 ACL 里，
//! 而这个 app 是自签名的，每次重新打包 CDHash 都会变，于是**每次重新构建后第一次读
//! 都会再弹一次授权框**，用户点了「允许」也只对当下这一次有效。所以 Key 拿到之后会
//! 落一份到 `本地仓库`（见 `vault_*`，权限 0600、放在配置目录而不是会被云同步的数据
//! 目录里），之后一律先读本地仓库，不再打扰系统钥匙串。钥匙串仍然是 Key 的正本，
//! 本地仓库只是「读过了就别再问」的缓存，换 Key / 删模型时两边一起清。

pub mod json;
pub mod prompts;

use crate::models::{ModelInput, ModelSpec, TestResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const SERVICE: &str = "com.neuralkb.local";
const COOLDOWN: Duration = Duration::from_secs(120);

/// 用途标识。与 `llm_route.purpose` 一一对应。
pub const PURPOSES: &[(&str, &str)] = &[
    ("chat", "对话"),
    ("capture", "截图识别"),
    ("question", "AI 出题"),
    ("classify", "归类与解析"),
    ("geogebra", "GeoGebra 制图"),
    ("diagram", "AI 制图（SVG）"),
    ("repair", "图片修复"),
    ("grading", "批改抄录"),
    ("advice", "学情建议"),
    ("courseware", "课件生成"),
    ("courseware_import", "课件整理"),
];

pub struct ImageInput {
    pub mime: String,
    pub bytes: Vec<u8>,
}

pub struct Msg {
    pub role: &'static str,
    pub content: String,
}

pub fn msg(role: &'static str, content: impl Into<String>) -> Msg {
    Msg {
        role,
        content: content.into(),
    }
}

fn entry(key_ref: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, key_ref).map_err(|e| format!("打不开系统钥匙串：{e}"))
}

// ---------------------------------------------------------------------------
// 本地仓库：让 Key 只从钥匙串读一次
// ---------------------------------------------------------------------------

/// 配置目录（不是数据目录：数据目录可能在 iCloud 里被同步出去）。
static VAULT_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 启动时登记配置目录。没登记时本地仓库整体失效，一切照旧走钥匙串。
pub fn init_vault(dir: &Path) {
    let _ = VAULT_DIR.set(dir.to_path_buf());
}

fn vault_path() -> Option<PathBuf> {
    let dir = VAULT_DIR.get()?.join("secrets");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("keys.json"))
}

fn vault_seed() -> Option<Vec<u8>> {
    let path = VAULT_DIR.get()?.join("secrets").join(".seed");
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() >= 16 {
            return Some(bytes);
        }
    }
    let mut seed = Vec::with_capacity(32);
    seed.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    seed.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    if std::fs::write(&path, &seed).is_err() {
        return None;
    }
    restrict(&path);
    Some(seed)
}

/// 0600：只有当下这个用户能读。
fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// 只是遮一下明文，别让 `grep sk-` 或云同步直接把 Key 带走；不是密码学意义上的加密。
/// 异或本身对称，加密解密是同一个函数。
fn mask(data: &[u8], seed: &[u8]) -> Vec<u8> {
    if seed.is_empty() {
        return data.to_vec();
    }
    data.iter()
        .enumerate()
        .map(|(index, byte)| byte ^ seed[index % seed.len()])
        .collect()
}

fn vault_all() -> HashMap<String, String> {
    let Some(file) = vault_path() else {
        return HashMap::new();
    };
    let Ok(raw) = std::fs::read_to_string(file) else {
        return HashMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn vault_get(key_ref: &str) -> Option<String> {
    use base64::Engine;
    let masked = vault_all().remove(key_ref)?;
    let seed = vault_seed()?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(masked.as_bytes())
        .ok()?;
    let plain = mask(&bytes, &seed);
    String::from_utf8(plain).ok().filter(|key| !key.trim().is_empty())
}

fn vault_put(key_ref: &str, key: &str) {
    use base64::Engine;
    let (Some(file), Some(seed)) = (vault_path(), vault_seed()) else {
        return;
    };
    let mut all = vault_all();
    all.insert(
        key_ref.to_string(),
        base64::engine::general_purpose::STANDARD.encode(mask(key.as_bytes(), &seed)),
    );
    if let Ok(body) = serde_json::to_string_pretty(&all) {
        if std::fs::write(&file, body).is_ok() {
            restrict(&file);
        }
    }
}

fn vault_remove(key_ref: &str) {
    let Some(file) = vault_path() else { return };
    let mut all = vault_all();
    if all.remove(key_ref).is_none() {
        return;
    }
    if let Ok(body) = serde_json::to_string_pretty(&all) {
        let _ = std::fs::write(&file, body);
    }
}

/// 进程内 Key 缓存：`None` 表示「查过，钥匙串里确实没有」。
///
/// 为什么必须有：macOS 上每次 `get_password()` 都会弹一次「想访问钥匙串中的机密信息」，
/// 而 `run_classify` 是「每道题一次模型调用」，一次归类 30 道题就会连弹 30 次授权框。
/// 缓存后一个进程只问一次；换 Key / 删模型时同步失效，改完立刻生效。
static KEY_CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();

fn key_cache() -> &'static Mutex<HashMap<String, Option<String>>> {
    KEY_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_key(key_ref: &str) -> Option<Option<String>> {
    let cache = key_cache().lock().ok()?;
    cache.get(key_ref).cloned()
}

fn cache_put(key_ref: &str, value: Option<String>) {
    if let Ok(mut cache) = key_cache().lock() {
        cache.insert(key_ref.to_string(), value);
    }
}

pub fn set_key(key_ref: &str, key: &str) -> Result<(), String> {
    if key.trim().is_empty() {
        return Ok(());
    }
    let key = key.trim();
    // 先落本地仓库：即使钥匙串那边被拒绝，这次保存也算成功，后面调用直接用得上。
    vault_put(key_ref, key);
    cache_put(key_ref, Some(key.to_string()));
    entry(key_ref)?
        .set_password(key)
        .map_err(|e| format!("Key 保存失败：{e}"))
}

/// 读 Key：内存缓存 → 本地仓库 → 系统钥匙串（只读这一次，读到就落本地仓库）。
pub fn get_key(key_ref: &str) -> Option<String> {
    if let Some(hit) = cached_key(key_ref) {
        return hit;
    }
    if let Some(key) = vault_get(key_ref) {
        cache_put(key_ref, Some(key.clone()));
        return Some(key);
    }
    let value = match keyring::Entry::new(SERVICE, key_ref) {
        Ok(item) => match item.get_password() {
            Ok(password) => {
                // 迁移到本地仓库，下次启动不再弹授权框。
                vault_put(key_ref, &password);
                Some(password)
            }
            // 确实没存过：记住这个结论，别每次调用都去敲一次钥匙串。
            Err(keyring::Error::NoEntry) => None,
            // 授权被拒 / 钥匙串锁着：这次拿不到，但不记账——用户下次点「允许」还能用。
            Err(_) => return None,
        },
        Err(_) => return None,
    };
    cache_put(key_ref, value.clone());
    value
}

pub fn delete_key(key_ref: &str) {
    if let Ok(entry) = entry(key_ref) {
        let _ = entry.delete_credential();
    }
    vault_remove(key_ref);
    if let Ok(mut cache) = key_cache().lock() {
        cache.remove(key_ref);
    }
}

pub fn list_models(conn: &Connection) -> Result<Vec<ModelSpec>, String> {
    // has_key 从数据库列读，不碰系统钥匙串：每次读钥匙串都会弹 macOS 授权框。
    let mut stmt = conn
        .prepare(
            "select id, name, base_url, model, supports_vision, enabled, note, has_key
             from llm_model order by created_at",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<ModelSpec> = stmt
        .query_map([], |row| {
            Ok(ModelSpec {
                id: row.get(0)?,
                name: row.get(1)?,
                base_url: row.get(2)?,
                model: row.get(3)?,
                supports_vision: row.get::<_, i64>(4)? != 0,
                enabled: row.get::<_, i64>(5)? != 0,
                note: row.get(6)?,
                has_key: row.get::<_, i64>(7)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

pub fn save_model(conn: &mut Connection, input: &ModelInput) -> Result<String, String> {
    let name = input.name.trim();
    let base_url = input.base_url.trim().trim_end_matches('/');
    let model = input.model.trim();
    if name.is_empty() {
        return Err("给这个模型起个名字，方便自己认".into());
    }
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return Err("Base URL 需要以 http:// 或 https:// 开头".into());
    }
    if model.is_empty() {
        return Err("模型名不能为空，填服务商给的模型 ID".into());
    }
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let key_ref = id.clone();
    let exists: Option<String> = conn
        .query_row("select id from llm_model where id = ?1", [&id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;

    if exists.is_some() {
        conn.execute(
            "update llm_model set name = ?2, base_url = ?3, model = ?4, supports_vision = ?5,
                enabled = ?6, note = ?7 where id = ?1",
            params![
                id,
                name,
                base_url,
                model,
                input.supports_vision as i64,
                input.enabled as i64,
                input.note
            ],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "insert into llm_model (id, name, base_url, model, supports_vision, enabled, key_ref, note)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                name,
                base_url,
                model,
                input.supports_vision as i64,
                input.enabled as i64,
                key_ref,
                input.note
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    if let Some(key) = input.api_key.as_deref() {
        if !key.trim().is_empty() {
            set_key(&key_ref, key)?;
            // 记一个标记位，列表页读它就不用再碰钥匙串了。
            let _ = conn.execute(
                "update llm_model set has_key = 1 where id = ?1",
                params![id],
            );
        }
    }

    // 第一次添加模型时把它排进所有用途，避免用户加完模型仍然「没有可用的模型」。
    let route_count: i64 = conn
        .query_row("select count(*) from llm_route", [], |row| row.get(0))
        .unwrap_or(0);
    if route_count == 0 {
        for (purpose, _) in PURPOSES {
            let _ = conn.execute(
                "insert or ignore into llm_route (purpose, priority, model_id) values (?1, 0, ?2)",
                params![purpose, id],
            );
        }
    }
    Ok(id)
}

pub fn delete_model(conn: &mut Connection, id: &str) -> Result<(), String> {
    conn.execute("delete from llm_model where id = ?1", [id])
        .map_err(|e| e.to_string())?;
    delete_key(id);
    Ok(())
}

/// 迁移 0008 加了 has_key 列，默认 0。已有模型的 Key 还在钥匙串里，
/// 这里回填一次：对 has_key=0 的模型去钥匙串看一眼，有就置 1。
/// 之后 list_models 只读这一列，不再碰钥匙串，设置页就不会反复弹授权框。
///
/// 只做一次：没 Key 的模型（本地 ollama 之类）has_key 永远是 0，
/// 每次启动都去敲一遍钥匙串，等于每次启动都弹一个授权框。
pub fn backfill_has_key(conn: &Connection) -> Result<(), String> {
    let done: Option<String> = conn
        .query_row(
            "select value_json from app_setting where key = 'llm.has_key_backfilled'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if done.as_deref() == Some("1") {
        return Ok(());
    }

    let mut stmt = conn
        .prepare("select id from llm_model where has_key = 0")
        .map_err(|e| e.to_string())?;
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    for id in ids {
        if get_key(&id).map(|key| !key.is_empty()).unwrap_or(false) {
            let _ = conn.execute(
                "update llm_model set has_key = 1 where id = ?1",
                params![id],
            );
        }
    }
    let _ = conn.execute(
        "insert or replace into app_setting (key, value_json) values ('llm.has_key_backfilled', '1')",
        [],
    );
    Ok(())
}

pub fn routes(conn: &Connection) -> Result<Value, String> {
    let mut stmt = conn
        .prepare("select purpose, model_id from llm_route order by purpose, priority")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    let mut map = serde_json::Map::new();
    for (purpose, model_id) in rows {
        let list = map
            .entry(purpose)
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(array) = list.as_array_mut() {
            array.push(Value::String(model_id));
        }
    }
    Ok(serde_json::json!({
        "purposes": PURPOSES.iter().map(|(key, label)| json!({"key": key, "label": label})).collect::<Vec<_>>(),
        "routes": map,
    }))
}

pub fn set_route(conn: &mut Connection, purpose: &str, model_ids: &[String]) -> Result<(), String> {
    if !PURPOSES.iter().any(|(key, _)| *key == purpose) {
        return Err("未知的用途".into());
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("delete from llm_route where purpose = ?1", [purpose])
        .map_err(|e| e.to_string())?;
    for (index, model_id) in model_ids.iter().enumerate() {
        tx.execute(
            "insert or ignore into llm_route (purpose, priority, model_id) values (?1, ?2, ?3)",
            params![purpose, index as i64, model_id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

fn endpoint(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

fn payload(
    model: &str,
    messages: &[Msg],
    image: Option<&ImageInput>,
    json_mode: bool,
    max_tokens: u32,
) -> Value {
    let built_messages: Vec<Value> = messages
        .iter()
        .map(|message| {
            let is_last_user = message.role == "user";
            if is_last_user && image.is_some() {
                let image = image.unwrap();
                let url = format!(
                    "data:{};base64,{}",
                    image.mime,
                    base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &image.bytes
                    )
                );
                json!({
                    "role": message.role,
                    "content": [
                        {"type": "text", "text": message.content},
                        {"type": "image_url", "image_url": {"url": url}}
                    ]
                })
            } else {
                json!({"role": message.role, "content": message.content})
            }
        })
        .collect();

    let mut body = json!({
        "model": model,
        "messages": built_messages,
        "temperature": 0.3,
        "max_tokens": max_tokens,
        "stream": false,
    });
    if json_mode {
        body["response_format"] = json!({"type": "json_object"});
    }
    body
}

fn describe_error(status: reqwest::StatusCode, body: &str) -> (String, bool) {
    let snippet = json::snippet(body);
    let lower = body.to_lowercase();
    let quota = status.as_u16() == 429
        || lower.contains("rate limit")
        || lower.contains("quota")
        || lower.contains("insufficient");
    let hint = match status.as_u16() {
        401 | 403 => "（Key 不对或没有权限）",
        404 => "（Base URL 或模型名不对）",
        429 => "（请求太频繁或额度用完了）",
        _ => "",
    };
    (
        format!(
            "模型服务返回 {} {}{hint}：{}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            snippet
        ),
        quota,
    )
}

fn call_once(
    client: &reqwest::blocking::Client,
    model: &ModelSpec,
    messages: &[Msg],
    image: Option<&ImageInput>,
    json_mode: bool,
    max_tokens: u32,
) -> Result<String, (String, bool)> {
    let key = get_key(&model.id);
    // 库里记着「有 Key」却读不出来：多半是系统钥匙串没放行。别让它退化成 401
    // 让人以为是 Key 填错了，直接把这条原因说清楚。
    if key.is_none() && model.has_key {
        return Err((
            format!(
                "读不到「{}」的 Key：系统钥匙串没把 Key 交给应用（授权被拒或钥匙串锁着）。到「设置 → 模型」把这个模型的 Key 重填一次即可",
                model.name
            ),
            false,
        ));
    }
    let mut request = client
        .post(endpoint(&model.base_url))
        .timeout(Duration::from_secs(180))
        .json(&payload(
            &model.model,
            messages,
            image,
            json_mode,
            max_tokens,
        ));
    if let Some(key) = key.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(key);
    }
    let response = request.send().map_err(|e| {
        let hint = if e.is_timeout() {
            "请求超时，题目图片可能太大"
        } else if e.is_connect() {
            "连不上这个地址，检查 Base URL 与网络"
        } else {
            "网络请求失败"
        };
        (format!("{hint}：{e}"), false)
    })?;
    let status = response.status();
    let text_body = response
        .text()
        .map_err(|e| (format!("读取模型响应失败：{e}"), false))?;
    if !status.is_success() {
        return Err(describe_error(status, &text_body));
    }
    let value: Value = serde_json::from_str(&text_body).map_err(|e| {
        (
            format!(
                "模型返回的不是 JSON：{e}；原文「{}」",
                json::snippet(&text_body)
            ),
            false,
        )
    })?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(|node| node.as_str())
        .or_else(|| {
            value
                .pointer("/choices/0/message/reasoning_content")
                .and_then(|node| node.as_str())
        })
        .map(|text| text.to_string())
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| {
            (
                format!("模型没有给出内容，原文「{}」", json::snippet(&text_body)),
                false,
            )
        })?;
    Ok(content)
}

fn candidates(
    conn: &Connection,
    purpose: &str,
    need_vision: bool,
    cooldowns: &HashMap<String, Instant>,
) -> Result<Vec<ModelSpec>, String> {
    let models = list_models(conn)?;
    let mut stmt = conn
        .prepare("select model_id from llm_route where purpose = ?1 order by priority")
        .map_err(|e| e.to_string())?;
    let routed: Vec<String> = stmt
        .query_map([purpose], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let pick = |id: &str| models.iter().find(|model| model.id == id).cloned();
    let mut ordered: Vec<ModelSpec> = routed.iter().filter_map(|id| pick(id)).collect();
    if ordered.is_empty() {
        // 没排用途也别忘了用户刚加的模型：按添加顺序兜底，避免「明明配了却说我没错」。
        ordered = models
            .iter()
            .filter(|model| model.enabled)
            .cloned()
            .collect();
    }
    ordered.retain(|model| model.enabled && (!need_vision || model.supports_vision));

    if ordered.is_empty() {
        return Err(if need_vision {
            "没有可用的视觉模型。到「设置 → 模型」添加一个支持图片的模型，或在「截图识别」里改用粘贴文字。".into()
        } else if models.is_empty() {
            "还没有添加模型。到「设置 → 模型」添加一个 OpenAI 兼容端点；只录题不联网可以先跳过。"
                .into()
        } else {
            "这个用途还没有可用模型。到「设置 → 模型」把它排进用途，或添加新模型。".into()
        });
    }

    // 冷却中的排到后面，但不清除——全部冷却时仍要试一次。
    let now = Instant::now();
    ordered.sort_by_key(|model| {
        cooldowns
            .get(&model.id)
            .map(|until| if *until > now { 1 } else { 0 })
            .unwrap_or(0)
    });
    Ok(ordered)
}

pub fn complete(
    conn: &Connection,
    client: &reqwest::blocking::Client,
    purpose: &str,
    messages: &[Msg],
    image: Option<&ImageInput>,
    json_mode: bool,
    max_tokens: u32,
    cooldowns: &mut HashMap<String, Instant>,
) -> Result<String, String> {
    let ordered = candidates(conn, purpose, image.is_some(), cooldowns)?;
    let mut errors: Vec<String> = Vec::new();
    for model in &ordered {
        if let Some(until) = cooldowns.get(&model.id) {
            if *until > Instant::now() {
                continue;
            }
        }
        match call_once(client, model, messages, image, json_mode, max_tokens) {
            Ok(text) => return Ok(text),
            Err((error, quota)) => {
                if quota {
                    cooldowns.insert(model.id.clone(), Instant::now() + COOLDOWN);
                }
                errors.push(format!("{}：{}", model.name, error));
            }
        }
    }
    // 全都因冷却被跳过时，放行最后一个再试一次。
    if errors.is_empty() {
        if let Some(model) = ordered.last() {
            cooldowns.remove(&model.id);
            if let Ok(text) = call_once(client, model, messages, image, json_mode, max_tokens) {
                return Ok(text);
            }
        }
    }
    Err(errors.join("\n"))
}

/// 拉取服务商的模型列表（OpenAI 兼容的 `GET {base}/models`）。
///
/// Key 的来源：优先用表单里刚填的，其次用已保存模型的钥匙串。这样「添加」和「编辑」两个
/// 场景都能拉。Ollama 一类无 Key 的本地服务直接匿名请求。
pub fn list_remote_models(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: Option<String>,
    saved_model_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let base = base_url.trim().trim_end_matches('/');
    let base = base.strip_suffix("/chat/completions").unwrap_or(base);
    let url = format!("{base}/models");
    let key = api_key
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| saved_model_id.and_then(get_key));
    let mut request = client.get(&url).timeout(Duration::from_secs(20));
    if let Some(key) = key.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(key);
    }
    let response = request.send().map_err(|e| {
        if e.is_connect() {
            format!("连不上 {url}，检查 Base URL 与网络")
        } else {
            format!("拉取模型列表失败：{e}")
        }
    })?;
    let status = response.status();
    let text = response.text().map_err(|e| format!("读取响应失败：{e}"))?;
    if !status.is_success() {
        let (error, _) = describe_error(status, &text);
        return Err(error);
    }
    let ids = parse_model_ids(&text)?;
    if ids.is_empty() {
        return Err(
            "这个地址没有返回任何模型。确认 Base URL（一般到 /v1），或直接手填模型 ID".into(),
        );
    }
    Ok(ids)
}

/// 解析 `/models` 的返回。各家字段不统一，按常见形态依次尝试：
/// OpenAI 兼容 `{"data":[{"id":…}]}`、`{"models":[{"id"|"name":…}]}`、纯数组。
fn parse_model_ids(text: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| format!("返回的不是 JSON：{e}；原文「{}」", json::snippet(text)))?;
    let mut ids: Vec<String> = Vec::new();

    let read_item = |item: &Value| -> Option<String> {
        item.get("id")
            .or_else(|| item.get("name"))
            .and_then(|node| node.as_str())
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
    };

    if let Some(array) = value.get("data").and_then(Value::as_array) {
        array
            .iter()
            .filter_map(read_item)
            .for_each(|id| ids.push(id));
    }
    if ids.is_empty() {
        if let Some(array) = value.get("models").and_then(Value::as_array) {
            array
                .iter()
                .filter_map(read_item)
                .for_each(|id| ids.push(id));
        }
    }
    if ids.is_empty() {
        if let Some(array) = value.as_array() {
            array
                .iter()
                .filter_map(|item| item.as_str().map(|id| id.trim().to_string()))
                .filter(|id| !id.is_empty())
                .for_each(|id| ids.push(id));
        }
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub fn test(
    conn: &Connection,
    client: &reqwest::blocking::Client,
    id: &str,
) -> Result<TestResult, String> {
    let models = list_models(conn)?;
    let model = models
        .into_iter()
        .find(|model| model.id == id)
        .ok_or_else(|| "模型不在了".to_string())?;
    let started = Instant::now();
    let messages = vec![msg("user", "ping")];
    match call_once(client, &model, &messages, None, false, 8) {
        Ok(text) => Ok(TestResult {
            ok: true,
            latency_ms: started.elapsed().as_millis() as i64,
            detail: format!("连接正常，模型回了一句「{}」", json::snippet(&text)),
        }),
        Err((error, _)) => Ok(TestResult {
            ok: false,
            latency_ms: started.elapsed().as_millis() as i64,
            detail: error,
        }),
    }
}

/// 把模型返回的 JSON 解析成对象；失败时带上用途名。
pub fn parse_json(raw: &str, what: &str) -> Result<Value, String> {
    json::parse(raw, what)
}

#[cfg(test)]
mod tests {
    use super::{cache_put, cached_key, mask, parse_model_ids};

    /// 本地仓库要能存、能取、能删，文件本身不能出现 Key 明文。
    /// `init_vault` 是进程级 OnceLock，所以这一个测试独占它。
    #[test]
    fn vault_round_trip_keeps_plaintext_out_of_file() {
        let dir = std::env::temp_dir().join(format!("nb-vault-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        super::init_vault(&dir);
        super::vault_put("model-a", "sk-vault-secret");
        assert_eq!(
            super::vault_get("model-a").as_deref(),
            Some("sk-vault-secret")
        );
        let raw = std::fs::read_to_string(dir.join("secrets/keys.json")).unwrap();
        assert!(!raw.contains("sk-vault-secret"), "文件里不该有明文：{raw}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("secrets/keys.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        super::vault_remove("model-a");
        assert_eq!(super::vault_get("model-a"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 本地仓库的遮罩必须可逆，且不能让 Key 明文落在文件里。
    #[test]
    fn vault_mask_is_reversible_and_hides_plaintext() {
        let seed = b"0123456789abcdef0123456789abcdef";
        let plain = b"sk-test-1234567890";
        let masked = mask(plain, seed);
        assert_ne!(masked.as_slice(), plain.as_slice());
        assert_eq!(mask(&masked, seed).as_slice(), plain.as_slice());
        // 空 seed 时原样返回，别把 Key 变成乱码。
        assert_eq!(mask(plain, b"").as_slice(), plain.as_slice());
    }

    /// 缓存是「一个进程只问一次钥匙串」的全部依赖，命中/缺失/显式空三种情形都要对。
    #[test]
    fn key_cache_remembers_hits_and_misses() {
        let ref_key = "test-cache-hit";
        assert_eq!(cached_key(ref_key), None, "没写过的引用不该有缓存");

        cache_put(ref_key, Some("sk-test".into()));
        assert_eq!(cached_key(ref_key), Some(Some("sk-test".into())));

        // 「查过但确实没有」也要记住，否则每次调用都会再弹一次授权框。
        let ref_missing = "test-cache-missing";
        cache_put(ref_missing, None);
        assert_eq!(cached_key(ref_missing), Some(None));
    }

    #[test]
    fn parses_openai_shape_and_sorts() {
        let ids = parse_model_ids(
            r#"{"object":"list","data":[{"id":"gpt-4o"},{"id":"gpt-4o-mini"},{"id":"gpt-4o"}]}"#,
        )
        .unwrap();
        assert_eq!(ids, vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()]);
    }

    #[test]
    fn parses_models_key_name_and_plain_array() {
        let named =
            parse_model_ids(r#"{"models":[{"name":"qwen2.5:7b"},{"name":"llama3"}]}"#).unwrap();
        assert_eq!(named, vec!["llama3".to_string(), "qwen2.5:7b".to_string()]);
        let plain = parse_model_ids(r#"["deepseek-chat","deepseek-reasoner"]"#).unwrap();
        assert_eq!(plain.len(), 2);
        assert!(plain.contains(&"deepseek-chat".to_string()));
    }

    #[test]
    fn empty_list_and_bad_json_are_errors_or_empty() {
        assert!(parse_model_ids(r#"{"data":[]}"#).unwrap().is_empty());
        assert!(parse_model_ids("not json").is_err());
    }
}

#[cfg(test)]
mod list_remote_tests {
    use super::list_remote_models;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// 起一个一次性 mock 服务，验证请求打到了 /v1/models、Key 走 Bearer、解析结果排序去重。
    #[test]
    fn fetches_sorted_ids_with_bearer_key() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let authed = request.contains("Authorization: Bearer sk-local-test");
            let path_ok = request.starts_with("GET /v1/models");
            let body = format!(
                "{{\"object\":\"list\",\"data\":[{{\"id\":\"m-b\"}},{{\"id\":\"m-a\"}},{{\"id\":\"m-b\"}},{{\"id\":\"  \"}}],\"echo\":{{\"path\":{path_ok},\"auth\":{authed}}}}}"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            let _ = stream.flush();
        });

        let client = reqwest::blocking::Client::new();
        let ids = list_remote_models(
            &client,
            &format!("http://{addr}/v1"),
            Some("sk-local-test".into()),
            None,
        )
        .unwrap();
        handle.join().unwrap();
        assert_eq!(ids, vec!["m-a".to_string(), "m-b".to_string()]);
    }

    /// Base URL 误填到 /chat/completions 时也应能拉到列表（后端剥掉后缀）。
    #[test]
    fn strips_chat_completions_suffix() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let path_ok = request.starts_with("GET /v1/models");
            let body = format!("{{\"data\":[{{\"id\":\"ollama3\"}}],\"echoPath\":{path_ok}}}");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            let _ = stream.flush();
        });

        let client = reqwest::blocking::Client::new();
        let ids = list_remote_models(
            &client,
            &format!("http://{addr}/v1/chat/completions"),
            None,
            None,
        )
        .unwrap();
        handle.join().unwrap();
        assert_eq!(ids, vec!["ollama3".to_string()]);
    }

    /// 地址不通时给出人话报错。
    #[test]
    fn reports_connect_failure_in_plain_words() {
        let client = reqwest::blocking::Client::new();
        let error = list_remote_models(&client, "http://127.0.0.1:9/v1", None, None).unwrap_err();
        assert!(error.contains("连不上"), "实际报错：{error}");
    }
}
