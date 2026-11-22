//! 智能裁剪：对整卷图做灰度阈值 + 行列投影，找出一块块「文字块（题）」的候选框。
//! 纯本地像素运算，不调模型、不联网，白底黑字的试卷效果最好。
//!
//! 思路：题与题之间必有横向空白带，行与行之间空白更窄。先用行投影把整卷切成若干
//! 横带（每带≈一道题或一段），再在每带内做列投影收紧左右边界。返回原图坐标，
//! 前端把这些框盖在图上，用户点哪块就裁哪块去识别。
//!
//! 阈值不能写死像素：同一份卷子 24pt 标题和 10.5pt 选项的行高差三倍，
//! 换个分辨率又差一倍。这里全部按**本页自己的行高与行距**推：
//!
//! - 先统计每一行文字的高度，取中位数当 `line_h`；
//! - 再统计相邻两行之间的空白，取 40 分位当「题内正常行距」；
//! - 只有空白达到正常行距的两倍以上，才算「另起一题」；
//! - 选项常常另起一段但缩进一点，光看空白会被切成两块——所以再看左边缘：
//!   明显缩进（相对本页主边距）并且空白不大的横带，并回上一题。
//!
//! 最后一条是这次修的关键：以前「题干一块、A/B/C/D 一块」，裁出来只有半道题。

use image::{DynamicImage, GenericImageView};

/// 候选区域（原图像素坐标）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

const MAX_EDGE: u32 = 1400; // 投影用的降采样长边，够看清题间空白又足够快
const MAX_REGIONS: usize = 40; // 一页最多给这么多块，再多点起来也累
const MIN_EDGE: u32 = 40; // 小于这个尺寸的图不值得找块

/// 一条横带：行投影切出来的一段，`left` 是这段最左的有墨列（降采样坐标）。
struct Band {
    top: u32,
    bottom: u32,
    left: u32,
}

/// 在图上找候选文字块。按阅读顺序（先上后下、再左后右）返回，最多 40 块。
pub fn detect_regions(image: &DynamicImage) -> Vec<Region> {
    let (ow, oh) = image.dimensions();
    if ow < MIN_EDGE || oh < MIN_EDGE {
        return Vec::new();
    }
    let scale = (MAX_EDGE as f64 / ow.max(oh) as f64).min(1.0);
    let sw = (ow as f64 * scale).round().max(1.0) as u32;
    let sh = (oh as f64 * scale).round().max(1.0) as u32;
    let work = if scale < 1.0 {
        image.resize_exact(sw, sh, image::imageops::FilterType::Triangle)
    } else {
        image.clone()
    };
    let gray = work.to_luma8();
    let threshold = otsu(&gray);

    // 行墨量：每行有多少个像素比阈值暗。
    let row_ink: Vec<u32> = (0..sh)
        .map(|y| {
            (0..sw)
                .filter(|x| gray.get_pixel(*x, y).0[0] <= threshold)
                .count() as u32
        })
        .collect();
    let blank_limit = (sw as f64 * 0.004).max(1.0) as u32;
    let runs = text_runs(&row_ink, blank_limit);
    if runs.is_empty() {
        return Vec::new();
    }

    // 本页的行高与行距：所有阈值都由它们推出来，不写死像素。
    let line_h = {
        let mut heights: Vec<u32> = runs
            .iter()
            .map(|(top, bottom)| bottom - top)
            .filter(|height| *height >= 3)
            .collect();
        percentile(&mut heights, 50).unwrap_or(16).max(6)
    };
    // 分界按**行高**推，不按「空白的统计」推。
    //
    // 空白统计看着更聪明，实际会被「题间的大空白」污染：一页只有两道题时，
    // 唯一的那个空白就是题间距，取什么分位数都是它，于是永远切不开。
    // 行高就稳得多——题内换行约 0.3~0.6 倍行高，空一行约 1.0~1.3 倍，
    // 另起一题约 1.5~3 倍。取 1.8 倍当分界：
    //   · 空一行接选项（≈1.2 倍）留在题内；
    //   · 真正的题间距（≥1.5 倍）切开。
    // 切过头了还有下面的「缩进并回上一题」兜底，所以宁可少切一点。
    let split_gap = ((line_h as f64 * 1.8) as u32).max(8);

    // 先按横向空白粗分带。
    let mut bands: Vec<Band> = Vec::new();
    let mut current: Option<(u32, u32)> = None;
    for (top, bottom) in runs {
        match current {
            Some((start, end)) if top.saturating_sub(end) < split_gap => {
                current = Some((start, bottom));
            }
            Some((start, end)) => {
                bands.push(Band {
                    top: start,
                    bottom: end,
                    left: band_left(&gray, threshold, start, end, sw),
                });
                current = Some((top, bottom));
            }
            None => current = Some((top, bottom)),
        }
    }
    if let Some((start, end)) = current {
        bands.push(Band {
            top: start,
            bottom: end,
            left: band_left(&gray, threshold, start, end, sw),
        });
    }

    // 分数线、页边横线这类只有几像素高的碎片，单独成块没有意义。
    let min_band = (line_h as f64 * 0.5) as u32;
    bands.retain(|band| band.bottom - band.top >= min_band.max(4));
    if bands.is_empty() {
        return Vec::new();
    }

    // 本页主边距取各带左边缘的众数：缩进判断要跟「这一页的版心」比，不是跟 0 比。
    let margin = mode_of(bands.iter().map(|band| band.left).collect());
    let indent = (sw as f64 * 0.012).max(8.0) as u32;

    // 缩进的横带并回上一块：选项另起一段、解答题的分问缩进，都属于「上一题的继续」。
    let mut merged: Vec<Band> = Vec::new();
    for band in bands {
        if let Some(prev) = merged.last_mut() {
            let gap = band.top.saturating_sub(prev.bottom);
            if band.left >= margin.saturating_add(indent) && gap <= (split_gap as f64 * 2.2) as u32
            {
                prev.bottom = band.bottom;
                prev.left = prev.left.min(band.left);
                continue;
            }
        }
        merged.push(band);
    }

    let inv = 1.0 / scale;
    let pad_x = (6.0 * scale).max(1.0) as u32;
    let pad_y = (5.0 * scale).max(1.0) as u32;
    let mut regions: Vec<Region> = Vec::new();
    for band in merged {
        // 带内列投影：找出这一带里最左 / 最右有墨的列。
        let mut first = None;
        let mut last = None;
        for x in 0..sw {
            let mut ink = 0u32;
            for y in band.top..band.bottom {
                if gray.get_pixel(x, y).0[0] <= threshold {
                    ink += 1;
                }
            }
            if ink > 0 {
                if first.is_none() {
                    first = Some(x);
                }
                last = Some(x);
            }
        }
        let (Some(l), Some(r)) = (first, last) else {
            continue;
        };
        if r <= l {
            continue;
        }
        let x0 = l.saturating_sub(pad_x);
        let y0 = band.top.saturating_sub(pad_y);
        let x1 = (r + pad_x + 1).min(sw);
        let y1 = (band.bottom + pad_y).min(sh);
        regions.push(Region {
            x: (x0 as f64 * inv).round() as u32,
            y: (y0 as f64 * inv).round() as u32,
            w: ((x1 - x0) as f64 * inv).round() as u32,
            h: ((y1 - y0) as f64 * inv).round() as u32,
        });
    }

    // 按阅读顺序排：同一行（y 接近）从左到右，否则从上到下。
    regions.sort_by(|a, b| {
        let ay = a.y as i64 / 20;
        let by = b.y as i64 / 20;
        ay.cmp(&by).then(a.x.cmp(&b.x))
    });
    regions.truncate(MAX_REGIONS);
    regions
}

/// 逐行扫出「有墨的连续行」，也就是一行行文字。
fn text_runs(row_ink: &[u32], blank_limit: u32) -> Vec<(u32, u32)> {
    let mut runs = Vec::new();
    let mut y = 0u32;
    let height = row_ink.len() as u32;
    while y < height {
        if row_ink[y as usize] > blank_limit {
            let start = y;
            while y < height && row_ink[y as usize] > blank_limit {
                y += 1;
            }
            runs.push((start, y));
        } else {
            y += 1;
        }
    }
    runs
}

/// 这一带最左的有墨列；整带全白就退回 0（后面会被丢掉）。
fn band_left(gray: &image::GrayImage, threshold: u8, top: u32, bottom: u32, width: u32) -> u32 {
    for x in 0..width {
        for y in top..bottom {
            if gray.get_pixel(x, y).0[0] <= threshold {
                return x;
            }
        }
    }
    0
}

/// 取分位数（会就地排序）。传 50 就是中位数。
fn percentile(values: &mut [u32], percent: usize) -> Option<u32> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let index = (values.len() - 1) * percent / 100;
    Some(values[index])
}

/// 出现次数最多的值；并列时取小的（版心一般靠左）。
fn mode_of(values: Vec<u32>) -> u32 {
    let mut counts: Vec<(u32, usize)> = Vec::new();
    for value in values {
        match counts.iter_mut().find(|(seen, _)| *seen == value) {
            Some((_, count)) => *count += 1,
            None => counts.push((value, 1)),
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(value, _)| value)
        .unwrap_or(0)
}

/// Otsu 自适应阈值：白底黑字这种双峰直方图，取类间方差最大的灰度作分界。
fn otsu(gray: &image::GrayImage) -> u8 {
    let mut hist = [0u32; 256];
    for pixel in gray.pixels() {
        hist[pixel.0[0] as usize] += 1;
    }
    let total = gray.width() as u64 * gray.height() as u64;
    if total == 0 {
        return 170;
    }
    let mut sum = 0u64;
    for (i, count) in hist.iter().enumerate() {
        sum += i as u64 * *count as u64;
    }
    let mut sum_b = 0u64;
    let mut w_b = 0u64;
    let mut best = 0f64;
    let mut threshold = 170u8;
    for (i, count) in hist.iter().enumerate() {
        w_b += *count as u64;
        if w_b == 0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0 {
            break;
        }
        sum_b += i as u64 * *count as u64;
        let m_b = sum_b as f64 / w_b as f64;
        let m_f = (sum - sum_b) as f64 / w_f as f64;
        let between = (w_b as f64) * (w_f as f64) * (m_b - m_f) * (m_b - m_f);
        if between > best {
            best = between;
            threshold = i as u8;
        }
    }
    threshold
}

/// 按候选框裁出一块区域，交给前端当新图去识别。
pub fn crop_region(image: &DynamicImage, region: &Region) -> DynamicImage {
    let (w, h) = image.dimensions();
    let x = region.x.min(w.saturating_sub(1));
    let y = region.y.min(h.saturating_sub(1));
    let cw = region.w.max(1).min(w - x);
    let ch = region.h.max(1).min(h - y);
    image.crop_imm(x, y, cw, ch)
}

/// 跨页题拼接：把相邻几页竖着叠成一张图。
/// 一道题被翻页截断时，分开识别会得到「半道题 + 半个选项表」，
/// 拼起来再识别才是一道完整的题；页间留一条白缝便于人眼分辨从哪断的。
pub fn stitch(
    paths: &[String],
    data_dir: &std::path::Path,
) -> Result<crate::models::CaptureResult, String> {
    use image::Rgba;
    if paths.len() < 2 {
        return Err("至少要有两页才能拼接".into());
    }
    let mut pages = Vec::new();
    for path in paths {
        let loaded = image::ImageReader::open(std::path::Path::new(path))
            .map_err(|e| format!("读不了这一页：{e}"))?
            .with_guessed_format()
            .map_err(|e| format!("认不出这一页的格式：{e}"))?
            .decode()
            .map_err(|e| format!("解析这一页失败：{e}"))?;
        pages.push(loaded.to_rgba8());
    }
    let width = pages.iter().map(|page| page.width()).max().unwrap_or(0);
    let gap = 20u32;
    let height: u32 =
        pages.iter().map(|page| page.height()).sum::<u32>() + gap * (pages.len() as u32 - 1);
    if width == 0 || height == 0 {
        return Err("这两页渲出来是空的".into());
    }
    let mut canvas = image::RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));
    let mut y = 0i64;
    for page in &pages {
        let x = ((width - page.width()) / 2) as i64;
        image::imageops::overlay(&mut canvas, page, x, y);
        y += page.height() as i64 + gap as i64;
    }
    let dir = data_dir.join("tmp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;
    let out = dir.join(format!("stitch-{}.png", uuid::Uuid::new_v4()));
    canvas
        .save(&out)
        .map_err(|e| format!("保存拼接图失败：{e}"))?;
    crate::capture::from_path(&out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};

    /// 造一张白卷：两道「题」是两条黑带，中间隔着一大段空白。
    fn paper() -> DynamicImage {
        let mut img = RgbaImage::from_pixel(600, 400, Rgba([255, 255, 255, 255]));
        for y in 40..80 {
            for x in 30..560 {
                img.put_pixel(x, y, Rgba([20, 20, 20, 255]));
            }
        }
        for y in 220..260 {
            for x in 30..560 {
                img.put_pixel(x, y, Rgba([20, 20, 20, 255]));
            }
        }
        DynamicImage::ImageRgba8(img)
    }

    /// 造一页「题干 + 选项」：题干从左边距起，选项缩进两格，
    /// 题干与选项之间隔一行空白（比题内的行距大），题与题之间隔三行。
    fn paper_with_options() -> DynamicImage {
        let mut img = RgbaImage::from_pixel(800, 520, Rgba([255, 255, 255, 255]));
        let mut line = |top: u32, left: u32| {
            for y in top..top + 16 {
                for x in left..760 {
                    img.put_pixel(x, y, Rgba([20, 20, 20, 255]));
                }
            }
        };
        // 第 1 题：题干（行 1、行 2）+ 空一行 + 选项两行（缩进 40）
        line(40, 40);
        line(64, 40);
        line(110, 80);
        line(134, 80);
        // 第 2 题：隔三行空白后另起
        line(200, 40);
        line(224, 40);
        line(270, 80);
        line(294, 80);
        DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn detects_two_question_bands() {
        let regions = detect_regions(&paper());
        assert_eq!(regions.len(), 2, "应检出两块题，实得 {:?}", regions);
        assert!(regions[0].y < 100);
        assert!(regions[1].y > 180);
    }

    /// 这是用户报的那个毛病：题干和 A/B/C/D 被切成两块。
    /// 选项明明缩进、又紧跟题干，就该并成一道题。
    #[test]
    fn options_stay_with_their_stem() {
        let regions = detect_regions(&paper_with_options());
        assert_eq!(
            regions.len(),
            2,
            "题干 + 选项应各成一块，实得 {:?}",
            regions
                .iter()
                .map(|r| (r.x, r.y, r.w, r.h))
                .collect::<Vec<_>>()
        );
        // 第一块要一直盖到第二行选项（y≈294 之前只有一处题）
        assert!(
            regions[0].h > 110,
            "第一块应包含选项，实得高 {}",
            regions[0].h
        );
        assert!(
            regions[1].y > 180,
            "第二块应从 200 附近开始，实得 {}",
            regions[1].y
        );
    }

    #[test]
    fn single_line_question_is_not_dropped() {
        // 一整页都是单行题：以前这些会被 MIN_BAND 直接丢掉，一块都找不到。
        let mut img = RgbaImage::from_pixel(600, 300, Rgba([255, 255, 255, 255]));
        for (index, top) in [30u32, 90, 150, 210].iter().enumerate() {
            let _ = index;
            for y in *top..*top + 14 {
                for x in 40..560 {
                    img.put_pixel(x, y, Rgba([30, 30, 30, 255]));
                }
            }
        }
        let regions = detect_regions(&DynamicImage::ImageRgba8(img));
        assert_eq!(regions.len(), 4, "四道单行题应各成一块，实得 {regions:?}");
    }

    #[test]
    fn crop_region_clamps_to_bounds() {
        let img = paper();
        let cropped = crop_region(
            &img,
            &Region {
                x: 20,
                y: 30,
                w: 800,
                h: 800,
            },
        );
        let (w, h) = cropped.dimensions();
        assert!(w <= 600 && h <= 400);
    }

    /// 手画框选走的是同一个 crop_region：不落在检测块上也要裁对位置和尺寸。
    #[test]
    fn manual_rect_crops_exact_area() {
        let img = paper();
        let cocked = crop_region(
            &img,
            &Region {
                x: 40,
                y: 50,
                w: 200,
                h: 60,
            },
        );
        let (w, h) = cocked.dimensions();
        assert_eq!((w, h), (200, 60), "手画选区应按原尺寸裁出");
        // 选区内含那道黑带（y 40..80），裁出来不能是一片白。
        let dark = cocked
            .to_luma8()
            .pixels()
            .filter(|pixel| pixel.0[0] < 60)
            .count();
        assert!(dark > 500, "选区里应保留题目墨迹，实得 {dark} 个暗像素");
    }

    #[test]
    fn manual_rect_beyond_edges_is_trimmed() {
        let img = paper();
        let cocked = crop_region(
            &img,
            &Region {
                x: 560,
                y: 380,
                w: 400,
                h: 400,
            },
        );
        let (w, h) = cocked.dimensions();
        assert_eq!((w, h), (40, 20), "越界部分应被夹到图内");
    }

    #[test]
    fn blank_paper_yields_no_regions() {
        let blank =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(300, 300, Rgba([255, 255, 255, 255])));
        assert!(detect_regions(&blank).is_empty());
    }

    #[test]
    fn modes_and_percentiles_are_sane() {
        assert_eq!(percentile(&mut [5, 1, 3, 9], 50), Some(3));
        assert_eq!(percentile(&mut [], 50), None);
        assert_eq!(mode_of(vec![95, 95, 128, 95, 459]), 95);
        assert_eq!(mode_of(vec![7, 7, 3, 3]), 3, "并列时取小的");
    }

    /// 跨页题：两页竖着叠起来，宽度取较宽的那页，中间留白缝。
    #[test]
    fn stitch_stacks_pages_vertically() {
        let dir = std::env::temp_dir().join(format!("nb-stitch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let first = dir.join("p1.png");
        let second = dir.join("p2.png");
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(200, 100, Rgba([10, 10, 10, 255])))
            .save(&first)
            .expect("写第一页");
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(120, 80, Rgba([20, 20, 20, 255])))
            .save(&second)
            .expect("写第二页");
        let paths = vec![
            first.to_string_lossy().to_string(),
            second.to_string_lossy().to_string(),
        ];
        let out = stitch(&paths, &dir).expect("拼接两页");
        assert_eq!((out.width, out.height), (200, 200), "100 + 20 缝 + 80");
        assert!(
            std::path::Path::new(&out.path).exists(),
            "拼接图要落盘给识别用"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
