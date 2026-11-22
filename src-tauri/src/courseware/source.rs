//! 现有讲义的本地文字抽取。只返回文字、来源位置和警告，不把栅格化临时图交给后续流程。

use crate::models::LocalFile;
use std::io::Read;
use std::path::Path;

const MAX_XML_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SLIDES: usize = 500;

#[derive(Debug, Clone, Default)]
pub struct ExtractedSource {
    pub text: String,
    pub warnings: Vec<String>,
    pub locations: Vec<String>,
}

pub fn extract(path: &Path, data_dir: &Path) -> Result<ExtractedSource, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("读不到这个讲义文件：{e}"))?;
    if !metadata.is_file() {
        return Err("请选择讲义文件，不要选择文件夹".into());
    }
    if metadata.len() == 0 {
        return Err("这个讲义文件是空的".into());
    }
    if metadata.len() > super::MAX_BYTES {
        return Err(format!(
            "讲义文件超过 {} MB，请先精简或拆分后再导入",
            super::MAX_BYTES / 1024 / 1024
        ));
    }

    let ext = path
        .extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "docx" | "docm" => from_filein(path, data_dir, "Word 正文"),
        "md" | "markdown" => from_filein(path, data_dir, "Markdown 正文"),
        "txt" => from_filein(path, data_dir, "文本正文"),
        "html" | "htm" => from_filein(path, data_dir, "HTML 正文"),
        "pptx" => from_pptx(path),
        "pdf" => from_pdf(path, data_dir),
        _ => Err("支持 DOCX / DOCM、PPTX、PDF、Markdown、TXT 和 HTML 讲义".into()),
    }
}

fn from_filein(path: &Path, data_dir: &Path, location: &str) -> Result<ExtractedSource, String> {
    let opened = crate::filein::open(path, 1, data_dir)?;
    let text = opened.text.unwrap_or_default().trim().to_string();
    if text.is_empty() {
        return Err("这个讲义里没有抽取到文字，请检查文件内容".into());
    }
    Ok(ExtractedSource {
        text,
        warnings: Vec::new(),
        locations: vec![location.to_string()],
    })
}

fn append_section(source: &mut ExtractedSource, location: String, text: String) {
    if text.trim().is_empty() {
        return;
    }
    if !source.text.is_empty() {
        source.text.push_str("\n\n");
    }
    source.text.push_str(&format!("【{location}】\n"));
    source.text.push_str(text.trim());
    source.locations.push(location);
}

// ---------------------------------------------------------------------------
// PPTX：幻灯片与同名备注页都按数字顺序读取，只取 DrawingML 段落文字。
// ---------------------------------------------------------------------------

fn from_pptx(path: &Path) -> Result<ExtractedSource, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("打不开这个 PPTX：{e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("这个 PPTX 打不开或已损坏：{e}"))?;
    let mut slides = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| format!("读取 PPTX 目录失败：{e}"))?;
        if let Some(number) = slide_number(entry.name()) {
            slides.push((number, entry.name().to_string()));
        }
    }
    if slides.is_empty() {
        return Err("这个 PPTX 里没有幻灯片（ppt/slides/slideN.xml）".into());
    }
    if slides.len() > MAX_SLIDES {
        return Err(format!("这个 PPTX 超过 {MAX_SLIDES} 页，请先拆分后再导入"));
    }
    slides.sort_by_key(|(number, _)| *number);

    let mut source = ExtractedSource::default();
    for (number, name) in slides {
        let xml = read_zip_text(&mut archive, &name)?
            .ok_or_else(|| format!("PPTX 里找不到幻灯片 {number}"))?;
        append_section(&mut source, format!("幻灯片 {number}"), drawing_text(&xml));

        let notes_name = format!("ppt/notesSlides/notesSlide{number}.xml");
        if let Some(xml) = read_zip_text(&mut archive, &notes_name)? {
            append_section(
                &mut source,
                format!("幻灯片 {number} 备注"),
                drawing_text(&xml),
            );
        }
    }
    source
        .warnings
        .push("PPTX 只抽取文字，复杂图形、SmartArt 与动画无法 1:1 还原".to_string());
    if source.text.trim().is_empty() {
        return Err("这个 PPTX 里没有抽取到文字：幻灯片和备注可能都是图片".into());
    }
    Ok(source)
}

fn slide_number(name: &str) -> Option<u32> {
    let number = name
        .strip_prefix("ppt/slides/slide")?
        .strip_suffix(".xml")?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

fn read_zip_text(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
) -> Result<Option<String>, String> {
    let entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(format!("读取 PPTX 的 {name} 失败：{error}")),
    };
    if entry.size() > MAX_XML_BYTES {
        return Err(format!("PPTX 的 {name} 过大，无法安全抽取"));
    }
    let mut bytes = Vec::new();
    entry
        .take(MAX_XML_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读取 PPTX 的 {name} 失败：{e}"))?;
    if bytes.len() as u64 > MAX_XML_BYTES {
        return Err(format!("PPTX 的 {name} 过大，无法安全抽取"));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("PPTX 的 {name} 不是有效的 UTF-8 XML"))
}

fn drawing_text(xml: &str) -> String {
    let mut paragraphs = Vec::new();
    let mut paragraph = String::new();
    let mut in_paragraph = false;
    let mut cursor = 0usize;

    while let Some(open_relative) = xml[cursor..].find('<') {
        let open = cursor + open_relative;
        let Some(close_relative) = xml[open..].find('>') else {
            break;
        };
        let close = open + close_relative;
        let tag = xml[open + 1..close].trim();
        cursor = close + 1;
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|ch: char| ch.is_whitespace() || ch == '/')
            .next()
            .unwrap_or("");

        match (closing, name) {
            (false, "a:p") if !self_closing => {
                if in_paragraph {
                    finish_paragraph(&mut paragraphs, &mut paragraph);
                }
                in_paragraph = true;
            }
            (true, "a:p") => {
                if in_paragraph {
                    finish_paragraph(&mut paragraphs, &mut paragraph);
                    in_paragraph = false;
                }
            }
            (false, "a:br") if in_paragraph => paragraph.push('\n'),
            (false, "a:t") if in_paragraph && !self_closing => {
                let Some(end_relative) = xml[cursor..].find("</a:t>") else {
                    break;
                };
                paragraph.push_str(&unescape_xml(&xml[cursor..cursor + end_relative]));
                cursor += end_relative + "</a:t>".len();
            }
            _ => {}
        }
    }
    if in_paragraph {
        finish_paragraph(&mut paragraphs, &mut paragraph);
    }
    paragraphs.join("\n")
}

fn finish_paragraph(paragraphs: &mut Vec<String>, paragraph: &mut String) {
    let text = tidy_lines(&std::mem::take(paragraph));
    if !text.is_empty() {
        paragraphs.push(text);
    }
}

// ---------------------------------------------------------------------------
// PDF：复用 filein 的逐页栅格化，再把每页 PNG 交给本机 OCR。
// ---------------------------------------------------------------------------

fn from_pdf(path: &Path, data_dir: &Path) -> Result<ExtractedSource, String> {
    let first = crate::filein::open(path, 1, data_dir)?;
    if first.kind != "pdf" {
        return Err("这个文件没有按 PDF 打开，请检查文件格式".into());
    }
    let pages = first.pages.max(1);
    let ocr_available = crate::ocr::available();
    let mut source = ExtractedSource::default();
    if !ocr_available {
        source
            .warnings
            .push("本机 OCR 当前不可用，无法读取 PDF 页面文字".to_string());
    }

    for page in 1..=pages {
        let later;
        let opened = if page == 1 {
            &first
        } else {
            match crate::filein::open(path, page, data_dir) {
                Ok(file) => {
                    later = file;
                    &later
                }
                Err(error) => {
                    source
                        .warnings
                        .push(format!("第 {page} 页打开失败：{error}"));
                    continue;
                }
            }
        };

        if !ocr_available {
            discard_page_image(opened);
            break;
        }
        match recognize_pdf_page(opened) {
            Ok(text) => append_section(&mut source, format!("第 {page} 页"), text),
            Err(error) => source.warnings.push(format!("第 {page} 页：{error}")),
        }
    }

    if source.text.trim().is_empty() {
        let reason = if source.warnings.is_empty() {
            "页面里没有可识别的文字".to_string()
        } else {
            source.warnings.join("；")
        };
        return Err(format!("这个 PDF 没有抽取到文字：{reason}"));
    }
    Ok(source)
}

fn recognize_pdf_page(opened: &LocalFile) -> Result<String, String> {
    let image = opened
        .image
        .as_ref()
        .ok_or_else(|| "没有可供 OCR 的页面图像".to_string())?;
    let recognized = crate::ocr::recognize(&image.path);
    // filein 为本次抽取生成的 PNG 只服务于 OCR，不作为讲义素材或交付物保留。
    let _ = std::fs::remove_file(&image.path);
    let lines = recognized?;
    let text = tidy_lines(
        &lines
            .iter()
            .map(|line| line.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    );
    if text.is_empty() {
        Err("本机 OCR 没有识别到文字".into())
    } else {
        Ok(text)
    }
}

fn discard_page_image(opened: &LocalFile) {
    if let Some(image) = &opened.image {
        let _ = std::fs::remove_file(&image.path);
    }
}

// ---------------------------------------------------------------------------
// 小型 XML / 文本整理工具；DOCX 仍统一交给 filein，不在此另写一套。
// ---------------------------------------------------------------------------

fn unescape_xml(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let entity = &rest[at + 1..];
        let decoded = entity.find(';').and_then(|end| {
            if end > 16 {
                return None;
            }
            decode_entity(&entity[..end]).map(|ch| (ch, end + 1))
        });
        if let Some((ch, width)) = decoded {
            out.push(ch);
            rest = &entity[width..];
        } else {
            out.push('&');
            rest = &entity;
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = entity.strip_prefix('#')?;
            let code = if let Some(hex) = number.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                number.parse::<u32>().ok()?
            };
            char::from_u32(code)
        }
    }
}

fn tidy_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
            out.push('\n');
        } else {
            blank_run = 0;
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nb-source-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("建测试目录");
        dir
    }

    fn write_pptx(path: &Path, entries: &[(&str, &str)]) {
        let file = std::fs::File::create(path).expect("建 PPTX");
        let mut zip = zip::ZipWriter::new(file);
        for (name, xml) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .expect("加 PPTX 条目");
            zip.write_all(xml.as_bytes()).expect("写 PPTX XML");
        }
        zip.finish().expect("收尾 PPTX");
    }

    #[test]
    fn pptx_reads_numeric_slide_order_paragraphs_notes_and_entities() {
        let dir = temp_dir("pptx");
        let path = dir.join("讲义.pptx");
        write_pptx(
            &path,
            &[
                (
                    "ppt/slides/slide10.xml",
                    r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>第十页</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
                ),
                (
                    "ppt/notesSlides/notesSlide2.xml",
                    r#"<p:notes xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>第二条备注 &quot;保留&quot;</a:t></a:r></a:p></p:notes>"#,
                ),
                (
                    "ppt/slides/slide2.xml",
                    r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>第二页 &amp; </a:t></a:r><a:r><a:t>&lt;重点&gt; &#65; &#x42;</a:t></a:r></a:p><a:p><a:r><a:t>第二段</a:t></a:r><a:br/><a:r><a:t>换行</a:t></a:r></a:p></p:sld>"#,
                ),
                (
                    "ppt/slides/slide1.xml",
                    r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>第一页</a:t></a:r></a:p></p:sld>"#,
                ),
                (
                    "ppt/notesSlides/notesSlide1.xml",
                    r#"<p:notes xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>第一条备注 &apos;好&apos;</a:t></a:r></a:p></p:notes>"#,
                ),
                ("ppt/slides/_rels/slide1.xml.rels", "不应被当成幻灯片"),
            ],
        );

        let source = extract(&path, &dir).expect("抽取 PPTX");
        assert_eq!(
            source.locations,
            vec![
                "幻灯片 1",
                "幻灯片 1 备注",
                "幻灯片 2",
                "幻灯片 2 备注",
                "幻灯片 10",
            ]
        );
        assert!(source.text.contains("第一页"));
        assert!(source.text.contains("第一条备注 '好'"));
        assert!(source.text.contains("第二页 & <重点> A B"));
        assert!(source.text.contains("第二段\n换行"));
        assert!(source.text.contains("第二条备注 \"保留\""));
        assert!(source.text.contains("第十页"));

        let first = source.text.find("【幻灯片 1】").unwrap();
        let second = source.text.find("【幻灯片 2】").unwrap();
        let tenth = source.text.find("【幻灯片 10】").unwrap();
        assert!(first < second && second < tenth, "应按数字而非文件名排序");
        assert_eq!(source.warnings.len(), 1);
        assert!(source.warnings[0].contains("SmartArt"));
        assert!(source.warnings[0].contains("1:1"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pptx_with_no_text_reports_chinese_error() {
        let dir = temp_dir("empty-pptx");
        let path = dir.join("空讲义.pptx");
        write_pptx(
            &path,
            &[(
                "ppt/slides/slide1.xml",
                r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p></a:p></p:sld>"#,
            )],
        );
        let error = extract(&path, &dir).unwrap_err();
        assert!(error.contains("没有抽取到文字"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn xml_entities_and_line_cleanup_are_safe() {
        assert_eq!(
            unescape_xml("a &lt; b &amp;&amp; &#12289; &#x3001; &unknown;"),
            "a < b && 、 、 &unknown;"
        );
        assert_eq!(unescape_xml("&#x110000; &#xD800;"), "&#x110000; &#xD800;");
        assert_eq!(
            tidy_lines(" 第一段 \n\n\n第二段\n第三段 "),
            "第一段\n\n第二段\n第三段"
        );
    }
}
