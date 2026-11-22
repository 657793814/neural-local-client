use std::fs;
use std::path::PathBuf;

#[cfg(target_os = "macos")]
use std::process::Command;

use base64::Engine as _;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageReader};

/// 长边上限：跟网页端 cropFrame 保持同一套数，后端上限 12MB 也吃得下。
const MAX_EDGE: u32 = 2000;
/// 长边下限：太小的框放大一档，小字也看得清。
const MIN_EDGE: u32 = 640;
const MAX_UPSCALE: f64 = 2.0;
/// 选区层的预览图长边，只是给人看，不参与裁剪。
const PREVIEW_EDGE: u32 = 1800;

pub struct Shot {
    /// 落盘路径。走进程内抓帧时没有这个文件，就是 None。
    path: Option<PathBuf>,
    pub image: DynamicImage,
}

impl Shot {
    /// 抓主屏一帧。
    ///
    /// 走进程内直取：`screencapture` 是 fork 一个子进程 + 全屏 PNG 编码，实测 380ms，
    /// 而热键到选区层出现的时间预算总共只有一百来毫秒——这一步慢，后面怎么优化都白搭。
    /// `CGDisplayCreateImage` 直接拿显示器帧缓冲，代码里只剩一次拷贝。
    ///
    /// 取不到（老系统、接口行为变了、锁屏）才退回 `screencapture`：
    /// 慢，但至少还能出图，也不会把错误信息变成「什么都没发生」。
    pub fn capture_primary() -> Result<Self, String> {
        #[cfg(target_os = "macos")]
        {
            let started = std::time::Instant::now();
            if let Some(image) = display::grab() {
                eprintln!("[shell] 进程内抓帧 {}ms", started.elapsed().as_millis());
                return Ok(Self {
                    path: None,
                    image: DynamicImage::ImageRgba8(image),
                });
            }
            eprintln!("[shell] 进程内抓帧不可用，回退 screencapture");
        }
        #[cfg(target_os = "macos")]
        {
            Self::capture_via_command()
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err("截图选区暂不支持 Windows：请用「本地文件取题」（图片 / PDF / Word 都支持）或手工录题；配好视觉模型后也可以用「AI 识图」".into())
        }
    }

    /// 兜底路径：系统自带的 screencapture。多屏拼接、DPI、色彩管理都交给系统。
    ///
    /// 非 macOS（阶段一）没有这条兜底：Windows 的抓帧层要到阶段三
    /// （DXGI Desktop Duplication，见 docs/windows-port-plan.md）才实现，
    /// 现在直接报可操作错误，避免「调一个必然不存在的命令」这种隐晦失败。
    #[cfg(target_os = "macos")]
    fn capture_via_command() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("nb-shot-{}.png", stamp()));
        let output = Command::new("/usr/sbin/screencapture")
            .args(["-x", "-m", "-t", "png"])
            .arg(&path)
            .output()
            .map_err(|e| format!("调用截图命令失败：{e}"))?;
        if !output.status.success() {
            // 系统原话是英文的 could not create image from display，单看这句没法判断是权限
            // 还是锁屏，所以把原因和下一步一起写进去。
            let stderr = String::from_utf8_lossy(&output.stderr);
            let raw = stderr.trim();
            return Err(if raw.is_empty() {
                "截图命令没有输出".to_string()
            } else {
                format!("截图失败（{raw}）：多半是「屏幕录制」权限没给，或屏幕正在锁屏/休眠")
            });
        }
        let size = std::fs::metadata(&path)
            .ok()
            .map(|m| m.len())
            .ok_or_else(|| {
                "截图没有生成文件，通常是没给「屏幕录制」权限：系统设置 → 隐私与安全性 → 屏幕录制"
                    .to_string()
            })?;
        if size == 0 {
            return Err("截图是空文件，通常是没有「屏幕录制」权限".to_string());
        }
        let image = ImageReader::open(&path)
            .map_err(|e| format!("读取截图失败：{e}"))?
            .with_guessed_format()
            .map_err(|e| format!("识别截图格式失败：{e}"))?
            .decode()
            .map_err(|e| format!("解析截图失败：{e}"))?;
        Ok(Self {
            path: Some(path),
            image,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.image.dimensions()
    }

    /// 按比例裁切。用比例而不是像素，是为了让选区层不受窗口缩放、DPI 影响。
    pub fn crop(&self, fx: f64, fy: f64, fw: f64, fh: f64) -> DynamicImage {
        let (width, height) = self.image.dimensions();
        let x = (fx.clamp(0.0, 1.0) * width as f64).round() as u32;
        let y = (fy.clamp(0.0, 1.0) * height as f64).round() as u32;
        let mut w = (fw.clamp(0.0, 1.0) * width as f64).round() as u32;
        let mut h = (fh.clamp(0.0, 1.0) * height as f64).round() as u32;
        let x = x.min(width.saturating_sub(1));
        let y = y.min(height.saturating_sub(1));
        w = w.max(1).min(width - x);
        h = h.max(1).min(height - y);
        self.image.crop_imm(x, y, w, h)
    }

    pub fn cleanup(&self) {
        if let Some(path) = self.path.as_ref() {
            let _ = fs::remove_file(path);
        }
    }
}

/// 选区层背景：降采样后的 JPEG。
///
/// 早先这里是返回 data URL 直接塞进 IPC，但这条壳的 WKWebView 对几百 KB 的
/// data URL 图片不参与绘制——页面元素尺寸全对、图片也 load 成功，屏幕上却什么都不显示。
/// 改成落盘 + asset 协议，既绕开这个限制，也不必把 base64 拖过 IPC。
pub fn preview_file(image: &DynamicImage) -> Result<PathBuf, String> {
    // NB_PROBE_RED：把底图换成纯红，用来验证「底图这一层到底有没有被合成」。
    // 底图内容本来就等于当前屏幕，正常时肉眼分不出它画没画。
    let scaled = if std::env::var("NB_PROBE_RED").is_ok() {
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            64,
            40,
            image::Rgb([220, 30, 30]),
        ))
    } else {
        downscale_fast(image, PREVIEW_EDGE)
    };
    let path = std::env::temp_dir().join(format!("nb-overlay-{}.jpg", stamp()));
    let file = fs::File::create(&path).map_err(|e| format!("写选区预览图失败：{e}"))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 86);
    encoder
        .encode_image(&scaled.to_rgb8())
        .map_err(|e| format!("编码选区预览图失败：{e}"))?;
    Ok(path)
}

/// 裁好的那块图和它的字节一起给出来：页面要 data URL，剪贴板要原始字节。
/// 只编码一次——全屏图编两遍就是白花一百多毫秒。
pub fn crop_payload(image: &DynamicImage) -> Result<(String, Vec<u8>), String> {
    let normalized = normalize(image);
    let bytes = encode_bytes(&normalized, 92)?;
    let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok((format!("data:image/jpeg;base64,{base64}"), bytes))
}

fn normalize(image: &DynamicImage) -> DynamicImage {
    let (width, height) = image.dimensions();
    let long = width.max(height);
    if long > MAX_EDGE {
        return downscale_to(image, MAX_EDGE);
    }
    if long < MIN_EDGE && long > 0 {
        let ratio = (MIN_EDGE as f64 / long as f64).min(MAX_UPSCALE);
        let target_w = (width as f64 * ratio).round() as u32;
        let target_h = (height as f64 * ratio).round() as u32;
        return image.resize_exact(target_w.max(1), target_h.max(1), FilterType::CatmullRom);
    }
    image.clone()
}

fn downscale_to(image: &DynamicImage, edge: u32) -> DynamicImage {
    let (width, height) = image.dimensions();
    let long = width.max(height);
    if long <= edge || long == 0 {
        return image.clone();
    }
    let ratio = edge as f64 / long as f64;
    let target_w = (width as f64 * ratio).round().max(1.0) as u32;
    let target_h = (height as f64 * ratio).round().max(1.0) as u32;
    image.resize_exact(target_w, target_h, FilterType::Lanczos3)
}

/// 预览图专用的降采样。
///
/// `resize` 的重采样在全屏图上要两百毫秒，而这只是一张铺在底下给人定位用的背景——
/// 框选的精度靠原图，不靠它。`thumbnail` 是逐块盒式采样，快好几倍，
/// 缩小一倍多的时候肉眼也看不出差别。
fn downscale_fast(image: &DynamicImage, edge: u32) -> DynamicImage {
    let (width, height) = image.dimensions();
    let long = width.max(height);
    if long <= edge || long == 0 {
        return image.clone();
    }
    let ratio = edge as f64 / long as f64;
    let target_w = (width as f64 * ratio).round().max(1.0) as u32;
    let target_h = (height as f64 * ratio).round().max(1.0) as u32;
    DynamicImage::ImageRgba8(image::imageops::thumbnail(image, target_w, target_h))
}

fn encode_bytes(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buffer, quality);
    encoder
        .encode_image(&image.to_rgb8())
        .map_err(|e| format!("编码 JPEG 失败：{e}"))?;
    Ok(buffer)
}

/// 把选区上画的标注层叠到裁剪图上。标注层是前端 canvas 导出的透明 PNG，
/// 按裁剪图尺寸缩放后 alpha 混合。解码失败一律当无标注，不让一次标注问题毁掉整张图。
pub fn apply_markup(image: DynamicImage, markup: Option<&str>) -> DynamicImage {
    let Some(data) = markup else { return image };
    let Some(encoded) = data.strip_prefix("data:image/png;base64,") else {
        eprintln!("[local] 标注层格式不认识，按无标注处理");
        return image;
    };
    let bytes = match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("[local] 标注层解码失败：{err}");
            return image;
        }
    };
    let layer = match image::load_from_memory(&bytes) {
        Ok(layer) => layer.to_rgba8(),
        Err(err) => {
            eprintln!("[local] 标注层不是有效图片：{err}");
            return image;
        }
    };
    let (width, height) = (image.width(), image.height());
    let mut base = image.to_rgba8();
    let scaled = image::imageops::resize(
        &layer,
        width.max(1),
        height.max(1),
        image::imageops::FilterType::Triangle,
    );
    image::imageops::overlay(&mut base, &scaled, 0, 0);
    image::DynamicImage::ImageRgba8(base)
}

fn stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- 进程内抓帧

/// macOS 的显示器帧缓冲直取。
///
/// 快路径直接读 `dataProvider` 里的像素。显示器帧几乎总是 32 位小端 BGRA，
/// 认准这一种就地换通道，省掉一次「整张图重绘到另一个缓冲区」的拷贝——
/// 全屏 20MB，那一次拷贝就是几十毫秒，而这几十毫秒正好压在「按下热键」上。
/// 认不准的位深/字节序才退回 `CGBitmapContext` 重绘，那条路格式永远正确，只是慢一点。
#[cfg(target_os = "macos")]
mod display {
    use std::ffi::c_void;
    use std::time::Instant;

    use image::RgbaImage;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayCreateImage(display: u32) -> *mut c_void;
        fn CGImageGetWidth(image: *mut c_void) -> usize;
        fn CGImageGetHeight(image: *mut c_void) -> usize;
        fn CGImageGetBytesPerRow(image: *mut c_void) -> usize;
        fn CGImageGetBitsPerPixel(image: *mut c_void) -> usize;
        fn CGImageGetBitmapInfo(image: *mut c_void) -> u32;
        fn CGImageGetDataProvider(image: *mut c_void) -> *mut c_void;
        fn CGDataProviderCopyData(provider: *mut c_void) -> *const c_void;
        fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
        fn CGBitmapContextCreate(
            data: *mut u8,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            space: *mut c_void,
            bitmap_info: u32,
        ) -> *mut c_void;
        fn CGContextDrawImage(ctx: *mut c_void, rect: CGRect, image: *mut c_void);
        fn CGContextRelease(ctx: *mut c_void);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
        fn CFDataGetLength(data: *const c_void) -> isize;
        fn CFRelease(cf: *const c_void);
    }

    /// kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big：
    /// 内存里就是 R,G,B,A，和 `RgbaImage` 的布局一致。
    const RGBA_DEVICE: u32 = 0x4001;

    /// 抓主屏。拿不到（没权限、锁屏、接口不可用）返回 None，交回上层走兜底。
    pub fn grab() -> Option<RgbaImage> {
        unsafe { grab_unsafe() }
    }

    unsafe fn grab_unsafe() -> Option<RgbaImage> {
        let started = Instant::now();
        let image = CGDisplayCreateImage(CGMainDisplayID());
        if image.is_null() {
            return None;
        }
        let width = CGImageGetWidth(image);
        let height = CGImageGetHeight(image);
        if width == 0 || height == 0 {
            CFRelease(image as *const c_void);
            return None;
        }
        let created = started.elapsed();

        let direct = read_direct(image, width, height);
        if direct.is_some() {
            eprintln!(
                "[shell] 帧缓冲 {}ms · 直读 {}ms",
                created.as_millis(),
                started.elapsed().as_millis()
            );
        }
        if let Some(raw) = direct {
            CFRelease(image as *const c_void);
            return Some(raw);
        }

        let redrawn = redraw(image, width, height);
        eprintln!(
            "[shell] 帧缓冲 {}ms · 重绘 {}ms",
            created.as_millis(),
            started.elapsed().as_millis()
        );
        CFRelease(image as *const c_void);
        redrawn
    }

    /// 直读快路径。只认 32 位 + 小端（显示器帧的老实样子）。
    unsafe fn read_direct(image: *mut c_void, width: usize, height: usize) -> Option<RgbaImage> {
        if CGImageGetBitsPerPixel(image) != 32 {
            return None;
        }
        let stride = CGImageGetBytesPerRow(image);
        if stride < width * 4 {
            return None;
        }
        let info = CGImageGetBitmapInfo(image);
        let alpha = info & 0x1f;
        // alpha 为 0/2/6 时它是「第一个分量」，小端存储下内存是 B,G,R,A；
        // 为 1/5 时它是「最后一个分量」，内存是 A,B,G,R。
        let alpha_first = matches!(alpha, 0 | 2 | 6);
        let alpha_last = matches!(alpha, 1 | 5);
        if !alpha_first && !alpha_last {
            return None;
        }
        let order = info & 0x7000;
        if order != 0 && order != 0x2000 {
            // 32 位大端不常见，交给重绘那条路，别在这里赌字节序。
            return None;
        }

        let provider = CGImageGetDataProvider(image);
        if provider.is_null() {
            return None;
        }
        let data = CGDataProviderCopyData(provider);
        if data.is_null() {
            return None;
        }
        if CFDataGetBytePtr(data).is_null() || (CFDataGetLength(data) as usize) < stride * height {
            CFRelease(data as *const c_void);
            return None;
        }

        let source = std::slice::from_raw_parts(CFDataGetBytePtr(data), stride * height);
        let mut buffer = vec![0u8; width * height * 4];
        for (row, target) in buffer.chunks_exact_mut(width * 4).enumerate() {
            let line = &source[row * stride..row * stride + width * 4];
            for (pixel, out) in line.chunks_exact(4).zip(target.chunks_exact_mut(4)) {
                if alpha_first {
                    out[0] = pixel[2];
                    out[1] = pixel[1];
                    out[2] = pixel[0];
                } else {
                    out[0] = pixel[3];
                    out[1] = pixel[2];
                    out[2] = pixel[1];
                }
                // 显示器画面没有透明度，预乘这一步是恒等变换；把 alpha 摆正，
                // 后面不论怎么缩放、编码都按不透明确实处理。
                out[3] = 255;
            }
        }
        CFRelease(data as *const c_void);
        RgbaImage::from_raw(width as u32, height as u32, buffer)
    }

    /// 慢路径但格式永远对：画进一张自己指定布局的位图里。
    unsafe fn redraw(image: *mut c_void, width: usize, height: usize) -> Option<RgbaImage> {
        let space = CGColorSpaceCreateDeviceRGB();
        if space.is_null() {
            return None;
        }
        let mut buffer = vec![0u8; width * height * 4];
        let ctx = CGBitmapContextCreate(
            buffer.as_mut_ptr(),
            width,
            height,
            8,
            width * 4,
            space,
            RGBA_DEVICE,
        );
        CFRelease(space as *const c_void);
        if ctx.is_null() {
            return None;
        }
        let rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: width as f64,
                height: height as f64,
            },
        };
        CGContextDrawImage(ctx, rect, image);
        CGContextRelease(ctx);
        for pixel in buffer.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        RgbaImage::from_raw(width as u32, height as u32, buffer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn shot_of(width: u32, height: u32) -> Shot {
        let buffer = RgbaImage::from_pixel(width, height, Rgba([12, 18, 26, 255]));
        Shot {
            path: None,
            image: DynamicImage::ImageRgba8(buffer),
        }
    }

    #[test]
    fn crop_maps_fractions_to_pixels() {
        let shot = shot_of(2000, 1000);
        let cropped = shot.crop(0.25, 0.5, 0.5, 0.25);
        assert_eq!(cropped.dimensions(), (1000, 250));
    }

    #[test]
    fn crop_clamps_to_image_bounds() {
        let shot = shot_of(1000, 800);
        let cropped = shot.crop(0.9, 0.9, 0.5, 0.5);
        assert_eq!(cropped.dimensions(), (100, 80));
    }

    #[test]
    fn crop_never_returns_empty_image() {
        let shot = shot_of(400, 300);
        let cropped = shot.crop(0.0, 0.0, 0.0, 0.0);
        assert_eq!(cropped.dimensions(), (1, 1));
    }

    #[test]
    fn small_selection_is_upscaled_but_capped_at_two_times() {
        let small = shot_of(200, 120).image;
        assert_eq!(normalize(&small).dimensions(), (400, 240));
    }

    #[test]
    fn medium_selection_is_left_alone() {
        let medium = shot_of(900, 600).image;
        assert_eq!(normalize(&medium).dimensions(), (900, 600));
    }

    #[test]
    fn huge_selection_is_downscaled_to_limit() {
        let huge = shot_of(3200, 1600).image;
        assert_eq!(normalize(&huge).dimensions(), (2000, 1000));
    }

    #[test]
    fn preview_is_downscaled_to_readable_size() {
        let shot = shot_of(3200, 1600);
        let scaled = downscale_fast(&shot.image, PREVIEW_EDGE);
        // 长边压到 1800，比例保持：1600/3200 * 1800 = 900
        assert_eq!(scaled.dimensions(), (1800, 900));
    }

    #[test]
    fn preview_file_stays_small() {
        let shot = shot_of(3200, 1600);
        let path = preview_file(&shot.image).expect("preview");
        let size = std::fs::metadata(&path).expect("meta").len();
        // 1800 长边的 JPEG 落盘后几百 KB，不该把整张原图写出去
        assert!(size > 0 && size < 2_000_000, "预览太大：{size}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cleanup_without_temp_file_does_not_panic() {
        // 走进程内抓帧时 Shot 根本没有临时文件，cleanup 必须是空操作。
        shot_of(40, 30).cleanup();
    }
}
