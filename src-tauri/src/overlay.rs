//! 置顶选区层窗口管理。移植自 desktop 壳，动作完全一致：启动时预热一个
//! 全屏、置顶、不参与屏幕捕获的 WebviewWindow，热键路径上只剩挪位 + show。
//!
//! 与 desktop 的差别：状态放在本地客户端自己的 AppState 里；底图临时文件
//! 落在数据目录的 tmp 下（asset 协议放行该目录），不放在系统 TEMP。

use std::sync::atomic::Ordering;
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::state::{AppState, ShotInfo};

const LABEL: &str = "overlay";
const OFFSCREEN: f64 = -40000.0;

fn primary_geometry(app: &AppHandle) -> Result<(f64, f64, f64, f64), String> {
    let monitor = app
        .primary_monitor()
        .map_err(|e| format!("读取显示器信息失败：{e}"))?
        .ok_or_else(|| "找不到可用显示器".to_string())?;
    let scale = monitor.scale_factor();
    let size = monitor.size();
    let position = monitor.position();
    Ok((
        position.x as f64 / scale,
        position.y as f64 / scale,
        size.width as f64 / scale,
        size.height as f64 / scale,
    ))
}

/// 启动时建好选区层、加载页面、跑到全屏尺寸，再收起来。
pub fn prewarm(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    let (x, y, width, height) = primary_geometry(app)?;
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
        .title("选择题目区域")
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .focused(false)
        .position(OFFSCREEN, OFFSCREEN)
        .inner_size(width, height)
        .build()
        .map_err(|e| format!("预热选区层失败：{e}"))?;

    let _ = window.set_background_color(Some(tauri::webview::Color(6, 8, 12, 255)));

    #[cfg(target_os = "macos")]
    if std::env::var("NB_NO_PROTECT").is_err() {
        let _ = window.set_content_protected(true);
    }

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(900)).await;
        if let Some(window) = handle.get_webview_window(LABEL) {
            let picking = handle.state::<AppState>().picking.load(Ordering::SeqCst);
            if !picking {
                let _ = window.hide();
            }
        }
    });

    eprintln!("[local] 选区层预热 {}x{} @ {},{}", width, height, x, y);
    Ok(())
}

pub fn prepare(app: &AppHandle) {
    eval(app, "window.__NB_OVERLAY__&&window.__NB_OVERLAY__.reset();");
}

pub fn present(app: &AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(LABEL)
        .ok_or_else(|| "选区层没预热成功，无法显示".to_string())?;
    let (x, y, width, height) = primary_geometry(app)?;
    let same_size = window
        .inner_size()
        .map(|size| {
            let scale = window.scale_factor().unwrap_or(1.0);
            (size.width as f64 / scale - width).abs() < 1.0
                && (size.height as f64 / scale - height).abs() < 1.0
        })
        .unwrap_or(false);
    let same_place = window
        .outer_position()
        .map(|pos| {
            let scale = window.scale_factor().unwrap_or(1.0);
            (pos.x as f64 / scale - x).abs() < 1.0 && (pos.y as f64 / scale - y).abs() < 1.0
        })
        .unwrap_or(false);
    if !same_place {
        window
            .set_position(tauri::LogicalPosition::new(x, y))
            .map_err(|e| format!("移动选区层失败：{e}"))?;
    }
    if !same_size {
        window
            .set_size(tauri::LogicalSize::new(width, height))
            .map_err(|e| format!("调整选区层尺寸失败：{e}"))?;
    }
    window.show().map_err(|e| format!("显示选区层失败：{e}"))?;
    window
        .set_focus()
        .map_err(|e| format!("选区层抢焦点失败：{e}"))?;
    Ok(())
}

pub fn attach(app: &AppHandle, shot: ShotInfo) {
    let state = app.state::<AppState>();
    if let Ok(mut slot) = state.pending_shot.lock() {
        *slot = Some(shot.clone());
    }
    let path = serde_json::json!(&shot.shot_path).to_string();
    let script = format!(
        "window.__NB_OVERLAY__&&window.__NB_OVERLAY__.shoot({path}, {}, {});",
        shot.width, shot.height
    );
    eval(app, &script);
}

pub fn dismiss(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

fn eval(app: &AppHandle, script: &str) {
    if let Some(window) = app.get_webview_window(LABEL) {
        if let Err(err) = window.eval(script) {
            eprintln!("[local] 选区层脚本执行失败：{err}");
        }
    }
}

pub fn mark_ready(app: &AppHandle) {
    app.state::<AppState>()
        .overlay_ready
        .store(true, Ordering::SeqCst);
}

pub fn take_pending(app: &AppHandle) -> Option<ShotInfo> {
    app.state::<AppState>()
        .pending_shot
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
}
