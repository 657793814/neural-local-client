//! 本地文件取题：把本机的图片 / PDF / Word / txt 变成采集台里可预览、可识别的输入。
//!
//! 为什么要这一层：老师手里的整卷常常是下载来的 PDF、别人发来的 Word，不是网页也不是
//! 屏幕上的东西。「截图选区」抓不到没打开的窗口，「选图片」又只认 jpg/png。这里做四件事：
//!
//! 1. 认文件类型（扩展名 + 文件头双判，避免改了后缀的图骗过人）；
//! 2. 图片直接读；读不出来（HEIC / TIFF / BMP 这类）用系统 `sips` 转成 PNG；
//! 3. PDF 用 CoreGraphics 逐页栅格化（`CGPDFDocument` + `CGContextDrawPDFPage`），
//!    不联网、不带第三方二进制，翻到第几页就渲第几页；
//! 4. Word / txt 只取文字：它们没有「页」也没有版面，塞给视觉模型没意义，
//!    所以直接抽出纯文本，交给「文字取题」那条链路（同粘贴文字）。
//!    `.docx` 自己解 zip 里的 `word/document.xml`，老 `.doc` / `.rtf` 交给系统 `textutil`。
//!
//! 渲出来的图落在 `data_dir/tmp`，不能当场删：识别任务和入库都要按路径再读一次
//! （和截图那条链路的约束一致，见 capture.rs 顶部注释）。

use crate::models::{CaptureResult, LocalFile};
use std::path::{Path, PathBuf};

/// 预览长边像素：太小看不清题，太大 data URL 会拖慢界面。
/// 1800 对 A4 整卷≈1300×1800，单页 PNG 通常几百 KB，够模型认题也够人核对。
const PREVIEW_EDGE: u32 = 1800;

/// 只有文字、没有版面的文件。它们走文字取题，不渲染成图。
const TEXT_EXTS: &[&str] = &[
    "txt",
    "text",
    "md",
    "markdown",
    "csv",
    "tsv",
    "docx",
    "docm",
    "doc",
    "rtf",
    "odt",
    "html",
    "htm",
    "webarchive",
];

fn ext_of(path: &Path) -> String {
    path.extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn name_of(path: &Path) -> String {
    path.file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "本地文件".to_string())
}

fn is_text_ext(ext: &str) -> bool {
    TEXT_EXTS.contains(&ext)
}

/// 文件头是 `%PDF-` 才算 PDF：下载来的卷子常被改名成 `.pdf` 或干脆没有后缀。
fn looks_like_pdf(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 5];
    match std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut head)) {
        Ok(()) => &head == b"%PDF-",
        Err(_) => false,
    }
}

fn tmp_path(data_dir: &Path, prefix: &str, ext: &str) -> Result<PathBuf, String> {
    let dir = data_dir.join("tmp");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;
    Ok(dir.join(format!("{prefix}-{}.{ext}", uuid::Uuid::new_v4())))
}

/// 用系统 `sips` 把系统认得、但 `image` crate 不认的格式转成 PNG。
#[cfg(target_os = "macos")]
fn via_sips(path: &Path, data_dir: &Path) -> Result<PathBuf, String> {
    let out = tmp_path(data_dir, "local", "png")?;
    let status = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "png"])
        .arg(path)
        .arg("--out")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("调不起系统图像转换：{e}"))?;
    if !status.success() || !out.exists() {
        return Err(format!(
            "这个文件打不开（{}）。支持图片、PDF、Word、txt；其它格式（PPT / Excel 等）请先另存为 PDF。",
            path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default()
        ));
    }
    Ok(out)
}

#[cfg(not(target_os = "macos"))]
fn via_sips(path: &Path, _data_dir: &Path) -> Result<PathBuf, String> {
    // TIFF / BMP 已由 image crate 原生解码（见 Cargo.toml features），走到这里
    // 基本只剩 HEIC / HEIF：Windows 没有系统解码器，报可操作错误而不是笼统失败。
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext == "heic" || ext == "heif" {
        Err("HEIC / HEIF 图片在这台系统上不支持：请转成 JPG / PNG 再导入（iPhone 传输照片时选「自动」即可得到 JPG）".into())
    } else {
        Err(format!(
            "暂时读不了 {ext}：支持 png / jpg / webp / gif / tiff / bmp；其它格式请先转成 PNG 或 JPG"
        ))
    }
}

/// 读一张图片文件，转成前端要的 CaptureResult（含 data URL 预览）。
fn open_image(path: &Path, data_dir: &Path) -> Result<CaptureResult, String> {
    match crate::capture::from_path(path) {
        Ok(result) => Ok(result),
        // HEIC / TIFF / BMP 这类 image crate 不认，交给系统转一次。
        Err(_) => {
            let converted = via_sips(path, data_dir)?;
            crate::capture::from_path(&converted)
        }
    }
}

/// 打开本地文件：图片返回自己，PDF 返回指定页的栅格图，Word / txt 返回抽出来的文字。
pub fn open(path: &Path, page: i64, data_dir: &Path) -> Result<LocalFile, String> {
    if !path.exists() {
        return Err(format!("文件不在了：{}", path.to_string_lossy()));
    }
    let ext = ext_of(path);
    if ext == "pdf" || looks_like_pdf(path) {
        let pages = pdf_pages(path)?;
        let page = page.clamp(1, pages.max(1) as i64);
        let image = pdf_page(path, page as u32, data_dir)?;
        return Ok(LocalFile {
            path: path.to_string_lossy().to_string(),
            name: name_of(path),
            kind: "pdf".to_string(),
            page,
            pages: pages as i64,
            image: Some(image),
            text: None,
        });
    }
    if is_text_ext(&ext) {
        let text = read_text(path, &ext)?;
        if text.trim().is_empty() {
            return Err(format!(
                "{} 里没读到文字：可能是扫描件或整页图片，请另存为 PDF 再打开",
                name_of(path)
            ));
        }
        return Ok(LocalFile {
            path: path.to_string_lossy().to_string(),
            name: name_of(path),
            kind: "text".to_string(),
            page: 1,
            pages: 1,
            image: None,
            text: Some(text),
        });
    }
    let image = open_image(path, data_dir)?;
    Ok(LocalFile {
        path: path.to_string_lossy().to_string(),
        name: name_of(path),
        kind: "image".to_string(),
        page: 1,
        pages: 1,
        image: Some(image),
        text: None,
    })
}

// ---------------------------------------------------------------------------
// 文字类文件：Word / txt / rtf
// ---------------------------------------------------------------------------

fn read_text(path: &Path, ext: &str) -> Result<String, String> {
    match ext {
        "docx" | "docm" => docx_text(path),
        "doc" | "rtf" | "odt" | "html" | "htm" | "webarchive" => via_textutil(path),
        _ => plain_text(path),
    }
}

/// 纯文本：先按 UTF-8 读，不行再按 GB18030（中文卷子常见的 GBK 系）。
fn plain_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读不了这个文件：{e}"))?;
    if bytes.is_empty() {
        return Ok(String::new());
    }
    if let Some(body) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Ok(String::from_utf8_lossy(body).to_string());
    }
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Ok(encoding_rs::UTF_16LE.decode(body).0.into_owned());
    }
    if let Some(body) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Ok(encoding_rs::UTF_16BE.decode(body).0.into_owned());
    }
    if let Ok(text) = std::str::from_utf8(&bytes) {
        return Ok(text.to_string());
    }
    let (text, _, had_errors) = encoding_rs::GB18030.decode(&bytes);
    if had_errors {
        return Err(
            "这个文本文件的编码认不出来（试过 UTF-8 / GB18030）；改用「粘贴文字」把内容贴进来更稳"
                .into(),
        );
    }
    Ok(text.into_owned())
}

/// `.docx` 就是一个 zip，正文在 `word/document.xml`。
/// 只解 `<w:t>` 里的文字够用了：题目本来就没有表格 / 图片之外的排版要保留。
fn docx_text(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("打不开这个文件：{e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| {
        "这个 .docx 打不开：可能已损坏，或者是旧版 .doc 改了后缀（请用 Word 另存为 .docx）"
            .to_string()
    })?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .map_err(|_| "这个 .docx 里没有正文（word/document.xml）".to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| format!("这个 .docx 的正文读不出来：{e}"))?;
    let text = xml_to_text(&xml);
    Ok(tidy_lines(text))
}

/// 从 WordprocessingML 里抠纯文本：
/// `<w:p>` 分段、`<w:br>` 换行、`<w:tab>` 制表，`<w:t>` / `<m:t>` 是文字本身
/// （后者是公式，公式里的字符也是题目的组成部分，不能丢）。
fn xml_to_text(xml: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0usize;
    while let Some(open_rel) = xml[cursor..].find('<') {
        let open = cursor + open_rel;
        let Some(close_rel) = xml[open..].find('>') else {
            break;
        };
        let close = open + close_rel;
        let tag = &xml[open + 1..close];
        cursor = close + 1;
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        match name {
            "w:p" | "w:br" | "w:cr" => out.push('\n'),
            "w:tab" => out.push('\t'),
            "w:t" | "m:t" => {
                let end_tag = format!("</{name}>");
                let Some(end_rel) = xml[cursor..].find(&end_tag) else {
                    continue;
                };
                out.push_str(&unescape_xml(&xml[cursor..cursor + end_rel]));
                cursor += end_rel + end_tag.len();
            }
            _ => {}
        }
    }
    out
}

fn unescape_xml(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let (replacement, width) = if tail.starts_with("&amp;") {
            ('&', 5)
        } else if tail.starts_with("&lt;") {
            ('<', 4)
        } else if tail.starts_with("&gt;") {
            ('>', 4)
        } else if tail.starts_with("&quot;") {
            ('"', 6)
        } else if tail.starts_with("&apos;") {
            ('\'', 6)
        } else if let Some(semi) = tail[..tail.len().min(10)].find(';') {
            // 数字实体：&#12289; / &#x3001;
            let body = &tail[2..semi];
            let code = if let Some(hex) = body.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                body.parse::<u32>().ok()
            };
            match code.and_then(char::from_u32) {
                Some(ch) => (ch, semi + 1),
                None => {
                    out.push('&');
                    rest = &tail[1..];
                    continue;
                }
            }
        } else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        out.push(replacement);
        rest = &tail[width..];
    }
    out.push_str(rest);
    out
}

/// 去掉首尾空行、把 3 个以上连续空行压成一个，免得整页都是空白。
fn tidy_lines(text: String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            out.push('\n');
        } else {
            blank_run = 0;
            out.push_str(trimmed);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

/// 老 `.doc` / `.rtf` / `.odt` / `.html`：macOS 自带 `textutil`，不必引第三方解析器。
#[cfg(target_os = "macos")]
fn via_textutil(path: &Path) -> Result<String, String> {
    let output = std::process::Command::new("/usr/bin/textutil")
        .args(["-convert", "txt", "-stdout"])
        .arg(path)
        .output()
        .map_err(|e| format!("调不起系统文档转换：{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "这个文件读不出文字（{}）。旧版 .doc 请在 Word 里另存为 .docx 或 PDF 再打开",
            path.extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default()
        ));
    }
    Ok(tidy_lines(
        String::from_utf8_lossy(&output.stdout).to_string(),
    ))
}

/// 非 macOS：`.rtf` 走纯 Rust 的 striprtf（剥控制词、保留正文），`.html` 剥标签；
/// `.doc` / `.odt` / `.webarchive` 没有像样的纯 Rust 解析器，报引导性错误而不是硬猜。
#[cfg(not(target_os = "macos"))]
fn via_textutil(path: &Path) -> Result<String, String> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rtf" => {
            let bytes = std::fs::read(path).map_err(|e| format!("读不了这个文件：{e}"))?;
            let raw = String::from_utf8_lossy(&bytes);
            Ok(tidy_lines(rtf_to_text(&raw)))
        }
        "html" | "htm" => {
            let bytes = std::fs::read(path).map_err(|e| format!("读不了这个文件：{e}"))?;
            let raw = String::from_utf8_lossy(&bytes);
            Ok(tidy_lines(strip_html(&raw)))
        }
        other => Err(format!(
            "{other} 这类文档目前只在 macOS 上能直接读：.doc 请在 Word 里另存为 .docx 或 PDF；.rtf / html / docx / txt 可以直接打开"
        )),
    }
}

/// 极简 HTML 转文字：去掉 style / script 块与所有标签，实体交给 unescape_xml。
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn strip_html(html: &str) -> String {
    // 先整块删掉 <style>…</style> / <script>…</script>（大小写不敏感）。
    let mut cleaned = html.to_string();
    for tag in ["script", "style"] {
        loop {
            let lower = cleaned.to_lowercase();
            let open = match lower.find(&format!("<{tag}")) {
                Some(pos) => pos,
                None => break,
            };
            let close_tag = format!("</{tag}>");
            let end = match lower[open..].find(&close_tag) {
                Some(rel) => open + rel + close_tag.len(),
                // 没闭合就删到结尾，避免整篇被吞掉正文之外的内容
                None => cleaned.len(),
            };
            cleaned.replace_range(open..end, " ");
        }
    }
    // 再剥掉剩余标签；标签之间补空格防止文字粘连。
    let mut out = String::with_capacity(cleaned.len());
    let mut in_tag = false;
    for ch in cleaned.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    unescape_xml(&out)
}

// ---------------------------------------------------------------------------
// RTF：纯 Rust 迷你解析器（Windows 侧替代 textutil；macOS 仍走 textutil）
// ---------------------------------------------------------------------------

fn rtf_decode_bytes(bytes: &[u8], codepage: &str) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let enc = match codepage {
        "936" => encoding_rs::GBK,
        "950" => encoding_rs::BIG5,
        _ => encoding_rs::WINDOWS_1252,
    };
    enc.decode(bytes).0.into_owned()
}

/// 极简 RTF 转文字。覆盖取题场景需要的部分：
/// - 目的地组（fonttbl / colortbl / stylesheet / pict / info 等）整组丢弃，`{\* …}` 同理；
/// - `\'hh` 十六进制字节按文档代码页解码（`\ansicpg936` → GBK，默认 Windows-1252）；
/// - `\uNNNN` Unicode 字符（含负数补码写法），`\ucN` 决定其后要跳过的替代字符数；
/// - `\par` / `\line` 换行、`\tab` 制表、常见引号 / 破折号转义、`\\` `\{` `\}` 字面量；
/// - 其余控制词与控制符号一律忽略。
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn rtf_to_text(input: &str) -> String {
    struct Group {
        skip: bool,
        uc: usize,
    }

    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut codepage = String::from("1252");
    let mut stack: Vec<Group> = vec![Group { skip: false, uc: 1 }];
    let mut pending_skip = 0usize;
    let mut just_opened = false;
    let mut i = 0usize;

    fn flush(out: &mut String, pending: &mut Vec<u8>, codepage: &str) {
        if !pending.is_empty() {
            out.push_str(&rtf_decode_bytes(pending, codepage));
            pending.clear();
        }
    }

    while i < chars.len() {
        let ch = chars[i];
        if ch == '{' {
            let parent_skip = stack.last().map(|g| g.skip).unwrap_or(false);
            stack.push(Group { skip: parent_skip, uc: 1 });
            just_opened = true;
            i += 1;
            continue;
        }
        if ch == '}' {
            flush(&mut out, &mut pending, &codepage);
            stack.pop();
            i += 1;
            continue;
        }
        let cur_skip = stack.last().map(|g| g.skip).unwrap_or(false);
        if ch == '\\' {
            i += 1;
            match chars.get(i).copied() {
                Some('*') => {
                    if let Some(g) = stack.last_mut() {
                        g.skip = true;
                    }
                    i += 1;
                }
                Some('\'') => {
                    let hex = |k: usize| {
                        chars.get(k).and_then(|c| c.to_digit(16)).unwrap_or(0) as u8
                    };
                    let byte = hex(i + 1) * 16 + hex(i + 2);
                    i += 3;
                    if !cur_skip {
                        if pending_skip > 0 {
                            pending_skip -= 1;
                        } else {
                            pending.push(byte);
                        }
                    }
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    let start = i;
                    while i < chars.len() && chars[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word: String = chars[start..i].iter().collect();
                    let mut num: Option<i64> = None;
                    let mut neg = false;
                    if i < chars.len() && chars[i] == '-' {
                        neg = true;
                        i += 1;
                    }
                    let ds = i;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i > ds {
                        let v: i64 = chars[ds..i].iter().collect::<String>().parse().unwrap_or(0);
                        num = Some(if neg { -v } else { v });
                    }
                    if i < chars.len() && chars[i] == ' ' {
                        i += 1;
                    }
                    if word == "uc" {
                        let n = num.unwrap_or(1).max(0) as usize;
                        if let Some(g) = stack.last_mut() {
                            g.uc = n;
                        }
                    } else if !cur_skip {
                        match word.as_str() {
                            "ansicpg" => {
                                if let Some(n) = num {
                                    codepage = n.abs().to_string();
                                }
                            }
                            "u" => {
                                let v = num.unwrap_or(0) as u16;
                                flush(&mut out, &mut pending, &codepage);
                                if let Some(c) = char::from_u32(v as u32) {
                                    out.push(c);
                                }
                                pending_skip = stack.last().map(|g| g.uc).unwrap_or(1);
                            }
                            "par" | "line" | "sect" | "page" | "cell" | "row" => {
                                flush(&mut out, &mut pending, &codepage);
                                out.push('\n');
                            }
                            "tab" => out.push('\t'),
                            "emdash" => out.push('—'),
                            "endash" => out.push('–'),
                            "lquote" => out.push('‘'),
                            "rquote" => out.push('’'),
                            "ldblquote" => out.push('“'),
                            "rdblquote" => out.push('”'),
                            "bullet" => out.push('•'),
                            "_" | "-" => out.push('-'),
                            "~" => out.push('\u{00A0}'),
                            _ => {}
                        }
                    }
                    // 紧跟组开头的目的地控制词：整组丢弃（头部表、图片、页眉页脚……）
                    if just_opened
                        && matches!(
                            word.as_str(),
                            "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict"
                                | "object" | "thumbnail" | "listtable" | "rsidtbl"
                                | "generator" | "xmlnstbl" | "header" | "footer"
                                | "headerf" | "footerf" | "latentstyles" | "datastore"
                                | "themedata" | "colorschememapping" | "wgrffmtfilter"
                                | "pn" | "pntext" | "pntxtb" | "pntxta" | "bkmkstart"
                                | "bkmkend" | "shpinst" | "nonshppict" | "faltcontent"
                        )
                    {
                        if let Some(g) = stack.last_mut() {
                            g.skip = true;
                        }
                    }
                }
                Some(c) => {
                    i += 1;
                    if !cur_skip {
                        match c {
                            '\\' | '{' | '}' => {
                                flush(&mut out, &mut pending, &codepage);
                                out.push(c);
                            }
                            '~' => out.push('\u{00A0}'),
                            '_' | '-' => out.push('-'),
                            _ => {}
                        }
                    }
                }
                None => {
                    i += 1;
                }
            }
            just_opened = false;
            continue;
        }
        // 普通字符：跳过组直接丢弃；\u 之后的替代字符按 \ucN 跳过
        if pending_skip > 0 {
            pending_skip -= 1;
        } else if !cur_skip {
            flush(&mut out, &mut pending, &codepage);
            out.push(ch);
        }
        just_opened = false;
        i += 1;
    }
    flush(&mut out, &mut pending, &codepage);
    out
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

/// PDF 页数。macOS 走 CoreGraphics，Windows 走 pdfium，其它平台不支持。
fn pdf_pages(path: &Path) -> Result<usize, String> {
    #[cfg(target_os = "macos")]
    {
        mac::page_count(path)
    }
    #[cfg(windows)]
    {
        pdfium_backend::page_count(path)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
        Err("PDF 预览目前支持 macOS 与 Windows".into())
    }
}

fn pdf_page(path: &Path, page: u32, data_dir: &Path) -> Result<CaptureResult, String> {
    #[cfg(target_os = "macos")]
    {
        let rendered = mac::render(path, page, PREVIEW_EDGE)?;
        let out = tmp_path(data_dir, "pdf", "png")?;
        rendered
            .save(&out)
            .map_err(|e| format!("保存 PDF 页失败：{e}"))?;
        crate::capture::from_path(&out)
    }
    #[cfg(windows)]
    {
        let rendered = pdfium_backend::render(path, page, PREVIEW_EDGE)?;
        let out = tmp_path(data_dir, "pdf", "png")?;
        rendered
            .save(&out)
            .map_err(|e| format!("保存 PDF 页失败：{e}"))?;
        crate::capture::from_path(&out)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (path, page, data_dir);
        Err("PDF 预览目前支持 macOS 与 Windows".into())
    }
}

/// Windows：pdfium 渲染。pdfium-render 运行时加载 `pdfium.dll`（程序目录或系统 PATH），
/// 找不到时报可操作错误；随包分发见 docs/windows-port-plan.md §5。
#[cfg(target_os = "windows")]
mod pdfium_backend {
    use pdfium_render::prelude::*;
    use std::path::Path;

    fn pdfium() -> Result<Pdfium, String> {
        let bindings = Pdfium::bind_to_system_library().map_err(|_| {
            "找不到 pdfium.dll：请把它放到程序目录（与 NeuralKB Local.exe 同级）或系统 PATH 里，详见安装说明"
                .to_string()
        })?;
        Ok(Pdfium::new(bindings))
    }

    fn load(path: &Path) -> Result<(Pdfium, PdfDocument), String> {
        let pdfium = pdfium()?;
        let doc = pdfium
            .load_pdf_from_file(path, None)
            .map_err(|e| format!("这个 PDF 打不开：可能已损坏或设了打开密码（{e}）"))?;
        Ok((pdfium, doc))
    }

    pub fn page_count(path: &Path) -> Result<usize, String> {
        let (_pdfium, doc) = load(path)?;
        let pages = doc.pages().len();
        if pages == 0 {
            return Err("这个 PDF 里没有页面".into());
        }
        Ok(pages)
    }

    /// 把第 `page`（1 起）页栅格化成 PNG 像素，长边不超过 `edge`。
    pub fn render(path: &Path, page: u32, edge: u32) -> Result<image::RgbaImage, String> {
        let (_pdfium, doc) = load(path)?;
        let total = doc.pages().len();
        if page as usize > total {
            return Err(format!("这个 PDF 只有 {total} 页"));
        }
        let target = doc
            .pages()
            .get(page as u16 - 1)
            .map_err(|e| format!("读不出第 {page} 页：{e}"))?;
        let page_w = target.width().value();
        let page_h = target.height().value();
        if page_w <= 1.0 || page_h <= 1.0 {
            return Err(format!("第 {page} 页尺寸异常，渲不出来"));
        }
        let scale = (edge as f32 / page_w.max(page_h)).min(3.0).max(0.2);
        let width = (page_w * scale).round().max(1.0) as i16;
        let height = (page_h * scale).round().max(1.0) as i16;

        // 白底：卷子多是白纸黑字，与 macOS 侧保持同一行为。
        let config = PdfRenderConfig::new()
            .set_target_size(width, height)
            .set_clear_color(255, 255, 255, 255);
        let bitmap = target
            .render_with_config(&config)
            .map_err(|e| format!("渲染 PDF 页失败：{e}"))?;
        let rendered = bitmap
            .as_image()
            .ok_or_else(|| "渲染结果拿不到".to_string())?;
        Ok(rendered.to_rgba8())
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2_core_foundation::{CGPoint, CGRect, CGSize, CFURL};
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo,
        CGPDFBox, CGPDFDocument,
    };
    use std::ffi::c_void;
    use std::path::Path;

    fn document(path: &Path) -> Result<objc2_core_foundation::CFRetained<CGPDFDocument>, String> {
        let url = CFURL::from_file_path(path).ok_or_else(|| "这个文件路径打不开".to_string())?;
        CGPDFDocument::with_url(Some(&url))
            .ok_or_else(|| "这个 PDF 打不开：可能已损坏或设了打开密码".to_string())
    }

    pub fn page_count(path: &Path) -> Result<usize, String> {
        let doc = document(path)?;
        let pages = CGPDFDocument::number_of_pages(Some(&doc));
        if pages == 0 {
            return Err("这个 PDF 里没有页面".into());
        }
        Ok(pages)
    }

    /// 把第 `page`（1 起）页栅格化成 PNG 像素，长边不超过 `edge`。
    pub fn render(path: &Path, page: u32, edge: u32) -> Result<image::RgbaImage, String> {
        let doc = document(path)?;
        let total = CGPDFDocument::number_of_pages(Some(&doc));
        if page as usize > total {
            return Err(format!("这个 PDF 只有 {total} 页"));
        }
        let target = CGPDFDocument::page(Some(&doc), page as usize)
            .ok_or_else(|| format!("读不出第 {page} 页"))?;
        let rect = objc2_core_graphics::CGPDFPage::box_rect(Some(&target), CGPDFBox::MediaBox);
        let (page_w, page_h) = (rect.size.width, rect.size.height);
        if page_w <= 1.0 || page_h <= 1.0 {
            return Err(format!("第 {page} 页尺寸异常，渲不出来"));
        }
        let scale = (edge as f64 / page_w.max(page_h)).min(3.0).max(0.2);
        let width = (page_w * scale).round().max(1.0) as u32;
        let height = (page_h * scale).round().max(1.0) as u32;

        let mut buffer = vec![0u8; width as usize * height as usize * 4];
        let space = CGColorSpace::new_device_rgb().ok_or_else(|| "取不到色彩空间".to_string())?;
        let bitmap_info =
            CGImageAlphaInfo::PremultipliedLast.0 | CGImageByteOrderInfo::Order32Big.0;
        // 自备缓冲区：渲完直接从 buffer 取像素，不用再向 CG 要一次指针。
        let context = unsafe {
            CGBitmapContextCreate(
                buffer.as_mut_ptr() as *mut c_void,
                width as usize,
                height as usize,
                8,
                width as usize * 4,
                Some(&space),
                bitmap_info,
            )
        }
        .ok_or_else(|| "建位图上下文失败（可能内存不够）".to_string())?;

        // 白底：卷子多是白纸黑字，透明的角落转成 PNG 会变黑，模型容易认错。
        CGContext::set_rgb_fill_color(Some(&context), 1.0, 1.0, 1.0, 1.0);
        CGContext::fill_rect(
            Some(&context),
            CGRect::new(
                CGPoint::new(0.0, 0.0),
                CGSize::new(width as f64, height as f64),
            ),
        );
        CGContext::scale_ctm(Some(&context), scale, scale);
        CGContext::draw_pdf_page(Some(&context), Some(&target));
        drop(context);

        image::RgbaImage::from_raw(width, height, buffer)
            .ok_or_else(|| "渲染结果尺寸不对".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtf_strips_control_words_and_keeps_text() {
        let text = rtf_to_text(
            "{\\rtf1\\ansi\\deff0 {\\fonttbl{\\f0 Times;}}\n\\f0\\fs24 1. 已知函数$f(x)=x^2$，求导。\\par (A) $2x$\\par (B) $x$\\par}",
        );
        assert!(text.contains("1. 已知函数"), "控制词应剥干净：{text}");
        assert!(text.contains("(A) $2x$"), "正文要保留：{text}");
        assert!(!text.contains("Times"), "fonttbl 组要整组丢弃：{text}");
        assert!(!text.contains('\\'), "不应残留反斜杠：{text}");
    }

    #[test]
    fn rtf_decodes_unicode_and_hex_escapes() {
        // \u-29739 是「试」(U+8BD5) 的补码写法；\'41 是 ASCII 'A'。
        let text = rtf_to_text("{\\rtf1\\ansi\\uc1\\u-29739 ?\\'41\\'42}");
        assert!(text.contains("试"), "Unicode 补码要还原：{text}");
        assert!(text.contains("AB"), "十六进制字节要还原：{text}");
        assert!(!text.contains('?'), "\\uc1 的替代字符要跳过：{text}");
    }

    #[test]
    fn rtf_decodes_gbk_hex_bytes() {
        // \ansicpg936 下，「数」的 GBK 编码是 CA FD。
        let text = rtf_to_text("{\\rtf1\\ansi\\ansicpg936 \\'CA\\'FD\\'D1\\'A7}");
        assert!(text.contains("数学"), "GBK 字节应按代码页解码：{text}");
    }

    #[test]
    fn rtf_keeps_escaped_braces_and_newlines() {
        let text = rtf_to_text("{\\rtf1 行一\\par 行二\\{字\\}}");
        assert!(text.contains("行一\n行二{字}"), "换行与转义字面量：{text}");
    }

    #[test]
    fn html_strips_tags_and_keeps_text() {
        let html = "<html><head><style>p{color:red}</style></head><body><p>1+1=<b>2</b></p><script>var x=1;</script></body></html>";
        let text = strip_html(html);
        assert!(text.contains("1+1="), "正文要保留：{text}");
        assert!(text.contains("2"), "加粗文字要保留：{text}");
        assert!(!text.contains("color"), "style 块要整块删除：{text}");
        assert!(!text.contains("var x"), "script 块要整块删除：{text}");
    }

    #[test]
    fn detects_names_and_extensions() {
        assert_eq!(name_of(Path::new("/tmp/期中卷.pdf")), "期中卷");
        assert_eq!(ext_of(Path::new("/tmp/A.PDF")), "pdf");
        assert!(looks_like_pdf(Path::new("/tmp/nope-not-here.pdf")) == false);
        // 不存在的文件走「像不像 PDF」只能拿到 false，不能 panic。
        assert_eq!(name_of(Path::new("/tmp/noext")), "noext");
    }

    #[test]
    fn missing_file_is_reported_clearly() {
        let dir = std::env::temp_dir().join(format!("nb-filein-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let err = open(Path::new("/tmp/definitely-missing-nb.pdf"), 1, &dir).unwrap_err();
        assert!(err.contains("不在了"), "应说清文件不存在：{err}");
    }

    #[test]
    fn text_extensions_are_routed_to_text() {
        for ext in ["docx", "doc", "rtf", "txt", "md", "csv"] {
            assert!(is_text_ext(ext), "{ext} 应走文字取题");
        }
        for ext in ["pdf", "png", "jpg", "heic"] {
            assert!(!is_text_ext(ext), "{ext} 不该走文字取题");
        }
    }

    #[test]
    fn reads_utf8_and_gbk_text_files() {
        let dir = std::env::temp_dir().join(format!("nb-text-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("建临时目录");

        let utf8 = dir.join("卷子.txt");
        std::fs::write(&utf8, "1. 已知函数 $f(x)=x^2$\n求最值").expect("写 utf8");
        let file = open(&utf8, 1, &dir).expect("打开 txt");
        assert_eq!(file.kind, "text");
        assert!(file.image.is_none(), "文本文件不该有栅格图");
        assert!(file.text.expect("有文字").contains("求最值"));

        // 老 txt 常是 GBK：GB18030 里「数学」= D1 A7 CA FD? 这里用编码器回转一遍最稳。
        let (encoded, _, _) = encoding_rs::GB18030.encode("2. 数学试卷");
        let gbk = dir.join("老卷.txt");
        std::fs::write(&gbk, &encoded[..]).expect("写 gbk");
        let file = open(&gbk, 1, &dir).expect("打开 gbk txt");
        assert_eq!(file.text.expect("有文字"), "2. 数学试卷");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn docx_text_is_extracted_from_document_xml() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("nb-docx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let path = dir.join("试卷.docx");

        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="x"><w:body>
<w:p><w:r><w:t>1. 已知椭圆</w:t></w:r><w:r><w:t xml:space="preserve"> C 的焦点为 F1</w:t></w:r></w:p>
<w:p><w:r><w:t>A. 4</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>B. 5 &amp; 6</w:t></w:r></w:p>
<w:p><w:r><m:t>x^2</m:t></w:r></w:p>
</w:body></w:document>"#;

        {
            let file = std::fs::File::create(&path).expect("建 docx");
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("加正文");
            zip.write_all(xml.as_bytes()).expect("写正文");
            zip.finish().expect("收尾");
        }

        let file = open(&path, 1, &dir).expect("打开 docx");
        assert_eq!(file.kind, "text");
        let text = file.text.expect("有文字");
        assert!(
            text.contains("1. 已知椭圆 C 的焦点为 F1"),
            "题干要连起来：{text}"
        );
        assert!(
            text.contains("A. 4\tB. 5 & 6"),
            "制表符与实体要还原：{text}"
        );
        assert!(text.contains("x^2"), "公式里的字符不能丢：{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unescapes_xml_entities() {
        assert_eq!(unescape_xml("a &lt; b &amp;&amp; c &#65;"), "a < b && c A");
        assert_eq!(unescape_xml("没有实体"), "没有实体");
    }
}
