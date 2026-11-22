//! 应用状态：数据目录、数据库连接、导入预览缓存、模型冷却表。

use crate::db as database;
use crate::import::Preview;
use rusqlite::Connection;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

/// 选区层选完之后回传的结果：框在哪、画了什么标注、是否只要复制。
pub enum Pick {
    Done {
        rect: Option<(f64, f64, f64, f64)>,
        markup: Option<String>,
        copy_only: bool,
    },
    Cancel,
}

/// 选区层要贴的底图：落盘路径 + 原始像素尺寸。
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotInfo {
    pub shot_path: String,
    pub width: u32,
    pub height: u32,
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub db: Mutex<Connection>,
    pub previews: Mutex<HashMap<String, Preview>>,
    /// 本次会话是否已通过本地登录。纯内存，进程重启即回到未解锁。
    pub unlocked: Mutex<bool>,
    pub http: reqwest::blocking::Client,
    pub search_mode: String,
    pub app_version: String,
    // ---- 置顶选区层 ----
    pub picking: AtomicBool,
    pub pick_tx: Mutex<Option<oneshot::Sender<Pick>>>,
    pub pending_shot: Mutex<Option<ShotInfo>>,
    /// 已登记可预览的课件：id → 磁盘路径。自定义协议只认这张表，不开放目录。
    pub courseware_files: Mutex<HashMap<String, PathBuf>>,
    pub overlay_ready: AtomicBool,
    /// 内置浏览器里抓到的题归到哪个学科。主窗选定后写进来，
    /// 注入脚本调 `browser_collect` 时不用自己带学科。
    pub browser_subject: Mutex<String>,
}

pub const DEFAULT_DIR_NAME: &str = "NeuralKB-Local";

fn pointer_file(handle: &AppHandle) -> Result<PathBuf, String> {
    let dir = handle
        .path()
        .app_config_dir()
        .map_err(|e| format!("找不到配置目录：{e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("建配置目录失败：{e}"))?;
    Ok(dir.join("pointer.json"))
}

pub fn read_data_dir(handle: &AppHandle) -> Option<PathBuf> {
    let file = pointer_file(handle).ok()?;
    let raw = std::fs::read_to_string(file).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    value
        .get("dataDir")
        .and_then(|node| node.as_str())
        .map(PathBuf::from)
}

pub fn default_data_dir(handle: &AppHandle) -> PathBuf {
    handle
        .path()
        .document_dir()
        .map(|dir| dir.join(DEFAULT_DIR_NAME))
        .unwrap_or_else(|_| std::env::temp_dir().join(DEFAULT_DIR_NAME))
}

pub fn write_data_dir(handle: &AppHandle, dir: &Path) -> Result<(), String> {
    let file = pointer_file(handle)?;
    let body = json!({"dataDir": dir.to_string_lossy()});
    std::fs::write(
        file,
        serde_json::to_string_pretty(&body).unwrap_or_default(),
    )
    .map_err(|e| format!("保存数据目录失败：{e}"))
}

pub fn ensure_dirs(dir: &Path) -> Result<(), String> {
    for child in ["", "assets", "tmp", "backup"] {
        let path = if child.is_empty() {
            dir.to_path_buf()
        } else {
            dir.join(child)
        };
        std::fs::create_dir_all(&path).map_err(|e| format!("建目录失败：{e}"))?;
    }
    Ok(())
}

impl AppState {
    pub fn bootstrap(handle: &AppHandle) -> Result<Self, String> {
        let data_dir = read_data_dir(handle).unwrap_or_else(|| default_data_dir(handle));
        ensure_dirs(&data_dir)?;
        let db_path = data_dir.join("neuralkb.db");
        let mut conn = database::open(&db_path)?;
        database::migrate(&mut conn)?;
        database::seed_curriculum(&mut conn)?;
        // Key 的本地仓库放在配置目录：读过一次就不再敲系统钥匙串（macOS 每次重打包都会
        // 重新弹授权框）。配置目录不进 iCloud，Key 不会跟着数据目录被同步出去。
        if let Ok(config_dir) = handle.path().app_config_dir() {
            let _ = std::fs::create_dir_all(&config_dir);
            crate::llm::init_vault(&config_dir);
        }
        // 迁移 0008：回填 llm_model.has_key，之后设置页不再反复弹钥匙串授权。
        let _ = crate::llm::backfill_has_key(&conn);
        let search_mode = database::ensure_search(&conn)?;

        let http = reqwest::blocking::Client::builder()
            .user_agent(format!(
                "NeuralKB-Local/{} ({})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS
            ))
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .map_err(|e| format!("初始化网络失败：{e}"))?;

        Ok(Self {
            data_dir,
            db_path,
            db: Mutex::new(conn),
            previews: Mutex::new(HashMap::new()),
            unlocked: Mutex::new(false),
            http,
            search_mode,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            picking: AtomicBool::new(false),
            pick_tx: Mutex::new(None),
            pending_shot: Mutex::new(None),
            courseware_files: Mutex::new(HashMap::new()),
            overlay_ready: AtomicBool::new(false),
            browser_subject: Mutex::new(String::new()),
        })
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.data_dir.join("assets")
    }
}

/// 后台任务用的上下文：不持有 StateGuard，自己开连接。
#[derive(Clone)]
pub struct JobContext {
    pub db_path: PathBuf,
}

impl JobContext {
    pub fn from_state(state: &AppState) -> Self {
        Self {
            db_path: state.db_path.clone(),
        }
    }
}
