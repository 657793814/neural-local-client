//! 本地 OCR：图片 → 文字行 → 结构化题目。
//!
//! 走 macOS 自带的 Vision 框架（`VNRecognizeTextRequest`），不配置模型也能用，
//! 断网可用；识别不出来时由上层决定是否回退到视觉模型。
//!
//! 这一层的职责与 `local-client-my` 的 `ocr_engine.py` 对齐：
//! 1. 统一读图（EXIF 转正、webp/HEIC 兜底），不依赖调用方先转格式；
//! 2. 把引擎结果归一成 `{text, bbox, confidence}`，屏蔽不同版本的差异；
//! 3. 在本地按题号切题、拆选项、认「答案/解析/知识点/难度」标记；
//! 4. 解析不了时至少返回一整段题干，不把整张图丢掉。
//!
//! 公式图仍建议走「AI 识图」或 Pix2Text 一类专用能力，本地 OCR 只负责文字。

use serde_json::{json, Value};

#[derive(Clone, Debug)]
pub struct Line {
    pub text: String,
    /// 像素坐标：[x, y, 宽, 高]，左上角为原点。
    pub bbox: [f64; 4],
    pub confidence: f64,
}

/// 本机 OCR 引擎是否可用。目前只有 macOS 内置 Vision。
pub fn available() -> bool {
    cfg!(target_os = "macos")
}

pub fn status() -> Value {
    json!({
        "engine": if available() { "Apple Vision（本机）" } else { "不可用" },
        "available": available(),
        "offline": true,
        "languages": ["zh-Hans", "zh-Hant", "en-US"],
        "note": if available() {
            "本机离线识别，不需要模型 Key；公式图或版式复杂时切「AI 识图」"
        } else {
            "当前系统没有可用的本机 OCR，请切「AI 识图」"
        },
    })
}

/// 识别一张图，返回按阅读顺序排好的文字行。
pub fn recognize(path: &str) -> Result<Vec<Line>, String> {
    let image = read_image(path)?;
    recognize_image(&image)
}

/// 识别一张图并直接在本地切成题目。找不到题号时至少给出一道题干。
pub fn recognize_questions(path: &str, source: &str) -> Result<Vec<Value>, String> {
    let lines = recognize(path)?;
    if lines.is_empty() {
        return Err("本地 OCR 没读到文字：图可能太糊、是整页公式，或图片方向不对".into());
    }
    let items = questions_from_lines(&lines, source);
    if items.is_empty() {
        return Err("本地 OCR 读到了文字，但没有整理出题干".into());
    }
    Ok(items)
}

/// 把 OCR 行按题号切成题目。公开给单测和「粘贴文字」的本地整理复用。
pub fn questions_from_lines(lines: &[Line], source: &str) -> Vec<Value> {
    let mut texts: Vec<String> = Vec::new();
    for line in lines {
        let text = clean_line(&line.text);
        if text.is_empty() || is_page_noise(&text) {
            continue;
        }
        texts.extend(expand_markers(&text));
    }
    if texts.is_empty() {
        return Vec::new();
    }
    let blocks = split_blocks(&texts);
    let total_conf = if lines.is_empty() {
        0.0
    } else {
        lines.iter().map(|line| line.confidence).sum::<f64>() / lines.len() as f64
    };
    blocks
        .into_iter()
        .filter_map(|block| build_item(&block, source, total_conf))
        .collect()
}

// ---------------------------------------------------------------------------
// 读图：统一转成 RGBA，EXIF 转正；常见格式都走 image crate
// ---------------------------------------------------------------------------

fn read_image(path: &str) -> Result<image::RgbaImage, String> {
    let decoded = decode_image(path);
    match decoded {
        Ok(image) => Ok(image),
        Err(primary) => {
            // HEIC / 老 TIFF 等 image crate 不认的格式，交给系统 sips 转一次 PNG。
            #[cfg(target_os = "macos")]
            {
                if let Some(image) = sips_to_png(path)? {
                    return Ok(image);
                }
            }
            Err(primary)
        }
    }
}

fn decode_image(path: &str) -> Result<image::RgbaImage, String> {
    use image::{DynamicImage, ImageDecoder, ImageReader};

    let reader = ImageReader::open(path)
        .map_err(|e| format!("读不了图片：{e}"))?
        .with_guessed_format()
        .map_err(|e| format!("认不出图片格式：{e}"))?;
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("解析图片失败：{e}"))?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image =
        DynamicImage::from_decoder(decoder).map_err(|e| format!("解析图片失败：{e}"))?;
    image.apply_orientation(orientation);
    Ok(image.to_rgba8())
}

#[cfg(target_os = "macos")]
fn sips_to_png(path: &str) -> Result<Option<image::RgbaImage>, String> {
    let out = std::env::temp_dir().join(format!("nb-ocr-{}.png", uuid::Uuid::new_v4()));
    let status = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "png"])
        .arg(path)
        .arg("--out")
        .arg(&out)
        .output()
        .map_err(|e| format!("调用 sips 转图失败：{e}"))?;
    if !status.status.success() {
        let _ = std::fs::remove_file(&out);
        return Ok(None);
    }
    let decoded = decode_image(out.to_string_lossy().as_ref());
    let _ = std::fs::remove_file(&out);
    Ok(decoded.ok())
}

// ---------------------------------------------------------------------------
// 平台实现
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn recognize_image(image: &image::RgbaImage) -> Result<Vec<Line>, String> {
    mac::recognize(image)
}

#[cfg(not(target_os = "macos"))]
fn recognize_image(_image: &image::RgbaImage) -> Result<Vec<Line>, String> {
    Err("本地 OCR 目前只在 macOS 上可用，请改用「AI 识图」".into())
}

#[cfg(target_os = "macos")]
mod mac {
    use super::Line;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_core_foundation::CGFloat;
    use objc2_core_graphics::{
        CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage,
        CGImageAlphaInfo, CGImageByteOrderInfo,
    };
    use objc2_foundation::{NSArray, NSDictionary, NSString};
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequestTextRecognitionLevel,
    };
    use std::ffi::c_void;
    use std::ptr::NonNull;

    /// CGDataProvider 释放时把传给它的 Vec 一起回收，避免每张图漏一份像素。
    unsafe extern "C-unwind" fn release_rgba(
        info: *mut c_void,
        _data: NonNull<c_void>,
        _size: usize,
    ) {
        if !info.is_null() {
            drop(unsafe { Box::from_raw(info as *mut Vec<u8>) });
        }
    }

    fn cg_image(
        image: &image::RgbaImage,
    ) -> Result<objc2_core_foundation::CFRetained<CGImage>, String> {
        let (width, height) = image.dimensions();
        let bytes = image.as_raw().clone();
        let boxed = Box::new(bytes);
        let len = boxed.len();
        let ptr = Box::into_raw(boxed);
        let provider = unsafe {
            CGDataProvider::with_data(
                ptr as *mut c_void,
                (*ptr).as_ptr() as *const c_void,
                len,
                Some(release_rgba),
            )
        };
        let Some(provider) = provider else {
            // provider 没建起来时回调不会触发，这里自己收回。
            drop(unsafe { Box::from_raw(ptr) });
            return Err("创建图片数据失败".into());
        };
        let Some(space) = CGColorSpace::new_device_rgb() else {
            return Err("创建颜色空间失败".into());
        };
        let bitmap = CGBitmapInfo(
            CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0,
        );
        let image = unsafe {
            CGImage::new(
                width as usize,
                height as usize,
                8,
                32,
                width as usize * 4,
                Some(&space),
                bitmap,
                Some(&provider),
                std::ptr::null::<CGFloat>(),
                false,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        };
        image.ok_or_else(|| "创建 CGImage 失败".into())
    }

    pub fn recognize(image: &image::RgbaImage) -> Result<Vec<Line>, String> {
        let (width, height) = image.dimensions();
        if width == 0 || height == 0 {
            return Err("图片是空的".into());
        }
        let cg = cg_image(image)?;

        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(true);
        let languages = NSArray::from_retained_slice(&[
            NSString::from_str("zh-Hans"),
            NSString::from_str("zh-Hant"),
            NSString::from_str("en-US"),
        ]);
        request.setRecognitionLanguages(&languages);
        request.setAutomaticallyDetectsLanguage(true);
        request.setMinimumTextHeight(0.0);

        let options: Retained<NSDictionary<objc2_vision::VNImageOption, AnyObject>> =
            NSDictionary::new();
        let handler = unsafe {
            VNImageRequestHandler::initWithCGImage_options(
                VNImageRequestHandler::alloc(),
                &cg,
                &options,
            )
        };

        // VNRecognizeTextRequest → VNImageBasedRequest → VNRequest
        let as_request = request.clone().into_super().into_super();
        let requests = NSArray::from_retained_slice(&[as_request]);
        handler
            .performRequests_error(&requests)
            .map_err(|error| format!("本机 OCR 失败：{error}"))?;

        let Some(results) = request.results() else {
            return Ok(Vec::new());
        };

        let mut lines = Vec::new();
        for observation in results.iter() {
            let Some(candidate) = observation.topCandidates(1).iter().next() else {
                continue;
            };
            let text = candidate.string().to_string();
            if text.trim().is_empty() {
                continue;
            }
            let rect = unsafe { observation.boundingBox() };
            let x = rect.origin.x * width as f64;
            // Vision 的坐标原点在左下角，换成我们习惯的左上角。
            let y = (1.0 - rect.origin.y - rect.size.height) * height as f64;
            lines.push(Line {
                text,
                bbox: [
                    x,
                    y,
                    rect.size.width * width as f64,
                    rect.size.height * height as f64,
                ],
                confidence: candidate.confidence() as f64,
            });
        }

        // 阅读顺序：先上后下，再左后右。同一行的容差交给 y 排序即可。
        lines.sort_by(|a, b| {
            let row = (a.bbox[1] - b.bbox[1]).abs();
            if row < 8.0 {
                a.bbox[0]
                    .partial_cmp(&b.bbox[0])
                    .unwrap_or(std::cmp::Ordering::Equal)
            } else {
                a.bbox[1]
                    .partial_cmp(&b.bbox[1])
                    .unwrap_or(std::cmp::Ordering::Equal)
            }
        });
        Ok(lines)
    }
}

// ---------------------------------------------------------------------------
// 本地切题
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Stem,
    Options,
    Answer,
    Analysis,
    Knowledge,
    Difficulty,
}

fn clean_line(raw: &str) -> String {
    raw.replace('\u{00a0}', " ")
        .replace('\u{3000}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// 组卷网站（zujuan.xkw.com 等）底部的工具栏按钮文字：今日 / N次组卷 /
/// N卷引用 / 相似题 / 纠错 / 详情 / 收藏 / 加入试题篮。
/// 截图录题时这些会被 OCR 成一行行文本，再被题号规则误拆成独立题目。
/// 抽出常量给「行级过滤」和「整题兜底」共用。
const WEB_UI_TOKENS: &[&str] = &[
    "次组卷",
    "卷引用",
    "相似题",
    "加入试题篮",
    "试题篮",
    "纠错",
];

/// 一整道题的题干是否只是网页按钮文字拼出来的（不是真题目）。
/// 要求同时命中「加入试题篮」和一个组卷特征，避免误伤正文里偶然出现的词。
pub fn is_web_ui_stem(text: &str) -> bool {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !t.contains("加入试题篮") {
        return false;
    }
    WEB_UI_TOKENS
        .iter()
        .any(|token| t.contains(token) && *token != "加入试题篮" && *token != "试题篮")
}

/// 去掉页眉页脚、站点水印、网页工具栏这类与题目无关的行。
fn is_page_noise(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    if t.len() <= 3 && t.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    let lower = t.to_lowercase();
    if lower.starts_with("第") && lower.contains("页") && lower.len() <= 12 {
        return true;
    }
    if lower.contains("版权所有") || lower.contains("学科网") || lower.contains("组卷网")
    {
        return true;
    }
    if lower.starts_with("www.") || lower.starts_with("http://") || lower.starts_with("https://") {
        return true;
    }
    // 整行就是一个网页按钮：直接丢。这些词单独成行不可能是题目内容。
    // 「今日」「详情」「收藏」太短太常见，不单列，靠上面的整题兜底拦。
    let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if WEB_UI_TOKENS
        .iter()
        .any(|token| compact == *token || compact.ends_with("次组卷") || compact.contains("卷引用"))
    {
        return true;
    }
    false
}

/// 把同一行里的「答案：」「【解析】」等标记拆成多行，后面的解析才接得住。
fn expand_markers(line: &str) -> Vec<String> {
    const TOKENS: &[&str] = &[
        "【答案】",
        "【参考答案】",
        "【正确答案】",
        "答案：",
        "答案:",
        "正确答案：",
        "正确答案:",
        "参考答案：",
        "参考答案:",
        "【解析】",
        "【详解】",
        "【分析】",
        "【点睛】",
        "解析：",
        "解析:",
        "详解：",
        "详解:",
        "分析：",
        "分析:",
        "【知识点】",
        "【考点】",
        "知识点：",
        "知识点:",
        "考点：",
        "考点:",
        "【难度】",
        "难度：",
        "难度:",
    ];
    let mut cuts: Vec<usize> = Vec::new();
    for token in TOKENS {
        let mut from = 0;
        while let Some(index) = line[from..].find(token) {
            let absolute = from + index;
            if absolute == 0
                || line[..absolute]
                    .chars()
                    .last()
                    .map(|c| c.is_whitespace() || "。；;，,、".contains(c))
                    .unwrap_or(false)
            {
                cuts.push(absolute);
            }
            from = absolute + token.len();
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    if cuts.is_empty() {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    let mut start = 0;
    for cut in cuts {
        if cut > start {
            let piece = line[start..cut].trim();
            if !piece.is_empty() {
                out.push(piece.to_string());
            }
        }
        start = cut;
    }
    let tail = line[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

fn split_blocks(lines: &[String]) -> Vec<Vec<String>> {
    let mut starts = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if question_start(line).is_some() {
            starts.push(index);
        }
    }
    if starts.is_empty() {
        return vec![lines.to_vec()];
    }
    let mut blocks = Vec::new();
    // 题号前的零散文字通常是页眉；但整页只有一道题、题号又在第二行时，
    // 丢掉第一行会少半道题，所以只在它足够长的时候单独成块。
    if starts[0] > 0 {
        let head: Vec<String> = lines[..starts[0]]
            .iter()
            .filter(|line| line.chars().count() > 12)
            .cloned()
            .collect();
        if head.iter().map(|line| line.chars().count()).sum::<usize>() > 24 {
            blocks.push(head);
        }
    }
    for (position, start) in starts.iter().enumerate() {
        let end = starts.get(position + 1).copied().unwrap_or(lines.len());
        let mut block = lines[*start..end].to_vec();
        if let Some(rest) = question_start(&block[0]) {
            if rest.is_empty() {
                block.remove(0);
            } else {
                block[0] = rest;
            }
        }
        if !block.is_empty() {
            blocks.push(block);
        }
    }
    blocks
}

/// 认题号：`1.` / `1、` / `1)` / `1 `。只认行首，避免把 `x^2 + 1.5` 切坏。
fn question_start(line: &str) -> Option<String> {
    let text = line.trim_start();
    let mut digits = 0;
    let mut number = 0u32;
    for ch in text.chars() {
        if let Some(value) = ch.to_digit(10) {
            if digits >= 3 {
                return None;
            }
            digits += 1;
            number = number * 10 + value;
        } else {
            break;
        }
    }
    if digits == 0 || number == 0 {
        return None;
    }
    let separator = text[digits..].chars().next()?;
    if !matches!(separator, '.' | '．' | '、' | ')' | '）' | ' ') {
        return None;
    }
    let rest = text[digits + separator.len_utf8()..].trim_start();
    if rest.chars().count() < 2 {
        return None;
    }
    // `1.5` 这种小数不是题号；`2024 年` 也不是。
    if matches!(separator, '.' | '．')
        && rest
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
    {
        return None;
    }
    // 选项行 `A.` 不会走到这里（digits == 0），但要防止 `1 A.` 这种合并行。
    if option_marker(rest).is_some() && rest.chars().count() < 8 {
        return None;
    }
    if !rest.chars().any(|c| c.is_alphabetic() || c >= '\u{4e00}') {
        return None;
    }
    Some(rest.to_string())
}

fn option_marker(text: &str) -> Option<(char, &str)> {
    let trimmed = text.trim_start();
    let mut rest = trimmed;
    if let Some(stripped) = rest.strip_prefix('(').or_else(|| rest.strip_prefix('（')) {
        rest = stripped;
    }
    let mut chars = rest.chars();
    let key = chars.next()?;
    if !('A'..='H').contains(&key) && !('a'..='h').contains(&key) {
        return None;
    }
    let after_key = &rest[key.len_utf8()..];
    let separator = after_key.chars().next()?;
    if !matches!(separator, '.' | '．' | '、' | ')' | '）' | ' ') {
        return None;
    }
    let body = after_key[separator.len_utf8()..].trim_start();
    if body.is_empty() {
        return None;
    }
    Some((key.to_ascii_uppercase(), body))
}

/// 选项标记只认这些边界：行首、空白，或题干/上一选项的收尾标点。
/// 否则 `x=1。A.` 之后的 `A.` 会被漏掉，`C:\\Users` 这类路径又不会误判（没有边界）。
fn option_boundary(text: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let prev = match text[..index].chars().last() {
        Some(prev) => prev,
        None => return true,
    };
    prev.is_whitespace() || "。；;，,、）)？?！!".contains(prev)
}

fn split_option_line(line: &str) -> Vec<(char, String)> {
    let text = line.trim_start();
    // 一行多选项：`A. xx B. yy C. zz D. ww`。只在确实有 2 个以上选项时拆。
    let mut markers: Vec<(usize, char)> = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if option_boundary(text, index) {
            if let Some((key, _)) = option_marker(&text[index..]) {
                markers.push((index, key));
            }
        }
        index += text[index..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
    }
    if markers.len() >= 2 {
        let mut out = Vec::new();
        for (position, (start, key)) in markers.iter().enumerate() {
            let end = markers
                .get(position + 1)
                .map(|(next, _)| *next)
                .unwrap_or(text.len());
            if let Some((_, body)) = option_marker(&text[*start..end]) {
                out.push((*key, body.trim().to_string()));
            }
        }
        if out.len() >= 2 {
            return out;
        }
    }
    option_marker(text)
        .map(|(key, body)| vec![(key, body.trim().to_string())])
        .unwrap_or_default()
}

fn marker(line: &str) -> Option<(Field, String)> {
    let text = line.trim_start();
    let lookup = |prefix: &str| -> Option<String> {
        text.strip_prefix(prefix).map(|rest| {
            rest.trim_start_matches(['】', ']', ' ', '：', ':'])
                .trim()
                .to_string()
        })
    };
    for prefix in [
        "【答案】",
        "【参考答案】",
        "【正确答案】",
        "答案：",
        "答案:",
        "正确答案：",
        "正确答案:",
        "参考答案：",
        "参考答案:",
    ] {
        if let Some(rest) = lookup(prefix) {
            return Some((Field::Answer, rest));
        }
    }
    for prefix in [
        "【解析】",
        "【详解】",
        "【分析】",
        "【点睛】",
        "解析：",
        "解析:",
        "详解：",
        "详解:",
        "分析：",
        "分析:",
    ] {
        if let Some(rest) = lookup(prefix) {
            return Some((Field::Analysis, rest));
        }
    }
    for prefix in [
        "【知识点】",
        "【考点】",
        "知识点：",
        "知识点:",
        "考点：",
        "考点:",
    ] {
        if let Some(rest) = lookup(prefix) {
            return Some((Field::Knowledge, rest));
        }
    }
    for prefix in ["【难度】", "难度：", "难度:"] {
        if let Some(rest) = lookup(prefix) {
            return Some((Field::Difficulty, rest));
        }
    }
    None
}

fn build_item(lines: &[String], source: &str, total_conf: f64) -> Option<Value> {
    let mut field = Field::Stem;
    let mut stem: Vec<String> = Vec::new();
    let mut options: Vec<(char, String)> = Vec::new();
    let mut answer: Vec<String> = Vec::new();
    let mut analysis: Vec<String> = Vec::new();
    let mut knowledge: Vec<String> = Vec::new();
    let mut difficulty_text = String::new();

    let push = |field: Field,
                line: &str,
                stem: &mut Vec<String>,
                options: &mut Vec<(char, String)>,
                answer: &mut Vec<String>,
                analysis: &mut Vec<String>,
                knowledge: &mut Vec<String>,
                difficulty_text: &mut String| {
        match field {
            Field::Stem => stem.push(line.to_string()),
            Field::Options => {
                if let Some((_, text)) = options.last_mut() {
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(line);
                } else {
                    stem.push(line.to_string());
                }
            }
            Field::Answer => answer.push(line.to_string()),
            Field::Analysis => analysis.push(line.to_string()),
            Field::Knowledge => knowledge.push(line.to_string()),
            Field::Difficulty => {
                if !difficulty_text.is_empty() {
                    difficulty_text.push(' ');
                }
                difficulty_text.push_str(line);
            }
        }
    };

    for line in lines {
        if let Some((next, rest)) = marker(line) {
            field = next;
            if !rest.is_empty() {
                push(
                    field,
                    &rest,
                    &mut stem,
                    &mut options,
                    &mut answer,
                    &mut analysis,
                    &mut knowledge,
                    &mut difficulty_text,
                );
            }
            continue;
        }

        // 题干和选项挤在同一行时，前半句还属于题干，不能整行当选项丢掉。
        // 行中出现单个「A 的…」多半是题干里的字母，只有两个以上选项才认。
        let (lead, split) = split_options_from_text(line);
        let inline_options = !split.is_empty() && (lead.is_empty() || split.len() >= 2);
        if inline_options {
            if !lead.is_empty() {
                push(
                    field,
                    &lead,
                    &mut stem,
                    &mut options,
                    &mut answer,
                    &mut analysis,
                    &mut knowledge,
                    &mut difficulty_text,
                );
            }
            field = Field::Options;
            for (key, text) in split {
                options.push((key, text));
            }
            continue;
        }

        push(
            field,
            line,
            &mut stem,
            &mut options,
            &mut answer,
            &mut analysis,
            &mut knowledge,
            &mut difficulty_text,
        );
    }

    // 有些站点的选项混在题干最后一行，按 A/B/C/D 再兜一次。
    if options.is_empty() {
        let joined = stem.join("\n");
        let fallback = split_options_from_text(&joined);
        if fallback.1.len() >= 2 {
            options = fallback.1;
            stem = fallback
                .0
                .lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty())
                .collect();
        }
    }

    let stem_text = normalize_stem(&stem.join("\n"));
    if stem_text.trim().is_empty() || is_web_ui_stem(&stem_text) {
        return None;
    }
    let answer_text = answer.join("\n").trim().to_string();
    let analysis_text = analysis.join("\n").trim().to_string();
    let knowledge_names: Vec<String> = knowledge
        .join(" ")
        .split(['、', '，', ',', '；', ';', ' '])
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect();
    let kind = if options.is_empty() {
        if stem_text.contains("___") || stem_text.contains("____") {
            "FILL"
        } else {
            "SOLVED"
        }
    } else if answer_text
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .count()
        > 1
        && stem_text.contains("多选")
    {
        "MULTI"
    } else {
        "SINGLE"
    };
    let mut warnings = Vec::new();
    // Vision 对中文的置信度普遍在 0.5 上下，0.55 会让每张干净截图都带警告。
    if total_conf > 0.0 && total_conf < 0.35 {
        warnings.push("本地 OCR 置信度偏低，请核对题干与选项".to_string());
    }
    if options.is_empty()
        && answer_text.chars().count() <= 3
        && answer_text.chars().all(|c| c.is_ascii_uppercase())
    {
        warnings.push("答案看起来是选项字母，但没认出选项，请人工补全".to_string());
    }

    Some(json!({
        "type": kind,
        "stem": stem_text,
        "options": options
            .into_iter()
            .map(|(key, text)| json!({"key": key.to_string(), "text": text}))
            .collect::<Vec<_>>(),
        "answer": answer_text,
        "analysis": analysis_text,
        "difficulty": parse_difficulty(&difficulty_text),
        "knowledgeNames": knowledge_names,
        "source": source,
        "confidence": total_conf,
        "warnings": warnings,
        "ocr": true,
    }))
}

/// 题干里混着选项时的兜底：返回（去掉选项后的题干，选项列表）。
fn split_options_from_text(text: &str) -> (String, Vec<(char, String)>) {
    let mut first: Option<usize> = None;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if option_boundary(text, index) && option_marker(&text[index..]).is_some() {
            first = Some(index);
            break;
        }
        index += text[index..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
    }
    let Some(start) = first else {
        return (text.to_string(), Vec::new());
    };
    let options = split_option_line(&text[start..]);
    (text[..start].trim().to_string(), options)
}

fn normalize_stem(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    // 行末的空白和连续空行不影响阅读，清一遍。
    out.trim().to_string()
}

fn parse_difficulty(text: &str) -> i64 {
    let value = text.trim();
    if let Ok(number) = value.parse::<i64>() {
        return number.clamp(1, 5);
    }
    if value.contains("易") || value.contains("简单") || value.contains("基础") {
        2
    } else if value.contains("较难") || value.contains("难") || value.contains("高") {
        4
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, confidence: f64) -> Line {
        Line {
            text: text.to_string(),
            bbox: [0.0, 0.0, 100.0, 10.0],
            confidence,
        }
    }

    #[test]
    fn web_ui_toolbar_is_not_a_question() {
        // 组卷网页面底部的按钮文字，整段拼出来应识别为垃圾。
        let junk = "今日\n0次组卷\n1卷引用：北京市顺义区第九中学2026-2027学年试卷\n相似题\n纠错\n详情\n收藏\n加入试题篮";
        assert!(is_web_ui_stem(junk));

        // 正常题目不能被误伤。
        assert!(!is_web_ui_stem("已知集合 A={1,2}，求 A 的子集个数。"));
        assert!(!is_web_ui_stem("收藏本品并在今日下单，可享优惠——这件商品原价多少？"));

        // 走完整拆题链路：题号 + 按钮行不应产出题目。
        let lines = vec![
            line("1．已知 $x=2$，求 $x^2$。", 0.9),
            line("2．今日", 0.8),
            line("0次组卷", 0.8),
            line("相似题 纠错 详情 收藏 加入试题篮", 0.8),
        ];
        let items = questions_from_lines(&lines, "");
        assert_eq!(items.len(), 1, "网页按钮行不能拆成第二道题");
        assert!(items[0]["stem"].as_str().unwrap().contains("x^2"));
    }

    #[test]
    fn splits_two_numbered_questions_and_options() {
        let lines = vec![
            line("1. 设集合 A={1,2}，则 A 的子集个数是（ ）", 0.92),
            line("A. 2 B. 3 C. 4 D. 5", 0.9),
            line("答案：C", 0.95),
            line("解析：2 个元素有 2^2=4 个子集。", 0.9),
            line("2. 函数 y=ln(x-1) 的定义域是____。", 0.88),
            line("答案：(1,+∞)", 0.9),
        ];
        let items = questions_from_lines(&lines, "https://example.com/p/1");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["type"], "SINGLE");
        assert_eq!(items[0]["options"].as_array().unwrap().len(), 4);
        assert_eq!(items[0]["answer"], "C");
        assert!(items[0]["analysis"].as_str().unwrap().contains("子集"));
        assert_eq!(items[1]["type"], "FILL");
        assert_eq!(items[1]["answer"], "(1,+∞)");
    }

    #[test]
    fn keeps_unlabeled_text_as_one_question() {
        let lines = vec![
            line("求函数 f(x)=x^2+1 的最小值。", 0.8),
            line("答案：1", 0.9),
        ];
        let items = questions_from_lines(&lines, "");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["type"], "SOLVED");
        assert_eq!(items[0]["answer"], "1");
    }

    #[test]
    fn decimal_lines_are_not_question_numbers() {
        let lines = vec![line("1.5 米等于多少厘米？", 0.8)];
        let items = questions_from_lines(&lines, "");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["stem"], "1.5 米等于多少厘米？");
    }

    #[test]
    fn marker_inside_line_splits_answer() {
        let lines = vec![line(
            "下列说法正确的是（ ） A. 甲 B. 乙 答案：B 解析：乙正确。",
            0.8,
        )];
        let items = questions_from_lines(&lines, "");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["answer"], "B");
        assert!(items[0]["analysis"].as_str().unwrap().contains("乙正确"));
    }

    #[test]
    fn difficulty_words_and_numbers() {
        assert_eq!(parse_difficulty("易"), 2);
        assert_eq!(parse_difficulty("较难"), 4);
        assert_eq!(parse_difficulty("5"), 5);
        assert_eq!(parse_difficulty(""), 3);
    }

    /// 真机探针：`NB_OCR_IMAGE=/path/to.png cargo test real_image_probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_image_probe() {
        let Ok(path) = std::env::var("NB_OCR_IMAGE") else {
            return;
        };
        let items = recognize_questions(&path, "probe").expect("本机 OCR 失败");
        for item in &items {
            println!("{}", serde_json::to_string_pretty(item).unwrap());
        }
        assert!(!items.is_empty());
    }
}
