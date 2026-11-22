//! 前端可调用的命令。命名与前端 `src/lib/api.js` 一一对应。
//!
//! 网络类命令都是 `async` + `spawn_blocking`：模型一次要 30–60 秒，
//! 不能卡住窗口线程；数据库类命令很快，直接同步执行。

use crate::auth;
use crate::browser;
use crate::capture;
use crate::compose;
use crate::db;
use crate::import;
use crate::jobs;
use crate::llm;
use crate::models::*;
use crate::ocr;
use crate::overlay;
use crate::repo;
use crate::review;
use crate::screen;
use crate::smart;
use crate::state::{self, AppState, JobContext};
use crate::sync;
use crate::teaching;
use base64::Engine;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::MutexGuard;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

fn lock(state: &AppState) -> Result<MutexGuard<'_, rusqlite::Connection>, String> {
    state.db.lock().map_err(|_| "数据库忙，请重试".to_string())
}

async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| format!("后台任务失败：{e}"))?
}

fn setting_get(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    conn.query_row(
        "select value_json from app_setting where key = ?1",
        [key],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

// ---------------------------------------------------------------------------
// 启动与设置
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    let conn = lock(&state)?;
    let stats = repo::stats(&conn)?;
    let wizard_done = setting_get(&conn, "wizard.done")
        .map(|value| value == "1")
        .unwrap_or(false);
    let models: i64 = conn
        .query_row("select count(*) from llm_model", [], |row| row.get(0))
        .unwrap_or(0);
    let unlocked = *state
        .unlocked
        .lock()
        .map_err(|_| "会话状态忙".to_string())?;
    let auth_status = auth::status(&conn, unlocked)?;
    Ok(Bootstrap {
        data_dir: state.data_dir.to_string_lossy().to_string(),
        db_path: state.db_path.to_string_lossy().to_string(),
        has_data: stats.total > 0,
        wizard_done,
        search_mode: state.search_mode.clone(),
        stats,
        models,
        app_version: state.app_version.clone(),
        platform: std::env::consts::OS.to_string(),
        auth: auth_status,
    })
}

// ---------------------------------------------------------------------------
// 本地账号
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn auth_status(state: State<'_, AppState>) -> Result<AuthStatus, String> {
    let conn = lock(&state)?;
    let unlocked = *state
        .unlocked
        .lock()
        .map_err(|_| "会话状态忙".to_string())?;
    auth::status(&conn, unlocked)
}

#[tauri::command]
pub fn auth_signup(
    state: State<'_, AppState>,
    input: AuthSignupInput,
) -> Result<AuthStatus, String> {
    let conn = lock(&state)?;
    let result = auth::signup(&conn, &input.username, &input.password, &input.display_name)?;
    let mut unlocked = state
        .unlocked
        .lock()
        .map_err(|_| "会话状态忙".to_string())?;
    *unlocked = true;
    Ok(result)
}

#[tauri::command]
pub fn auth_login(state: State<'_, AppState>, input: AuthLoginInput) -> Result<AuthStatus, String> {
    let conn = lock(&state)?;
    let mut unlocked = state
        .unlocked
        .lock()
        .map_err(|_| "会话状态忙".to_string())?;
    auth::login(&conn, &mut unlocked, &input.username, &input.password)
}

#[tauri::command]
pub fn auth_logout(state: State<'_, AppState>) -> Result<(), String> {
    let mut unlocked = state
        .unlocked
        .lock()
        .map_err(|_| "会话状态忙".to_string())?;
    auth::logout(&mut unlocked);
    Ok(())
}

#[tauri::command]
pub fn auth_change_password(
    state: State<'_, AppState>,
    input: AuthPasswordInput,
) -> Result<(), String> {
    let conn = lock(&state)?;
    let mut unlocked = state
        .unlocked
        .lock()
        .map_err(|_| "会话状态忙".to_string())?;
    auth::change_password(
        &conn,
        &mut unlocked,
        &input.old_password,
        &input.new_password,
    )
}

#[tauri::command]
pub fn auth_update_profile(
    state: State<'_, AppState>,
    input: AuthProfileInput,
) -> Result<AuthStatus, String> {
    let conn = lock(&state)?;
    auth::update_profile(&conn, &input)
}

#[tauri::command]
pub fn stats(state: State<'_, AppState>) -> Result<Stats, String> {
    let conn = lock(&state)?;
    repo::stats(&conn)
}

#[tauri::command]
pub fn setting_get_value(
    state: State<'_, AppState>,
    key: String,
) -> Result<Option<String>, String> {
    let conn = lock(&state)?;
    Ok(setting_get(&conn, &key))
}

#[tauri::command]
pub fn setting_set_value(
    state: State<'_, AppState>,
    key: String,
    value: String,
) -> Result<(), String> {
    let conn = lock(&state)?;
    conn.execute(
        "insert into app_setting (key, value_json) values (?1, ?2)
         on conflict (key) do update set value_json = excluded.value_json",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn wizard_finish(state: State<'_, AppState>) -> Result<(), String> {
    let conn = lock(&state)?;
    conn.execute(
        "insert into app_setting (key, value_json) values ('wizard.done', '1')
         on conflict (key) do update set value_json = '1'",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> Result<Value, String> {
    Ok(json!({
        "version": state.app_version,
        "dataDir": state.data_dir.to_string_lossy(),
        "dbPath": state.db_path.to_string_lossy(),
        "searchMode": state.search_mode,
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "license": "PolyForm Noncommercial 1.0.0",
    }))
}

#[tauri::command]
pub fn data_dir_set(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let target = PathBuf::from(path.trim());
    if target.as_os_str().is_empty() {
        return Err("选一个文件夹".into());
    }
    if target.join("neuralkb.db").exists() {
        // 切到已有库：直接换指针，不动原数据库。
        state::ensure_dirs(&target)?;
        state::write_data_dir(&app, &target)?;
        app.restart();
    }
    state::ensure_dirs(&target)?;
    {
        let conn = lock(&state)?;
        conn.execute("pragma wal_checkpoint(TRUNCATE)", []).ok();
    }
    copy_dir(&state.data_dir, &target)?;
    state::write_data_dir(&app, &target)?;
    app.restart();
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    for entry in std::fs::read_dir(from).map_err(|e| format!("读不了数据目录：{e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if name == "tmp" || name == "backup" {
            continue;
        }
        let target = to.join(&name);
        if entry.path().is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            copy_dir(&entry.path(), &target)?;
        } else if !target.exists() {
            std::fs::copy(entry.path(), &target).map_err(|e| format!("复制失败：{e}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn reveal_data_dir(state: State<'_, AppState>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&state.data_dir)
            .status()
            .map_err(|e| format!("打不开 Finder：{e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&state.data_dir)
            .status()
            .map_err(|e| format!("打不开资源管理器：{e}"))?;
    }
    Ok(())
}

/// 整库备份：先 checkpoint，再打包 db + assets。
#[tauri::command]
pub fn backup_export(state: State<'_, AppState>) -> Result<String, String> {
    {
        let conn = lock(&state)?;
        conn.execute("pragma wal_checkpoint(TRUNCATE)", []).ok();
    }
    let target = write_backup(&state.data_dir)?;
    Ok(target.to_string_lossy().to_string())
}

/// 把 data_dir 里的库与图片打成一个 ZIP。跳过 backup/ 与 tmp/，跳过 WAL 影子文件，
/// 否则备份会把自己也装进去、越备越大。抽成普通函数是为了能直接写测试。
pub(crate) fn write_backup(data_dir: &Path) -> Result<PathBuf, String> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dir = data_dir.join("backup");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let target = dir.join(format!("neuralkb-backup-{stamp}.zip"));
    let file = std::fs::File::create(&target).map_err(|e| format!("建备份文件失败：{e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let root = data_dir.to_path_buf();
    let mut buffer = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir_path) = stack.pop() {
        for entry in std::fs::read_dir(&dir_path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "backup" || name == "tmp" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if name.ends_with("-wal") || name.ends_with("-shm") {
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|_| name.clone());
            buffer.clear();
            use std::io::Read;
            std::fs::File::open(&path)
                .and_then(|mut file| file.read_to_end(&mut buffer))
                .map_err(|e| format!("读 {relative} 失败：{e}"))?;
            zip.start_file(relative, options)
                .map_err(|e| e.to_string())?;
            use std::io::Write;
            zip.write_all(&buffer).map_err(|e| e.to_string())?;
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(target)
}

/// 导出一份「此刻的」数据库快照，给 Navicat / DB Browser 这类工具看数据。
///
/// 直接打开数据目录里的 `neuralkb.db` 不靠稳：库是 WAL 模式，最新写入还在 `-wal` 里，
/// 有些工具会读成旧数据、甚至因为拿不到 `-shm` 锁而打不开。`VACUUM INTO` 出来的
/// 是一个自包含、已整理的普通 `.db`，单文件就能开，也不影响正在用的题库。
#[tauri::command]
pub fn db_export(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let target = PathBuf::from(path.trim());
    if target.as_os_str().is_empty() {
        return Err("还没选保存位置".into());
    }
    if target.exists() {
        return Err("这个位置已经有同名文件了，换个名字或先删掉它".into());
    }
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err("保存目录不存在".into());
        }
    }
    let conn = lock(&state)?;
    conn.execute("vacuum into ?1", [target.to_string_lossy().to_string()])
        .map_err(|e| format!("导出数据库失败：{e}"))?;
    Ok(target.to_string_lossy().to_string())
}

#[tauri::command]
pub fn write_text_file(path: String, content: String) -> Result<(), String> {
    let target = PathBuf::from(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(target, content).map_err(|e| format!("写文件失败：{e}"))
}

#[tauri::command]
pub fn pack_prompt() -> String {
    llm::prompts::PACK_PROMPT.to_string()
}

// ---------------------------------------------------------------------------
// 教材目录与题目
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn curriculum(state: State<'_, AppState>, subject_id: Option<String>) -> Result<Value, String> {
    let conn = lock(&state)?;
    db::curriculum_tree(&conn, subject_id.as_deref())
}

/// 某学科下的全部知识点（这是唯一的知识点词表命令）。
#[tauri::command]
pub fn knowledge_point_list(
    state: State<'_, AppState>,
    subject_id: String,
) -> Result<Vec<KnowledgeRef>, String> {
    let conn = lock(&state)?;
    let mut stmt = conn
        .prepare(
            "select kp.id, kp.name, ifnull(ch.name, '')
             from knowledge_point kp
             left join chapter ch on ch.id = kp.chapter_id
             where kp.subject_id = ?1
             order by kp.sort, kp.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([subject_id], |row| {
            Ok(KnowledgeRef {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

/// 新建知识点：用户可以直接敲一个词表里没有的名字。
///
/// 为什么必须留这个口子：内置词表再全，也总有校本专题、新题型、跨章节综合
/// 覆盖不到；只能从列表里选，等于逼老师把题硬塞进一个不贴切的章节。
#[tauri::command]
pub fn knowledge_point_create(
    state: State<'_, AppState>,
    subject_id: String,
    name: String,
) -> Result<KnowledgeRef, String> {
    let conn = lock(&state)?;
    let id = db::ensure_knowledge_point(&conn, &subject_id, &name)?;
    conn.query_row(
        "select kp.id, kp.name, ifnull(ch.name, '')
         from knowledge_point kp
         left join chapter ch on ch.id = kp.chapter_id
         where kp.id = ?1",
        [&id],
        |row| {
            Ok(KnowledgeRef {
                id: row.get(0)?,
                name: row.get(1)?,
                path: row.get(2)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn question_list(
    state: State<'_, AppState>,
    query: QuestionQuery,
) -> Result<Page<Question>, String> {
    let conn = lock(&state)?;
    repo::list(&conn, &query)
}

/// 题库里出现过的采集来源（网址）及题量。
/// 题库统计 + 补题建议。纯本地 group by，口径与题库列表一致，
/// 不叫模型——用户要能拿这些数字跟列表对得上。
#[tauri::command]
/// `subject_ids` 为空看全库；选一个学科走单学科口径，选多个只算选中范围。
pub fn question_stats(
    state: State<'_, AppState>,
    subject_ids: Option<Vec<String>>,
) -> Result<crate::stats::Stats, String> {
    let conn = lock(&state)?;
    crate::stats::collect(&conn, &subject_ids.unwrap_or_default())
}

#[tauri::command]
pub fn question_sources(state: State<'_, AppState>) -> Result<Vec<SourceCount>, String> {
    let conn = lock(&state)?;
    repo::sources(&conn)
}

#[tauri::command]
pub fn question_get(state: State<'_, AppState>, id: String) -> Result<Question, String> {
    let conn = lock(&state)?;
    repo::get(&conn, &id)
}

#[tauri::command]
pub fn question_save(state: State<'_, AppState>, input: QuestionInput) -> Result<String, String> {
    let mut conn = lock(&state)?;
    repo::save(&mut conn, &input)
}

#[tauri::command]
pub fn question_review(state: State<'_, AppState>, input: ReviewInput) -> Result<usize, String> {
    let mut conn = lock(&state)?;
    repo::review(&mut conn, &input.ids, &input.action, input.note.as_deref())
}

#[tauri::command]
pub fn question_delete(state: State<'_, AppState>, ids: Vec<String>) -> Result<usize, String> {
    let mut conn = lock(&state)?;
    repo::soft_delete(&mut conn, &ids)
}

#[tauri::command]
pub fn question_restore(state: State<'_, AppState>, ids: Vec<String>) -> Result<usize, String> {
    let mut conn = lock(&state)?;
    repo::restore(&mut conn, &ids)
}

#[tauri::command]
pub fn question_purge(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut conn = lock(&state)?;
    let assets = state.assets_dir();
    repo::purge(&mut conn, &id, &assets)
}

#[tauri::command]
pub fn tag_list(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    let conn = lock(&state)?;
    Ok(repo::tags(&conn)?
        .into_iter()
        .map(|(tag, count)| json!({"tag": tag, "count": count}))
        .collect())
}

#[tauri::command]
pub fn question_export(state: State<'_, AppState>, ids: Vec<String>) -> Result<String, String> {
    let conn = lock(&state)?;
    crate::export::pack_json(&conn, &ids)
}

/// 带图 ZIP：题目包 + images/，能原样再导入。
#[tauri::command]
pub fn question_export_zip(
    state: State<'_, AppState>,
    ids: Vec<String>,
    path: String,
) -> Result<crate::export::ExportResult, String> {
    let conn = lock(&state)?;
    let dest = PathBuf::from(path.trim());
    if dest.as_os_str().is_empty() {
        return Err("还没选保存位置".into());
    }
    crate::export::export_zip(&conn, &state.data_dir, &ids, &dest)
}

#[tauri::command]
pub fn asset_data(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let conn = lock(&state)?;
    let relative: String = conn
        .query_row(
            "select relative_path from question_asset where id = ?1",
            [&id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "图片不在了".to_string())?;
    let path = state.data_dir.join("assets").join(relative);
    let bytes = std::fs::read(&path).map_err(|e| format!("读图片失败：{e}"))?;
    let (mime, _) = crate::text::sniff_image(&bytes).ok_or_else(|| "图片格式不认识".to_string())?;
    // 老版本生成的 SVG 只有 viewBox + width="100%"，当 <img> 用时没有固有尺寸、
    // 整张图是空白。已经在题目里的配图没法重生成，读出来时按 viewBox 补宽高。
    if mime == "image/svg+xml" {
        let text = String::from_utf8_lossy(&bytes);
        let fixed = crate::diagram::ensure_intrinsic_size(&text);
        return Ok(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(fixed.as_bytes())
        ));
    }
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

#[tauri::command]
pub fn asset_add(
    state: State<'_, AppState>,
    question_id: String,
    data_url: String,
    kind: Option<String>,
) -> Result<String, String> {
    let conn = lock(&state)?;
    let tmp = capture::save_data_url(&data_url, &state.data_dir)?;
    let bytes = std::fs::read(&tmp).map_err(|e| e.to_string())?;
    let (mime, ext) =
        crate::text::sniff_image(&bytes).ok_or_else(|| "图片格式不支持".to_string())?;
    let kind = kind.unwrap_or_else(|| "FIGURE".into());
    let dir = state.data_dir.join("assets").join(&question_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let count: i64 = conn
        .query_row(
            "select count(*) from question_asset where question_id = ?1",
            [&question_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let file_name = format!("{}-{}.{ext}", kind.to_lowercase(), count + 1);
    std::fs::copy(&tmp, dir.join(&file_name)).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);
    let (width, height) = image::image_dimensions(dir.join(&file_name)).unwrap_or((0, 0));
    let sha = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(&bytes))
    };
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "insert into question_asset (id, question_id, kind, relative_path, content_type, sha256, width, height, sort)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            question_id,
            kind,
            format!("{question_id}/{file_name}"),
            mime,
            sha,
            width as i64,
            height as i64,
            count
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn asset_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    let relative: Option<String> = conn
        .query_row(
            "select relative_path from question_asset where id = ?1",
            [&id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    conn.execute("delete from question_asset where id = ?1", [&id])
        .map_err(|e| e.to_string())?;
    if let Some(relative) = relative {
        let _ = std::fs::remove_file(state.data_dir.join("assets").join(relative));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 截图
// ---------------------------------------------------------------------------

/// 截图出题：置顶选区层 + 标注工具条（移植自 desktop 壳的成熟实现）。
///
/// 流程与 desktop 一致：热键/按钮 → 藏主窗 → 选区层上屏 → 进程内抓帧（CG，
/// screencapture 兜底）→ 贴底图 → 用户框选/标注 → 提交 → 裁剪 + 叠标注 →
/// 落盘到 tmp → 按原路径交给识别任务。选区层不参与屏幕捕获，所以底图里没有它自己。
#[tauri::command]
pub async fn capture_region(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CaptureResult, String> {
    // macOS 把抓屏权限记在「发起进程」头上，首次要主动问一次把自己登记进屏幕录制面板。
    if !crate::perm::request() {
        return Err(concat!(
            "还没拿到屏幕录制权限。请在「系统设置 → 隐私与安全性 → 屏幕录制」里",
            "勾选 NeuralKB Local，然后完全退出应用（⌘Q）再打开——权限只对新进程生效。"
        )
        .into());
    }

    state.picking.store(true, Ordering::SeqCst);

    // 藏起所有自己的窗口：抓帧前它们要真的从屏幕上消失，否则底图里会多出自己。
    // 选区层 content_protected 不参与捕获，但主窗、浏览器、控制条都得藏。
    let hidden = hide_our_windows(&app);

    overlay::prepare(&app);
    let present = overlay::present(&app);
    if let Err(err) = present {
        state.picking.store(false, Ordering::SeqCst);
        restore_windows(&app, &hidden);
        return Err(err);
    }
    tokio::time::sleep(Duration::from_millis(24)).await;

    let shot = match screen::Shot::capture_primary() {
        Ok(shot) => shot,
        Err(err) => {
            overlay::dismiss(&app);
            state.picking.store(false, Ordering::SeqCst);
            restore_windows(&app, &hidden);
            return Err(format!("截图失败：{err}"));
        }
    };
    let (width, height) = shot.size();
    let preview = screen::preview_file(&shot.image)?;
    overlay::attach(
        &app,
        crate::state::ShotInfo {
            shot_path: preview.to_string_lossy().to_string(),
            width,
            height,
        },
    );

    let (sender, receiver) = tokio::sync::oneshot::channel();
    if let Ok(mut slot) = state.pick_tx.lock() {
        *slot = Some(sender);
    }

    let pick = match tokio::time::timeout(Duration::from_secs(180), receiver).await {
        Ok(Ok(pick)) => pick,
        _ => crate::state::Pick::Cancel,
    };

    overlay::dismiss(&app);
    state.picking.store(false, Ordering::SeqCst);
    if let Ok(mut slot) = state.pick_tx.lock() {
        *slot = None;
    }
    restore_windows(&app, &hidden);
    let _ = std::fs::remove_file(&preview);

    let pick = match pick {
        crate::state::Pick::Cancel => return Err("截图已取消".into()),
        other => other,
    };

    let crate::state::Pick::Done {
        rect,
        markup,
        copy_only,
    } = pick
    else {
        return Err("截图已取消".into());
    };
    // 「复制到剪贴板」按钮也走这条链路。local 端暂未接系统剪贴板（desktop 有
    // clipboard.rs），因此 copy_only 与「识别」一样落到整图识别。字段保留以维持
    // 选区层 IPC 契约，显式消费避免 dead_code 警告；接剪贴板时从这里分流即可。
    let _ = copy_only;
    let cropped = match rect {
        Some((x, y, w, h)) => shot.crop(x, y, w, h),
        None => shot.image.clone(),
    };
    let merged = screen::apply_markup(cropped, markup.as_deref());
    let (data_url, bytes) = screen::crop_payload(&merged)?;
    shot.cleanup();

    let dir = state.data_dir.join("tmp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;
    let path = dir.join(format!("capture-{}.jpg", uuid::Uuid::new_v4()));
    std::fs::write(&path, &bytes).map_err(|e| format!("保存截图失败：{e}"))?;
    let _ = data_url; // 前端用 path 走 image_from_path，避免大 data URL 过 IPC。

    capture::from_path(&path)
}

/// 截图期间藏起来的窗口，记下各自原本是否可见，截完原样还回去。
struct HiddenWindows {
    main: bool,
}

fn hide_our_windows(app: &AppHandle) -> HiddenWindows {
    // 内嵌网页视图挂在主窗口上，藏主窗口就一起藏了。
    let state = HiddenWindows {
        main: app
            .get_webview_window("main")
            .map(|w| w.is_visible().unwrap_or(true))
            .unwrap_or(false),
    };
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    state
}

fn restore_windows(app: &AppHandle, hidden: &HiddenWindows) {
    if hidden.main {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }
}

/// 选区层页面加载完报到；若底图已经备好（启动瞬间按热键）顺手补给它。
#[tauri::command]
pub fn overlay_ready(app: AppHandle) -> Option<crate::state::ShotInfo> {
    overlay::mark_ready(&app);
    overlay::take_pending(&app)
}

/// 选区层框选完成：把结果送回正在等着的 capture_region。
#[tauri::command]
pub fn submit_selection(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    full: bool,
    markup: Option<String>,
    copy_only: Option<bool>,
) {
    finish_pick(
        &app,
        crate::state::Pick::Done {
            rect: if full {
                None
            } else {
                Some((x, y, width, height))
            },
            markup,
            copy_only: copy_only.unwrap_or(false),
        },
    );
}

#[tauri::command]
pub fn cancel_selection(app: AppHandle) {
    finish_pick(&app, crate::state::Pick::Cancel);
}

fn finish_pick(app: &AppHandle, pick: crate::state::Pick) {
    let state = app.state::<AppState>();
    let sender = state.pick_tx.lock().ok().and_then(|mut slot| slot.take());
    if let Some(sender) = sender {
        let _ = sender.send(pick);
    }
    overlay::dismiss(app);
}

#[tauri::command]
pub fn overlay_report(note: String) {
    eprintln!("[local] 选区层状态：{note}");
}

#[tauri::command]
pub fn image_from_path(path: String) -> Result<CaptureResult, String> {
    capture::from_path(std::path::Path::new(&path))
}

/// 打开本地文件取题：图片直接读，PDF 按页栅格化。`page` 从 1 开始。
#[tauri::command]
pub fn file_open(
    state: State<'_, AppState>,
    path: String,
    page: Option<i64>,
) -> Result<crate::models::LocalFile, String> {
    crate::filein::open(
        std::path::Path::new(&path),
        page.unwrap_or(1),
        &state.data_dir,
    )
}

#[tauri::command]
pub fn save_temp_image(state: State<'_, AppState>, data_url: String) -> Result<String, String> {
    let path = capture::save_data_url(&data_url, &state.data_dir)?;
    Ok(path.to_string_lossy().to_string())
}

// ---------------------------------------------------------------------------
// AI 制图
// ---------------------------------------------------------------------------

/// 制图结果：svg 文本（预览）、img data URL、临时 svg 路径（供草稿 figures 引用与入库）。
///
/// `rename_all` 不能省：前端读的是 `result.dataUrl`，漏了这一行后端就吐 `data_url`，
/// `<img src={undefined}>` 于是永远空白——SVG 明明生成成功，界面上却什么都没有。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramResult {
    pub svg: String,
    pub data_url: String,
    pub path: String,
}

/// 按题干生成一张 SVG 示意图。先落盘到 data_dir/tmp，得到临时路径；
/// 前端把它写进草稿 item.figures（ref + path），提交时由 stage_figures 正式入库。
#[tauri::command]
pub async fn diagram_generate(
    state: State<'_, AppState>,
    subject_id: String,
    stem: String,
    focus: Option<String>,
    mode: Option<String>,
) -> Result<DiagramResult, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    let data_dir = state.data_dir.clone();
    blocking(move || {
        let conn = db::open(&db_path)?;
        let subject: String = conn
            .query_row(
                "select name from subject where id = ?1",
                params![subject_id],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| subject_id.clone());
        let value = crate::diagram::generate(
            &conn,
            &http,
            &subject,
            &stem,
            focus.as_deref().unwrap_or(""),
            mode.as_deref().unwrap_or("general"),
        )?;
        let svg = value
            .get("svg")
            .and_then(|node| node.as_str())
            .ok_or_else(|| "制图结果不完整".to_string())?
            .to_string();

        // 落到 tmp：文件名带 .svg，stage_figures 会用 sniff_image 识别（已支持 svg）。
        let dir = data_dir.join("tmp");
        std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;
        let path = dir.join(format!("diagram-{}.svg", uuid::Uuid::new_v4()));
        std::fs::write(&path, svg.as_bytes()).map_err(|e| format!("保存图形失败：{e}"))?;

        Ok(DiagramResult {
            svg,
            data_url: crate::diagram::svg_data_url(
                &std::fs::read_to_string(&path).map_err(|e| e.to_string())?,
            ),
            path: path.to_string_lossy().to_string(),
        })
    })
    .await
}

// ---------------------------------------------------------------------------
// 导入
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn import_scan(state: State<'_, AppState>, path: String) -> Result<import::Preview, String> {
    let conn = lock(&state)?;
    let existing = repo::fingerprints(&conn)?;
    drop(conn);
    let preview = import::scan(std::path::Path::new(&path), &state.data_dir, &existing)?;
    register_preview(&state, preview)
}

#[tauri::command]
pub fn import_scan_text(
    state: State<'_, AppState>,
    text: String,
) -> Result<import::Preview, String> {
    let conn = lock(&state)?;
    let existing = repo::fingerprints(&conn)?;
    drop(conn);
    let preview = import::scan_text(&text, &state.data_dir, &existing)?;
    register_preview(&state, preview)
}

#[tauri::command]
pub fn import_commit(
    state: State<'_, AppState>,
    preview_id: String,
    rows: Vec<usize>,
    import_duplicates: bool,
) -> Result<import::CommitResult, String> {
    let preview = {
        let previews = state
            .previews
            .lock()
            .map_err(|_| "预览缓存忙".to_string())?;
        previews
            .get(&preview_id)
            .cloned()
            .ok_or_else(|| "预览已过期，请重新选择文件".to_string())?
    };
    let mut conn = lock(&state)?;
    let result = import::commit(
        &mut conn,
        &state.data_dir,
        &preview,
        &rows,
        import_duplicates,
    );
    drop(conn);
    if let Ok(mut previews) = state.previews.lock() {
        if let Some(removed) = previews.remove(&preview_id) {
            if let Some(workspace) = removed.workspace {
                let _ = std::fs::remove_dir_all(workspace);
            }
        }
    }
    result
}

#[tauri::command]
pub fn import_discard(state: State<'_, AppState>, preview_id: String) -> Result<(), String> {
    if let Ok(mut previews) = state.previews.lock() {
        if let Some(preview) = previews.remove(&preview_id) {
            if let Some(workspace) = preview.workspace {
                let _ = std::fs::remove_dir_all(workspace);
            }
        }
    }
    Ok(())
}

fn register_preview(state: &AppState, preview: import::Preview) -> Result<import::Preview, String> {
    let mut previews = state
        .previews
        .lock()
        .map_err(|_| "预览缓存忙".to_string())?;
    // 只留最近 10 份预览，超了先清掉旧的临时目录，避免内存与磁盘里堆 ZIP。
    if previews.len() >= 10 {
        for (_, old) in previews.drain() {
            if let Some(workspace) = old.workspace {
                let _ = std::fs::remove_dir_all(workspace);
            }
        }
    }
    previews.insert(preview.id.clone(), preview.clone());
    Ok(preview)
}

#[tauri::command]
pub fn import_batches(state: State<'_, AppState>) -> Result<Vec<BatchInfo>, String> {
    let conn = lock(&state)?;
    let mut stmt = conn
        .prepare(
            "select b.id, b.filename, b.source_format, b.total, b.imported, b.skipped, b.failed,
                    b.status, b.created_at,
                    (select count(*) from import_batch_question q
                     join question qq on qq.id = q.question_id
                     where q.batch_id = b.id and qq.deleted_at is null)
             from import_batch b order by b.created_at desc limit 20",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(BatchInfo {
                id: row.get(0)?,
                filename: row.get(1)?,
                source_format: row.get(2)?,
                total: row.get(3)?,
                imported: row.get(4)?,
                skipped: row.get(5)?,
                failed: row.get(6)?,
                status: row.get(7)?,
                created_at: row.get(8)?,
                remaining: row.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

#[tauri::command]
pub fn import_errors(
    state: State<'_, AppState>,
    batch_id: String,
) -> Result<Vec<ImportError>, String> {
    let conn = lock(&state)?;
    let mut stmt = conn
        .prepare(
            "select row_no, code, message from import_error where batch_id = ?1 order by row_no",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([batch_id], |row| {
            Ok(ImportError {
                row_no: row.get(0)?,
                code: row.get(1)?,
                message: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

#[tauri::command]
pub fn import_undo(state: State<'_, AppState>, batch_id: String) -> Result<usize, String> {
    let mut conn = lock(&state)?;
    import::undo(&mut conn, &batch_id)
}

// ---------------------------------------------------------------------------
// 模型
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn models_list(state: State<'_, AppState>) -> Result<Vec<ModelSpec>, String> {
    let conn = lock(&state)?;
    llm::list_models(&conn)
}

#[tauri::command]
pub fn model_save(state: State<'_, AppState>, input: ModelInput) -> Result<String, String> {
    let mut conn = lock(&state)?;
    llm::save_model(&mut conn, &input)
}

#[tauri::command]
pub fn model_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut conn = lock(&state)?;
    llm::delete_model(&mut conn, &id)
}

#[tauri::command]
pub async fn model_test(state: State<'_, AppState>, id: String) -> Result<TestResult, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    blocking(move || {
        let conn = db::open(&db_path)?;
        llm::test(&conn, &http, &id)
    })
    .await
}

/// 拉服务商的模型列表（`GET {base}/models`）。Key 用表单里的，没有就用已保存模型的。
#[tauri::command]
pub async fn model_list_remote(
    state: State<'_, AppState>,
    base_url: String,
    api_key: Option<String>,
    model_id: Option<String>,
) -> Result<Vec<String>, String> {
    let http = state.http.clone();
    blocking(move || llm::list_remote_models(&http, &base_url, api_key, model_id.as_deref())).await
}

#[tauri::command]
pub fn routes_get(state: State<'_, AppState>) -> Result<Value, String> {
    let conn = lock(&state)?;
    llm::routes(&conn)
}

#[tauri::command]
pub fn route_set(
    state: State<'_, AppState>,
    purpose: String,
    model_ids: Vec<String>,
) -> Result<(), String> {
    let mut conn = lock(&state)?;
    llm::set_route(&mut conn, &purpose, &model_ids)
}

// ---------------------------------------------------------------------------
// 任务
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn job_start(state: State<'_, AppState>, kind: String, input: Value) -> Result<String, String> {
    let conn = lock(&state)?;
    let ctx = JobContext::from_state(&state);
    jobs::start(ctx, state.http.clone(), &conn, &kind, input)
}

#[tauri::command]
pub fn job_get(state: State<'_, AppState>, id: String) -> Result<Job, String> {
    let conn = lock(&state)?;
    jobs::get(&conn, &id)
}

#[tauri::command]
pub fn job_recent(state: State<'_, AppState>) -> Result<Vec<Job>, String> {
    let conn = lock(&state)?;
    jobs::recent(&conn)
}

#[tauri::command]
pub fn job_cancel(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    jobs::cancel(&conn, &id);
    Ok(())
}

#[tauri::command]
pub fn job_commit(
    state: State<'_, AppState>,
    job_id: String,
    rows: Vec<usize>,
) -> Result<Value, String> {
    let mut conn = lock(&state)?;
    let data_dir = state.data_dir.clone();
    jobs::commit(&mut conn, &data_dir, &job_id, &rows)
}

/// 预览里就地改一道题（还没存草稿的时候）。
#[tauri::command]
pub fn job_item_update(
    state: State<'_, AppState>,
    job_id: String,
    index: usize,
    patch: Value,
) -> Result<Value, String> {
    let conn = lock(&state)?;
    jobs::update_item(&conn, &job_id, index, &patch)
}

// ---------------------------------------------------------------------------
// 同步
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> Result<SyncStatus, String> {
    let conn = lock(&state)?;
    sync::status(&conn)
}

#[tauri::command]
pub async fn sync_login(
    state: State<'_, AppState>,
    server: String,
    username: String,
    password: String,
    device_name: String,
) -> Result<SyncStatus, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    blocking(move || {
        let mut conn = db::open(&db_path)?;
        sync::login(
            &mut conn,
            &http,
            &server,
            &username,
            &password,
            &device_name,
        )
    })
    .await
}

#[tauri::command]
pub async fn sync_logout(state: State<'_, AppState>) -> Result<SyncStatus, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    blocking(move || {
        let mut conn = db::open(&db_path)?;
        sync::logout(&mut conn, &http)?;
        sync::status(&conn)
    })
    .await
}

#[tauri::command]
pub async fn sync_push(state: State<'_, AppState>) -> Result<Value, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    blocking(move || {
        let mut conn = db::open(&db_path)?;
        let pushed = sync::push(&mut conn, &http)?;
        Ok(json!({"pushed": pushed}))
    })
    .await
}

#[tauri::command]
pub async fn sync_pull(state: State<'_, AppState>) -> Result<Value, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    blocking(move || {
        let mut conn = db::open(&db_path)?;
        let pulled = sync::pull(&mut conn, &http)?;
        Ok(json!({"pulled": pulled}))
    })
    .await
}

#[tauri::command]
pub fn sync_retry(state: State<'_, AppState>) -> Result<(), String> {
    let conn = lock(&state)?;
    sync::retry_failed(&conn)
}

// ---------------------------------------------------------------------------
// 中心作答照片（阶段 B）
// ---------------------------------------------------------------------------

/// 中心里这位教师自己的班级。用来把本地作业绑到中心作业。
#[tauri::command]
pub async fn center_classes(state: State<'_, AppState>) -> Result<Value, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    blocking(move || {
        let conn = db::open(&db_path)?;
        let rows = crate::center_assets::classes(&conn, &http)?;
        serde_json::to_value(rows).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn center_assignments(
    state: State<'_, AppState>,
    class_id: String,
) -> Result<Value, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    blocking(move || {
        let conn = db::open(&db_path)?;
        let rows = crate::center_assets::assignments(&conn, &http, &class_id)?;
        serde_json::to_value(rows).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub fn center_binding(
    state: State<'_, AppState>,
    assignment_id: String,
) -> Result<Option<crate::center_assets::Binding>, String> {
    let conn = lock(&state)?;
    crate::center_assets::binding(&conn, &assignment_id)
}

#[tauri::command]
pub fn center_bind_assignment(
    state: State<'_, AppState>,
    assignment_id: String,
    center_assignment_id: String,
    center_title: String,
) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::center_assets::bind(&conn, &assignment_id, &center_assignment_id, &center_title)
}

/// 把本地学生绑到中心账号（或解绑传 null）。
///
/// 拉取照片时如果中心姓名与班级名单对不上，老师在这里指一次；绑定后一律按 id 认人，
/// 学生改名也不会再错位。
#[tauri::command]
pub fn student_bind_center(
    state: State<'_, AppState>,
    student_id: String,
    center_user_id: Option<String>,
) -> Result<(), String> {
    let conn = lock(&state)?;
    let value = center_user_id.filter(|id| !id.trim().is_empty());
    let changed = conn
        .execute(
            "update local_student set center_user_id = ?2, updated_at = ?3
              where id = ?1 and deleted_at is null",
            rusqlite::params![student_id, value, crate::text::now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("学生不存在或已删除".into());
    }
    Ok(())
}

/// 拉取这份作业全部学生的作答照片。
#[tauri::command]
pub async fn center_pull_assets(
    state: State<'_, AppState>,
    assignment_id: String,
) -> Result<crate::center_assets::PullResult, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    let data_dir = state.data_dir.clone();
    blocking(move || {
        let conn = db::open(&db_path)?;
        crate::center_assets::pull(&conn, &data_dir, &http, &assignment_id)
    })
    .await
}

// ---------------------------------------------------------------------------
// 事件：全局热键
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn emit_capture(app: AppHandle) -> Result<(), String> {
    app.emit("nb://capture", ()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 内置浏览器
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn browser_open(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    subject_id: Option<String>,
) -> Result<(), String> {
    if let Some(subject) = subject_id {
        if let Ok(mut slot) = state.browser_subject.lock() {
            *slot = subject;
        }
    }
    browser::open(&app, &url)?;
    // 复用已有窗口时，站内可能是 SPA 跳转，补一次注入让按钮跟上新页面。
    browser::rescan(&app);
    Ok(())
}

/// 主窗里换了学科：接下来在网页里抓到的题都归到这个学科。
#[tauri::command]
pub fn browser_set_subject(state: State<'_, AppState>, subject_id: String) -> Result<(), String> {
    let mut slot = state
        .browser_subject
        .lock()
        .map_err(|_| "会话状态忙".to_string())?;
    *slot = subject_id;
    Ok(())
}

/// 注入脚本抓到结构化题目后调这里：归一化成草稿任务，再通知主窗核对。
#[tauri::command]
pub fn browser_collect(
    app: AppHandle,
    state: State<'_, AppState>,
    items: Vec<Value>,
) -> Result<Value, String> {
    // 来源只认内嵌视图的真实地址，不能由远程网页伪造。
    let source = browser::site_view(&app)
        .and_then(|view| view.url().ok())
        .map(|url| url.to_string())
        .filter(|url| !url.is_empty())
        .ok_or("还没读到当前网页地址，请刷新页面后重试")?;
    let source = browser::normalize_url(&source)?;
    let browser::PreparedWebItems {
        items: prepared,
        total,
        truncated,
    } = browser::prepare_web_items(items, &source)?;

    let conn = lock(&state)?;
    let chosen = state
        .browser_subject
        .lock()
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let (subject_id, _, _) = jobs::resolve_subject(&conn, &json!({"subjectId": chosen}));
    let count = prepared.len();
    let job_id = jobs::create_ready_job(
        &conn,
        "capture_web",
        json!({"sourceUrl": source}),
        &prepared,
        &subject_id,
    )?;
    drop(conn);
    let payload = json!({"jobId": job_id, "count": count, "total": total, "truncated": truncated});
    app.emit("nb://browser-questions", payload.clone())
        .map_err(|e| e.to_string())?;
    Ok(payload)
}

/// 工具条「识别本页」：让页面里的脚本重新扫一遍并收集。
#[tauri::command]
pub fn browser_extract_questions(app: AppHandle) -> Result<(), String> {
    let view = browser::site_view(&app).ok_or("内置浏览器还没打开，先打开一个试题网址")?;
    // 老页面可能还没注入新脚本；__nbOverlay 守卫会让重复注入变成空操作。
    view.eval(browser::OVERLAY_SCRIPT)
        .map_err(|e| format!("注入取题脚本失败：{e}"))?;
    view.eval(
        "setTimeout(function(){ if (window.__nbCollectAll) window.__nbCollectAll(); }, 250);",
    )
    .map_err(|e| format!("启动取题失败：{e}"))?;
    Ok(())
}

/// 本机 OCR 是否可用；录入页据此决定默认用哪个识别引擎。
#[tauri::command]
pub fn ocr_status() -> Value {
    ocr::status()
}

#[tauri::command]
pub fn browser_close(app: AppHandle) -> Result<(), String> {
    browser::close(&app);
    Ok(())
}

/// 主窗把「网页取题」的占位框位置报上来，内嵌视图跟着摆。
#[tauri::command]
pub fn browser_bounds(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    browser::set_bounds(&app, x, y, width, height)
}

/// 切走「网页取题」时藏起内嵌视图，回来再显示，登录态不丢。
#[tauri::command]
pub fn browser_hide(app: AppHandle) -> Result<(), String> {
    browser::hide(&app);
    Ok(())
}

/// 控制条截完图后，把图片路径交给主窗的录入页去识别。
#[tauri::command]
pub fn emit_capture_image(app: AppHandle, path: String) -> Result<(), String> {
    app.emit("nb://capture-image", path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn browser_status(app: AppHandle) -> Result<serde_json::Value, String> {
    // 带上页面当前真实地址：用户在站内翻页后，来源记忆才不会记成入口页。
    let url = browser::site_view(&app)
        .and_then(|view| view.url().ok())
        .map(|url| url.to_string())
        .unwrap_or_default();
    Ok(json!({"open": browser::is_open(&app), "url": url}))
}

/// 注入脚本读取浏览器页面 DOM，提取题目文本。
/// 提取结果通过 `emit_browser_text` 回传，主窗录入页收到后走文字识别链路。
#[tauri::command]
pub fn browser_extract_text(app: AppHandle) -> Result<(), String> {
    let view = browser::site_view(&app).ok_or("内置浏览器还没打开，先打开一个试题网址")?;

    // 注入脚本：遍历 DOM 提取正文文本，然后调 emit_browser_text 回传。
    // 启发式选内容容器——题库站点的题目通常在 .question / .topic / article 等容器里，
    // 选不到就退到 body。保留段落与编号结构，方便下游切题。
    let script = r#"
    (function() {
        try {
            // 1. 找主内容容器：优先题目相关的 class / id，其次语义标签，最后 body
            var selectors = [
                '.question', '.topic', '[class*="question"]', '[class*="topic"]',
                '[class*="exam"]', '[class*="test"]', '[class*="paper"]',
                '.content', '#content', '.main-content', 'article', 'main'
            ];
            var root = null;
            for (var i = 0; i < selectors.length; i++) {
                var el = document.querySelector(selectors[i]);
                if (el && el.textContent.trim().length > 50) { root = el; break; }
            }
            if (!root) root = document.body;

            // 2. 去掉脚本、样式、导航、页脚等噪声
            var clone = root.cloneNode(true);
            var noise = clone.querySelectorAll('script, style, nav, header, footer, aside, .ad, .advertisement, .sidebar, .menu, .breadcrumb, [class*="ad-"], [class*="banner"], [class*="comment"], [class*="footer"], [class*="header"], [class*="nav"], [class*="sidebar"]');
            for (var j = 0; j < noise.length; j++) noise[j].remove();

            // 3. 提取公式源码优先：MathJax / KaTeX
            var maths = clone.querySelectorAll('annotation[encoding="application/x-tex"], [data-tex], .MathJax, .katex-mathml, .katex');
            for (var k = 0; k < maths.length; k++) {
                var m = maths[k];
                var tex = m.getAttribute('data-tex') || m.getAttribute('data-original-text') || '';
                if (!tex && m.tagName.toLowerCase() === 'annotation') tex = m.textContent;
                if (!tex) {
                    var ann = m.querySelector('annotation[encoding="application/x-tex"]');
                    if (ann) tex = ann.textContent;
                }
                if (tex) {
                    var span = document.createElement('span');
                    span.textContent = ' $' + tex.trim() + '$ ';
                    m.parentNode.replaceChild(span, m);
                }
            }

            // 4. 按块级元素切段落，保留编号结构
            var blocks = clone.querySelectorAll('p, div, li, h1, h2, h3, h4, h5, h6, td, th, pre, blockquote');
            var lines = [];
            var seen = {};
            for (var b = 0; b < blocks.length; b++) {
                var t = blocks[b].textContent.trim();
                if (t && !seen[t]) { lines.push(t); seen[t] = true; }
            }
            var text = lines.join('\n');
            if (!text || text.trim().length < 10) {
                text = clone.textContent.replace(/\s+/g, ' ').trim();
            }

            // 5. 回传
            window.__TAURI_INTERNALS__.invoke('emit_browser_text', { text: text });
        } catch(e) {
            window.__TAURI_INTERNALS__.invoke('emit_browser_text', { text: '', error: String(e) });
        }
    })();
    "#;

    view.eval(script)
        .map_err(|e| format!("注入提取脚本失败：{e}"))?;
    Ok(())
}

/// 接收浏览器 DOM 提取结果，转发给主窗录入页。
#[tauri::command]
pub fn emit_browser_text(
    app: AppHandle,
    text: String,
    error: Option<String>,
) -> Result<(), String> {
    let failure = error.map(|err| format!("页面提取失败：{err}")).or_else(|| {
        text.trim()
            .is_empty()
            .then(|| "页面里没提取到文字，可能是整页图片或被登录墙挡住了".into())
    });
    if let Some(message) = failure {
        // 工具条监听这个事件后立刻恢复按钮；原始 IPC 仍返回错误给页面脚本。
        let _ = app.emit("nb://browser-error", json!({"message": message.clone()}));
        return Err(message);
    }
    app.emit("nb://browser-text", text)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// 智能裁剪
// ---------------------------------------------------------------------------

/// 对一张整卷图找候选文字块（题），返回原图像素坐标的框。
#[tauri::command]
pub fn smart_regions(path: String) -> Result<Vec<smart::Region>, String> {
    let image = image::ImageReader::open(std::path::Path::new(&path))
        .map_err(|e| format!("读不了图片：{e}"))?
        .with_guessed_format()
        .map_err(|e| format!("认不出图片格式：{e}"))?
        .decode()
        .map_err(|e| format!("解析图片失败：{e}"))?;
    Ok(smart::detect_regions(&image))
}

/// 按候选框裁下一块，落到 tmp 供识别任务读取。
#[tauri::command]
pub fn smart_crop(
    state: State<'_, AppState>,
    path: String,
    region: smart::Region,
) -> Result<CaptureResult, String> {
    let image = image::ImageReader::open(std::path::Path::new(&path))
        .map_err(|e| format!("读不了图片：{e}"))?
        .with_guessed_format()
        .map_err(|e| format!("认不出图片格式：{e}"))?
        .decode()
        .map_err(|e| format!("解析图片失败：{e}"))?;
    let cropped = smart::crop_region(&image, &region);
    let dir = state.data_dir.join("tmp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;
    let out = dir.join(format!("crop-{}.png", uuid::Uuid::new_v4()));
    cropped
        .save(&out)
        .map_err(|e| format!("保存裁剪图失败：{e}"))?;
    capture::from_path(&out)
}

/// 跨页题：把相邻两页竖着拼成一张图，再走同一条识别链路。
#[tauri::command]
pub fn image_stitch(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<CaptureResult, String> {
    smart::stitch(&paths, &state.data_dir)
}

// ---------------------------------------------------------------------------
// 采集来源记忆与连拍队列（P0 取题快车道）
// ---------------------------------------------------------------------------
#[tauri::command]
pub fn capture_sources(
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::CaptureSource>, String> {
    let conn = lock(&state)?;
    crate::capture_store::sources(&conn)
}

#[tauri::command]
pub fn capture_source_remember(
    state: State<'_, AppState>,
    url: String,
    label: String,
) -> Result<crate::models::CaptureSource, String> {
    let conn = lock(&state)?;
    let id = crate::capture_store::remember(&conn, &url, &label)?;
    crate::capture_store::sources(&conn)?
        .into_iter()
        .find(|source| source.id == id)
        .ok_or_else(|| "来源没记住".into())
}

#[tauri::command]
pub fn capture_source_star(
    state: State<'_, AppState>,
    id: String,
    starred: bool,
) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::capture_store::star(&conn, &id, starred)
}

#[tauri::command]
pub fn capture_source_forget(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::capture_store::forget(&conn, &id)
}

#[tauri::command]
pub fn capture_queue_list(
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::QueueItem>, String> {
    let conn = lock(&state)?;
    crate::capture_store::list(&conn)
}

/// 把一张图加入连拍队列。sourceUrl 若有，会记到来源表并自动关联。
#[tauri::command]
pub fn capture_queue_add(
    state: State<'_, AppState>,
    image_path: String,
    source_url: String,
) -> Result<crate::models::QueueItem, String> {
    let conn = lock(&state)?;
    let source_id = if source_url.trim().is_empty() {
        None
    } else {
        Some(crate::capture_store::remember(&conn, &source_url, "")?)
    };
    crate::capture_store::enqueue(&conn, &image_path, source_id.as_deref(), &source_url)
}

#[tauri::command]
pub fn capture_queue_retry(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::capture_store::retry(&conn, &id)
}

#[tauri::command]
pub fn capture_queue_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::capture_store::remove(&conn, &id)
}

#[tauri::command]
pub fn capture_queue_clear_done(state: State<'_, AppState>) -> Result<usize, String> {
    let conn = lock(&state)?;
    crate::capture_store::clear_done(&conn)
}

// ---------------------------------------------------------------------------
// 教学闭环：学生 / 班级 / 作业 / 作答
// ---------------------------------------------------------------------------

fn teacher_name(conn: &rusqlite::Connection) -> String {
    conn.query_row(
        "select case when trim(display_name) = '' then username else display_name end
           from local_user where id = 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .unwrap_or_else(|_| "老师".to_string())
}

#[tauri::command]
pub fn student_list(
    state: State<'_, AppState>,
    keyword: Option<String>,
    status: Option<String>,
) -> Result<Vec<teaching::Student>, String> {
    let conn = lock(&state)?;
    teaching::student_list(&conn, keyword.as_deref(), status.as_deref())
}

#[tauri::command]
pub fn student_save(
    state: State<'_, AppState>,
    input: teaching::StudentInput,
) -> Result<teaching::Student, String> {
    let conn = lock(&state)?;
    let id = teaching::student_save(&conn, &input)?;
    teaching::student_get(&conn, &id)
}

#[tauri::command]
pub fn student_archive(
    state: State<'_, AppState>,
    id: String,
    archived: bool,
) -> Result<(), String> {
    let conn = lock(&state)?;
    teaching::student_archive(&conn, &id, archived)
}

#[tauri::command]
pub fn student_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    teaching::student_delete(&conn, &id)
}

#[tauri::command]
pub fn student_import_preview(
    state: State<'_, AppState>,
    text: String,
) -> Result<teaching::ImportPreview, String> {
    let conn = lock(&state)?;
    teaching::student_import_preview(&conn, &text)
}

#[tauri::command]
pub fn student_import_commit(
    state: State<'_, AppState>,
    text: String,
) -> Result<teaching::ImportResult, String> {
    let mut conn = lock(&state)?;
    teaching::student_import_commit(&mut conn, &text)
}

#[tauri::command]
pub fn class_list(state: State<'_, AppState>) -> Result<Vec<teaching::ClassRow>, String> {
    let conn = lock(&state)?;
    teaching::class_list(&conn)
}

#[tauri::command]
pub fn class_get(state: State<'_, AppState>, id: String) -> Result<teaching::ClassRow, String> {
    let conn = lock(&state)?;
    teaching::class_get(&conn, &id)
}

#[tauri::command]
pub fn class_save(
    state: State<'_, AppState>,
    input: teaching::ClassInput,
) -> Result<teaching::ClassRow, String> {
    let conn = lock(&state)?;
    let id = teaching::class_save(&conn, &input)?;
    teaching::class_get(&conn, &id)
}

#[tauri::command]
pub fn class_close(state: State<'_, AppState>, id: String, closed: bool) -> Result<(), String> {
    let conn = lock(&state)?;
    teaching::class_close(&conn, &id, closed)
}

#[tauri::command]
pub fn class_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    teaching::class_delete(&conn, &id)
}

#[tauri::command]
pub fn class_regenerate_code(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let conn = lock(&state)?;
    teaching::class_regenerate_code(&conn, &id)
}

#[tauri::command]
pub fn class_members(
    state: State<'_, AppState>,
    class_id: String,
) -> Result<Vec<teaching::Student>, String> {
    let conn = lock(&state)?;
    teaching::class_members(&conn, &class_id)
}

#[tauri::command]
pub fn class_add_members(
    state: State<'_, AppState>,
    class_id: String,
    student_ids: Vec<String>,
) -> Result<i64, String> {
    let mut conn = lock(&state)?;
    teaching::class_add_members(&mut conn, &class_id, &student_ids)
}

#[tauri::command]
pub fn class_remove_member(
    state: State<'_, AppState>,
    class_id: String,
    student_id: String,
) -> Result<(), String> {
    let conn = lock(&state)?;
    teaching::class_remove_member(&conn, &class_id, &student_id)
}

#[tauri::command]
pub fn assignment_list(
    state: State<'_, AppState>,
    class_id: Option<String>,
) -> Result<Vec<teaching::AssignmentRow>, String> {
    let conn = lock(&state)?;
    teaching::assignment_list(&conn, class_id.as_deref())
}

#[tauri::command]
pub fn assignment_get(
    state: State<'_, AppState>,
    id: String,
) -> Result<teaching::AssignmentDetail, String> {
    let conn = lock(&state)?;
    teaching::assignment_detail(&conn, &id)
}

#[tauri::command]
pub fn assignment_save(
    state: State<'_, AppState>,
    input: teaching::AssignmentInput,
) -> Result<teaching::AssignmentRow, String> {
    let mut conn = lock(&state)?;
    let id = teaching::assignment_save(&mut conn, &input)?;
    teaching::assignment_get(&conn, &id)
}

#[tauri::command]
pub fn assignment_publish(
    state: State<'_, AppState>,
    id: String,
) -> Result<teaching::AssignmentRow, String> {
    let conn = lock(&state)?;
    teaching::assignment_publish(&conn, &id)?;
    teaching::assignment_get(&conn, &id)
}

#[tauri::command]
pub fn assignment_close(
    state: State<'_, AppState>,
    id: String,
) -> Result<teaching::AssignmentRow, String> {
    let conn = lock(&state)?;
    teaching::assignment_close(&conn, &id)?;
    teaching::assignment_get(&conn, &id)
}

#[tauri::command]
pub fn assignment_reopen(
    state: State<'_, AppState>,
    id: String,
) -> Result<teaching::AssignmentRow, String> {
    let conn = lock(&state)?;
    teaching::assignment_reopen(&conn, &id)?;
    teaching::assignment_get(&conn, &id)
}

#[tauri::command]
pub fn assignment_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    teaching::assignment_delete(&conn, &id)
}

/// 组卷前看一眼题库有多少可出的题（按知识点 / 难度）。
#[tauri::command]
pub fn compose_pool(
    state: State<'_, AppState>,
    subject_id: Option<String>,
) -> Result<compose::Pool, String> {
    let conn = lock(&state)?;
    compose::pool(&conn, subject_id.as_deref())
}

/// 按「知识点 + 每档题数」生成作业草稿。不落库，前端改完再走 `assignment_save`。
#[tauri::command]
pub fn compose_draft(
    state: State<'_, AppState>,
    input: compose::DraftInput,
) -> Result<compose::Draft, String> {
    let conn = lock(&state)?;
    compose::draft(&conn, &input)
}

/// 换一题：同档、同知识点里另抽一道。
#[tauri::command]
pub fn compose_swap(
    state: State<'_, AppState>,
    input: compose::SwapInput,
) -> Result<Option<compose::ComposedQuestion>, String> {
    let conn = lock(&state)?;
    compose::swap(&conn, &input)
}

/// 讲评清单：按题汇总错题、名单与批改备注。
#[tauri::command]
pub fn review_sheet(
    state: State<'_, AppState>,
    assignment_id: String,
) -> Result<review::ReviewSheet, String> {
    let conn = lock(&state)?;
    review::sheet(&conn, &assignment_id)
}

#[tauri::command]
pub fn attempt_submit(
    state: State<'_, AppState>,
    input: teaching::SubmitInput,
) -> Result<teaching::SubmitResult, String> {
    let mut conn = lock(&state)?;
    teaching::submit(&mut conn, &input)
}

#[tauri::command]
pub fn attempt_sheet(
    state: State<'_, AppState>,
    assignment_id: String,
    student_id: String,
) -> Result<teaching::AttemptSheet, String> {
    let conn = lock(&state)?;
    teaching::attempt_sheet(&conn, &assignment_id, &student_id)
}

#[tauri::command]
pub fn attempt_grade(
    state: State<'_, AppState>,
    input: teaching::GradeInput,
) -> Result<(), String> {
    let mut conn = lock(&state)?;
    let teacher = teacher_name(&conn);
    teaching::grade(&mut conn, &input, &teacher)
}

#[tauri::command]
pub fn attempt_batch_grade(
    state: State<'_, AppState>,
    items: Vec<teaching::GradeInput>,
) -> Result<i64, String> {
    let mut conn = lock(&state)?;
    let teacher = teacher_name(&conn);
    teaching::batch_grade(&mut conn, &items, &teacher)
}

#[tauri::command]
pub fn attempt_clear(
    state: State<'_, AppState>,
    assignment_id: String,
    student_id: String,
) -> Result<i64, String> {
    let conn = lock(&state)?;
    teaching::attempt_clear(&conn, &assignment_id, &student_id)
}

#[tauri::command]
pub fn teaching_overview(state: State<'_, AppState>) -> Result<Value, String> {
    let conn = lock(&state)?;
    teaching::overview(&conn)
}

// ---------------------------------------------------------------------------
// 学情与错题本
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn insights_class(
    state: State<'_, AppState>,
    class_id: String,
) -> Result<crate::insights::ClassInsight, String> {
    let conn = lock(&state)?;
    crate::insights::class_insight(&conn, &class_id)
}

#[tauri::command]
pub fn insights_student(
    state: State<'_, AppState>,
    student_id: String,
) -> Result<crate::insights::StudentInsight, String> {
    let conn = lock(&state)?;
    crate::insights::student_insight(&conn, &student_id)
}

/// 学生维度的学情名单：一个人一个人地看，不用先进班级。
#[tauri::command]
pub fn insights_roster(
    state: State<'_, AppState>,
    class_id: Option<String>,
    keyword: Option<String>,
) -> Result<crate::insights::Roster, String> {
    let conn = lock(&state)?;
    crate::insights::roster(&conn, class_id.as_deref(), keyword.as_deref())
}

/// 学情诊断：有缓存就读缓存，没有就先用规则版（不联网也能看）。
#[tauri::command]
pub fn insight_advice(
    state: State<'_, AppState>,
    student_id: String,
) -> Result<crate::advice::Advice, String> {
    let conn = lock(&state)?;
    crate::advice::get(&conn, &student_id)
}

/// 生成诊断：先用本地规则算出保底结论，再请模型补「为什么」和「怎么讲」。
/// 走独立连接 + 后台线程，模型慢的时候不占着主库锁。
#[tauri::command]
pub async fn insight_advice_generate(
    state: State<'_, AppState>,
    student_id: String,
    force: Option<bool>,
) -> Result<crate::advice::Advice, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    blocking(move || {
        let conn = db::open(&db_path)?;
        crate::advice::generate(&conn, &http, &student_id, force.unwrap_or(false))
    })
    .await
}

#[tauri::command]
pub fn wrong_book_list(
    state: State<'_, AppState>,
    student_id: String,
    state_filter: Option<String>,
) -> Result<Vec<crate::insights::WrongRow>, String> {
    let conn = lock(&state)?;
    crate::insights::wrong_list(&conn, &student_id, state_filter.as_deref())
}

#[tauri::command]
pub fn wrong_book_review(
    state: State<'_, AppState>,
    input: crate::insights::ReviewInput,
) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::insights::wrong_review(&conn, &input)
}

#[tauri::command]
pub fn wrong_book_remove(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::insights::wrong_remove(&conn, &id)
}

#[tauri::command]
pub fn wrong_book_due(
    state: State<'_, AppState>,
    student_id: Option<String>,
) -> Result<i64, String> {
    let conn = lock(&state)?;
    crate::insights::due_review(&conn, student_id.as_deref())
}

// ---------------------------------------------------------------------------
// 成绩（考试）
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn score_exams(
    state: State<'_, AppState>,
    class_id: Option<String>,
) -> Result<Vec<crate::scores::ExamRow>, String> {
    let conn = lock(&state)?;
    crate::scores::exam_list(&conn, class_id.as_deref())
}

#[tauri::command]
pub fn score_exam_sheet(
    state: State<'_, AppState>,
    exam_id: String,
) -> Result<crate::scores::ExamSheet, String> {
    let conn = lock(&state)?;
    crate::scores::exam_sheet(&conn, &exam_id)
}

#[tauri::command]
pub fn score_exam_save(
    state: State<'_, AppState>,
    input: crate::scores::ExamInput,
) -> Result<String, String> {
    let conn = lock(&state)?;
    crate::scores::exam_save(&conn, &input)
}

#[tauri::command]
pub fn score_exam_delete(state: State<'_, AppState>, exam_id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::scores::exam_delete(&conn, &exam_id)
}

#[tauri::command]
pub fn score_save(
    state: State<'_, AppState>,
    exam_id: String,
    rows: Vec<crate::scores::ScoreInput>,
) -> Result<i64, String> {
    let mut conn = lock(&state)?;
    crate::scores::exam_scores_save(&mut conn, &exam_id, &rows)
}

#[tauri::command]
pub fn score_student_exams(
    state: State<'_, AppState>,
    student_id: String,
) -> Result<Vec<crate::scores::StudentExam>, String> {
    let conn = lock(&state)?;
    crate::scores::student_exams(&conn, &student_id, 12)
}

/// 作业 + 考试合成一条时间轴（统一换成百分制）。
#[tauri::command]
pub fn student_progress(
    state: State<'_, AppState>,
    student_id: String,
    limit: Option<i64>,
) -> Result<crate::progress::Progress, String> {
    let conn = lock(&state)?;
    crate::progress::student_progress(&conn, &student_id, limit.unwrap_or(12))
}

// ---------------------------------------------------------------------------
// 本机存储
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn storage_usage(state: State<'_, AppState>) -> Result<crate::storage::Usage, String> {
    let conn = lock(&state)?;
    crate::storage::usage(&conn, &state.data_dir, &state.db_path)
}

/// `kind`：`tmp` 清临时图，`orphans` 清无主图片，`backups` 清旧备份。
/// 三档都不会碰数据库，也不会碰有题目引用的图。
#[tauri::command]
pub fn storage_clean(
    state: State<'_, AppState>,
    kind: String,
) -> Result<crate::storage::CleanResult, String> {
    let guard = crate::storage::GUARD;
    match kind.as_str() {
        "tmp" => {
            let pinned = {
                let conn = lock(&state)?;
                crate::storage::pinned_tmp(&conn)?
            };
            crate::storage::clean_tmp(&state.data_dir, guard, &pinned)
        }
        "orphans" => {
            let conn = lock(&state)?;
            crate::storage::clean_orphan_assets(&conn, &state.data_dir, guard)
        }
        "backups" => crate::storage::clean_backups(&state.data_dir, crate::storage::KEEP_BACKUPS),
        _ => Err(format!("不认识的清理类别：{kind}")),
    }
}

// ---------------------------------------------------------------------------
// 教师课件（讲义）：库 / 导入 / 编译 / 预览 / 布置 / AI 补题
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn courseware_list(
    state: State<'_, AppState>,
    keyword: Option<String>,
    subject_id: Option<String>,
    source: Option<String>,
) -> Result<Vec<crate::courseware::Courseware>, String> {
    let conn = lock(&state)?;
    crate::courseware::list(
        &conn,
        keyword.as_deref().filter(|value| !value.trim().is_empty()),
        subject_id
            .as_deref()
            .filter(|value| !value.trim().is_empty()),
        source.as_deref().filter(|value| !value.trim().is_empty()),
    )
}

#[tauri::command]
pub fn courseware_import(
    state: State<'_, AppState>,
    input: crate::courseware::ImportRequest,
) -> Result<crate::courseware::ImportResult, String> {
    let conn = lock(&state)?;
    crate::courseware::import(&conn, &state.data_dir, &input)
}

#[tauri::command]
pub fn courseware_update(
    state: State<'_, AppState>,
    input: crate::courseware::CoursewarePatch,
) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::courseware::update_meta(&conn, &input)
}

#[tauri::command]
pub fn courseware_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::courseware::remove(&conn, &id)
}

/// 预览：登记 id → 文件路径，返回自定义协议 URL（iframe 内加载，隔离主窗）。
#[tauri::command]
pub fn courseware_preview(
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::courseware::Preview, String> {
    let conn = lock(&state)?;
    let relative = crate::courseware::file_of(&conn, &id)?.ok_or("课件不在了")?;
    let path = state.data_dir.join("assets").join(&relative);
    if !path.exists() {
        return Err("课件文件不在了，可能被清理了".into());
    }
    let title = crate::courseware::title_of(&conn, &id)?.unwrap_or_default();
    let warnings = crate::courseware::warnings_of(&conn, &id);
    let url = crate::courseware::asset_url(&id);
    crate::courseware::register_preview(&state, &id, &path);
    Ok(crate::courseware::Preview {
        id,
        title,
        url,
        warnings,
    })
}

#[tauri::command]
pub fn courseware_build(
    state: State<'_, AppState>,
    input: crate::courseware::HandoutSpec,
    existing_id: Option<String>,
) -> Result<crate::courseware::BuildOutcome, String> {
    let mut spec = input;
    let conn = lock(&state)?;
    crate::courseware::build(&conn, &state.data_dir, &mut spec, existing_id.as_deref())
}

#[tauri::command]
pub fn courseware_spec(state: State<'_, AppState>, id: String) -> Result<Option<Value>, String> {
    let conn = lock(&state)?;
    crate::courseware::spec_of(&conn, &id)
}

#[tauri::command]
pub fn courseware_prompt(
    input: crate::courseware::prompt::PromptInput,
) -> Result<crate::courseware::prompt::PromptResult, String> {
    crate::courseware::prompt::build(&input)
}

#[tauri::command]
pub fn courseware_image_add(
    state: State<'_, AppState>,
    draft_id: String,
    image_id: String,
    path: String,
) -> Result<crate::courseware::HandoutImage, String> {
    crate::courseware::add_image(&state.data_dir, &draft_id, &image_id, &path)
}

#[tauri::command]
pub fn courseware_image_dir(
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<String, String> {
    crate::courseware::draft_image_dir(&state.data_dir, &draft_id)
        .map(|path| path.to_string_lossy().to_string())
}

/// 整份课件生成走后台线程；原始模型返回与结构化结果一起交给校对台。
#[tauri::command]
pub async fn courseware_ai_generate(
    state: State<'_, AppState>,
    input: crate::courseware::prompt::GenerateInput,
) -> Result<crate::courseware::prompt::GenerateResult, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    let assets = state.data_dir.join("assets");
    blocking(move || {
        let conn = db::open(&db_path)?;
        crate::courseware::prompt::generate(&conn, &http, &assets, &input)
    })
    .await
}

/// 外部智能体只需回传 HandoutSpec JSON，本地图片与来源信息仍以当前草稿为准。
#[tauri::command]
pub fn courseware_parse_draft(
    raw: String,
    base: crate::courseware::HandoutSpec,
) -> Result<crate::courseware::HandoutSpec, String> {
    crate::courseware::prompt::parse_draft(&raw, &base, "外部课件草稿")
}

#[tauri::command]
pub async fn courseware_export(
    state: State<'_, AppState>,
    id: String,
    kind: String,
    path: String,
) -> Result<crate::courseware::ExportOutcome, String> {
    let db_path = state.db_path.clone();
    let data_dir = state.data_dir.clone();
    blocking(move || {
        let conn = db::open(&db_path)?;
        crate::courseware::export(&conn, &data_dir, &id, &kind, Path::new(&path))
    })
    .await
}

/// 批量导入只负责建任务，实际抽取与模型调用在独立后台线程顺序执行。
#[tauri::command]
pub fn courseware_import_start(
    state: State<'_, AppState>,
    input: crate::courseware::ingest::ImportStartInput,
) -> Result<Vec<crate::courseware::ingest::ImportItem>, String> {
    let items = {
        let conn = lock(&state)?;
        crate::courseware::ingest::start(&conn, &state.data_dir, &input)?
    };
    let batch_id = items
        .first()
        .map(|item| item.batch_id.clone())
        .ok_or_else(|| "没有可处理的课件文件".to_string())?;
    let db_path = state.db_path.clone();
    let data_dir = state.data_dir.clone();
    let http = state.http.clone();
    std::thread::spawn(move || {
        if let Err(error) =
            crate::courseware::ingest::process_batch(&db_path, &data_dir, &http, &batch_id)
        {
            eprintln!("课件批量导入后台任务失败：{error}");
        }
    });
    Ok(items)
}

#[tauri::command]
pub fn courseware_import_list(
    state: State<'_, AppState>,
    batch_id: Option<String>,
) -> Result<Vec<crate::courseware::ingest::ImportItem>, String> {
    let conn = lock(&state)?;
    crate::courseware::ingest::list(&conn, batch_id.as_deref())
}

#[tauri::command]
pub fn courseware_import_retry(
    state: State<'_, AppState>,
    id: String,
    use_ai: bool,
    allow_duplicate: bool,
) -> Result<crate::courseware::ingest::ImportItem, String> {
    let item = {
        let conn = lock(&state)?;
        crate::courseware::ingest::retry(&conn, &id, use_ai, allow_duplicate)?
    };
    let db_path = state.db_path.clone();
    let data_dir = state.data_dir.clone();
    let http = state.http.clone();
    std::thread::spawn(move || {
        if let Err(error) =
            crate::courseware::ingest::process_one(&db_path, &data_dir, &http, &id, allow_duplicate)
        {
            eprintln!("课件导入重试失败：{error}");
        }
    });
    Ok(item)
}

#[tauri::command]
pub fn courseware_import_save(
    state: State<'_, AppState>,
    id: String,
    spec: crate::courseware::HandoutSpec,
) -> Result<crate::courseware::ingest::ImportItem, String> {
    let conn = lock(&state)?;
    crate::courseware::ingest::save_draft(&conn, &id, &spec)
}

#[tauri::command]
pub fn courseware_import_commit(
    state: State<'_, AppState>,
    id: String,
    spec: crate::courseware::HandoutSpec,
) -> Result<crate::courseware::BuildOutcome, String> {
    let conn = lock(&state)?;
    crate::courseware::ingest::commit(&conn, &state.data_dir, &id, &spec)
}

#[tauri::command]
pub fn courseware_import_original(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let conn = lock(&state)?;
    crate::courseware::ingest::original_path(&conn, &state.data_dir, &id)
}

#[tauri::command]
pub fn courseware_assign(
    state: State<'_, AppState>,
    input: crate::courseware::AssignRequest,
) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::courseware::assign(&conn, &input)
}

#[tauri::command]
pub fn courseware_unassign(
    state: State<'_, AppState>,
    courseware_id: String,
    class_id: String,
) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::courseware::unassign(&conn, &courseware_id, &class_id)
}

/// 让模型从一道例题出发补变式 / 拔高 / 思维训练题。产出只是草稿，老师过目再编译。
#[tauri::command]
pub async fn courseware_ai_draft(
    state: State<'_, AppState>,
    input: Value,
) -> Result<Value, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    blocking(move || {
        let conn = db::open(&db_path)?;
        let mut cooldowns = std::collections::HashMap::new();
        let raw = llm::complete(
            &conn,
            &http,
            "question",
            &llm::prompts::handout_draft(&input),
            None,
            true,
            4096,
            &mut cooldowns,
        )?;
        llm::parse_json(&raw, "讲义补题")
    })
    .await
}

// ---------------------------------------------------------------------------
// 拍照批改（P3 后半）
// ---------------------------------------------------------------------------

/// 导入一张整页作答照片（拍纸质卷子）。
#[tauri::command]
pub fn grading_photo_add(
    state: State<'_, AppState>,
    input: crate::grading::PhotoInput,
) -> Result<crate::grading::PhotoAsset, String> {
    let conn = lock(&state)?;
    crate::grading::store_page(&conn, &state.data_dir, &input)
}

/// 某学生在一份作业下的全部作答照片。
#[tauri::command]
pub fn grading_photo_list(
    state: State<'_, AppState>,
    student_id: String,
    assignment_id: Option<String>,
) -> Result<Vec<crate::grading::PhotoAsset>, String> {
    let conn = lock(&state)?;
    crate::grading::list(
        &conn,
        &student_id,
        assignment_id.as_deref().filter(|value| !value.is_empty()),
    )
}

/// 某次作答挂着的题级照片。
#[tauri::command]
pub fn grading_photo_by_attempt(
    state: State<'_, AppState>,
    attempt_id: String,
) -> Result<Vec<crate::grading::PhotoAsset>, String> {
    let conn = lock(&state)?;
    crate::grading::list_by_attempt(&conn, &attempt_id)
}

/// 从整页照片切一块出来（题级照片），可先不挂题。
#[tauri::command]
pub fn grading_photo_crop(
    state: State<'_, AppState>,
    input: crate::grading::CropRequest,
) -> Result<crate::grading::PhotoAsset, String> {
    let conn = lock(&state)?;
    crate::grading::crop_from_page(&conn, &state.data_dir, &input)
}

/// 在一张作答照片上找候选题块（自动切题）。
#[tauri::command]
pub fn grading_photo_regions(
    state: State<'_, AppState>,
    asset_id: String,
) -> Result<Vec<crate::smart::Region>, String> {
    let conn = lock(&state)?;
    crate::grading::detect_regions(&conn, &state.data_dir, &asset_id)
}

/// 把一张照片挂到某道题。
#[tauri::command]
pub fn grading_photo_attach(
    state: State<'_, AppState>,
    asset_id: String,
    attempt_id: String,
) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::grading::attach(&conn, &asset_id, &attempt_id)
}

/// 删掉一张作答照片（软删，原图保留在盘上）。
#[tauri::command]
pub fn grading_photo_remove(state: State<'_, AppState>, asset_id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::grading::remove(&conn, &asset_id)
}

/// 读一张作答照片给前端渲染。
#[tauri::command]
pub fn grading_photo_data(state: State<'_, AppState>, asset_id: String) -> Result<String, String> {
    let conn = lock(&state)?;
    crate::grading::read_data(&conn, &state.data_dir, &asset_id)
}

/// 纸质作业代提交：学生没在线交卷时先建一次空提交，照片才有地方挂。
#[tauri::command]
pub fn grading_paper_submit(
    state: State<'_, AppState>,
    assignment_id: String,
    student_id: String,
) -> Result<String, String> {
    let mut conn = lock(&state)?;
    crate::grading::paper_submit(&mut conn, &assignment_id, &student_id)
}

/// AI 抄录手写作答，给建议分与评语。**只返回建议，不写库**——分数由老师确认。
#[tauri::command]
pub async fn grading_transcribe(
    state: State<'_, AppState>,
    asset_id: String,
    attempt_id: String,
) -> Result<crate::grading::Suggestion, String> {
    let http = state.http.clone();
    let db_path = state.db_path.clone();
    let data_dir = state.data_dir.clone();
    blocking(move || {
        let conn = db::open(&db_path)?;
        crate::grading::transcribe_with(&conn, &http, &data_dir, &asset_id, &attempt_id)
    })
    .await
}

/// 开启手机扫码上传：给当前学生起一个临时局域网入口，返回 URL + 二维码 SVG。
#[tauri::command]
pub fn photo_upload_start(
    app: AppHandle,
    state: State<'_, AppState>,
    student_id: String,
    assignment_id: Option<String>,
) -> Result<crate::photo_upload_server::UploadStartInfo, String> {
    crate::photo_upload_server::start(
        &app,
        &state.data_dir,
        &state.db_path,
        &student_id,
        assignment_id.as_deref(),
    )
}

/// 停止手机上传：令牌立即失效，服务线程退出。
#[tauri::command]
pub fn photo_upload_stop() -> Result<(), String> {
    crate::photo_upload_server::stop();
    Ok(())
}

/// 手机上传会话状态（已收张数 / 剩余时间），前端轮询用。
#[tauri::command]
pub fn photo_upload_status() -> Result<crate::photo_upload_server::UploadStatusInfo, String> {
    Ok(crate::photo_upload_server::status())
}

// ---------------------------------------------------------------------------
// 听写台（P0）：OCR 切词 + 拼音校对 + 批次存取。播报走 WebView 的系统语音，不经过 IPC。
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn dictation_ocr_from_path(path: String) -> Result<Value, String> {
    let lines = ocr::recognize(&path)?;
    let items = ocr::dictation_items_from_lines(&lines, &path);
    if items.is_empty() {
        return Err("这张图里没读到听写内容：确认图片方向、清晰度，或手工添加词条".into());
    }
    Ok(json!({"items": items}))
}

#[tauri::command]
pub fn dictation_pinyin(text: String) -> Result<Value, String> {
    Ok(crate::pinyin::annotate(&text))
}

#[tauri::command]
pub fn dictation_set_save(state: State<'_, AppState>, input: Value) -> Result<String, String> {
    let mut conn = lock(&state)?;
    crate::dictation::save_set(&mut conn, &state.data_dir, &input)
}

#[tauri::command]
pub fn dictation_set_list(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    let conn = lock(&state)?;
    crate::dictation::list_sets(&conn)
}

#[tauri::command]
pub fn dictation_set_get(state: State<'_, AppState>, id: String) -> Result<Value, String> {
    let conn = lock(&state)?;
    crate::dictation::get_set(&conn, &id)
}

#[tauri::command]
pub fn dictation_set_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let conn = lock(&state)?;
    crate::dictation::delete_set(&conn, &id)
}

#[cfg(test)]
mod tests {
    use super::DiagramResult;

    /// 前端按 `result.dataUrl` 取图。字段名一旦退回 snake_case，
    /// 弹窗里的 `<img src={undefined}>` 就是一片空白——图明明生成成功了却看不见。
    #[test]
    fn diagram_result_serializes_as_camel_case() {
        let value = serde_json::to_value(DiagramResult {
            svg: "<svg></svg>".into(),
            data_url: "data:image/svg+xml;base64,AAA".into(),
            path: "/tmp/x.svg".into(),
        })
        .unwrap();
        assert!(value.get("dataUrl").is_some(), "前端读的是 dataUrl：{value}");
        assert!(value.get("data_url").is_none(), "不要再吐 snake_case：{value}");
    }
}
