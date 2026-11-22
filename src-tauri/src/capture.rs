//! 读回图片并转成 data URL 给前端预览。
//!
//! 截图本身走 `commands::capture_region`：置顶选区层 + 进程内 CG 抓帧（`screen.rs`），
//! 这里只负责把落盘的图片读回来。落盘路径必须留在 data_dir/tmp 且**不能当场删**：
//! 返回的 CaptureResult.path 会被前端原样传给识别任务
//! （Capture.jsx 的 recognize → startJob('capture', ...)），任务线程
//! jobs.rs::run_capture 要按这个路径 std::fs::read 一次，入库时 attach_capture 还要
//! 再读一次。以前图省事删了系统临时文件，结果「截图后识别」必然报
//! 「读不了截图：No such file or directory」。

use crate::models::CaptureResult;
use base64::Engine;

pub fn from_path(path: &std::path::Path) -> Result<CaptureResult, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读不了图片：{e}"))?;
    let (mime, _) = crate::text::sniff_image(&bytes)
        .ok_or_else(|| "这不是 jpg / png / webp / gif 图片".to_string())?;
    let (width, height) = image::image_dimensions(path).unwrap_or((0, 0));
    Ok(CaptureResult {
        path: path.to_string_lossy().to_string(),
        data_url: format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        ),
        width: width as i64,
        height: height as i64,
    })
}

/// 把前端传来的 data URL 落到数据目录的 tmp 下，供视觉模型读取。
pub fn save_data_url(
    data_url: &str,
    data_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let (meta, payload) = data_url
        .split_once(',')
        .ok_or_else(|| "图片数据不完整".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| format!("图片解码失败：{e}"))?;
    if bytes.len() > 12 * 1024 * 1024 {
        return Err("图片超过 12MB，先裁掉不需要的部分".into());
    }
    let (_, ext) = crate::text::sniff_image(&bytes)
        .ok_or_else(|| "只支持 jpg / png / webp / gif 图片".to_string())?;
    let _ = meta;
    let dir = data_dir.join("tmp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;
    let path = dir.join(format!("capture-{}.{ext}", uuid::Uuid::new_v4()));
    std::fs::write(&path, &bytes).map_err(|e| format!("保存图片失败：{e}"))?;
    Ok(path)
}
