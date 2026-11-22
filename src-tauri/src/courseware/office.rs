//! 课件的 Office Open XML 导出。
//!
//! 这里直接写 OOXML 包，不依赖 Office、LibreOffice 或网络服务。图片统一在
//! 建包前读取并校验，必需图片缺失时不会留下一个看似成功的空文件。

use super::{HandoutImage, HandoutItem, HandoutSpec, PointSpec, ThinkSpec};
use image::GenericImageView;
use std::io::{Cursor, Write};
use std::path::{Component, Path};
use zip::write::SimpleFileOptions;

const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";
const EMU_PER_PIXEL: u64 = 9_525;
const SLIDE_CX: u64 = 12_192_000;
const SLIDE_CY: u64 = 6_858_000;

/// 导出一份可直接用 Word / WPS 打开的 DOCX 讲义。
pub fn export_docx(spec: &HandoutSpec, assets_root: &Path, target: &Path) -> Result<(), String> {
    let images = load_images(spec, assets_root)?;
    let package = docx_package(spec, &images)?;
    write_package(target, package)
}

/// 导出一份 16:9、可直接用 PowerPoint / WPS 打开的 PPTX 课件。
pub fn export_pptx(spec: &HandoutSpec, assets_root: &Path, target: &Path) -> Result<(), String> {
    let images = load_images(spec, assets_root)?;
    let package = pptx_package(spec, &images)?;
    write_package(target, package)
}

/// 用本机 Chromium 的无头打印生成真实 PDF。返回 false 表示没有可用浏览器，调用方应打开系统打印页。
pub fn export_pdf(html: &Path, target: &Path) -> Result<bool, String> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    for path in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    ] {
        let candidate = std::path::PathBuf::from(path);
        if candidate.is_file() {
            candidates.push(candidate);
        }
    }
    for name in [
        "google-chrome",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
    ] {
        if std::process::Command::new(name)
            .arg("--version")
            .output()
            .is_ok()
        {
            candidates.push(std::path::PathBuf::from(name));
            break;
        }
    }
    let Some(browser) = candidates.first() else {
        return Ok(false);
    };
    let html = html
        .canonicalize()
        .map_err(|e| format!("找不到要打印的课件：{e}"))?;
    let profile = std::env::temp_dir().join(format!("neuralkb-pdf-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&profile).map_err(|e| format!("创建打印临时目录失败：{e}"))?;
    let output = std::process::Command::new(browser)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-pdf-header-footer")
        .arg("--virtual-time-budget=3000")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--print-to-pdf={}", target.display()))
        .arg(format!("file://{}", html.display()))
        .output();
    let _ = std::fs::remove_dir_all(&profile);
    let output = output.map_err(|e| format!("启动浏览器打印失败：{e}"))?;
    if !output.status.success() {
        return Err(format!(
            "浏览器打印 PDF 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if !target.is_file()
        || std::fs::metadata(target)
            .map(|meta| meta.len())
            .unwrap_or(0)
            < 500
    {
        return Err("浏览器没有生成有效的 PDF，请改用课件预览里的系统打印".into());
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// 共用：XML、图片与 ZIP
// ---------------------------------------------------------------------------

fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\u{9}' | '\u{a}' | '\u{d}' => escaped.push(ch),
            '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}' => {
                escaped.push(ch)
            }
            _ => {}
        }
    }
    escaped
}

#[derive(Debug)]
struct EmbeddedImage {
    title: String,
    caption: String,
    media_name: String,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
}

fn image_label(image: &HandoutImage, index: usize) -> String {
    let id = image.id.trim();
    let name = image.name.trim();
    if !id.is_empty() && !name.is_empty() && id != name {
        format!("{id} {name}")
    } else if !id.is_empty() {
        id.to_string()
    } else if !name.is_empty() {
        name.to_string()
    } else {
        format!("图 {}", index + 1)
    }
}

fn load_images(spec: &HandoutSpec, assets_root: &Path) -> Result<Vec<EmbeddedImage>, String> {
    let mut loaded = Vec::new();
    for (index, image) in spec.images.iter().enumerate() {
        let label = image_label(image, index);
        match load_image(image, assets_root, index, label.clone()) {
            Ok(image) => loaded.push(image),
            Err(error) if image.required => {
                return Err(format!("必需图片「{label}」无法导出：{error}"));
            }
            Err(_) => {
                // 可选图片允许在原始文件已移走或格式损坏时跳过。
            }
        }
    }
    Ok(loaded)
}

fn load_image(
    image: &HandoutImage,
    assets_root: &Path,
    index: usize,
    title: String,
) -> Result<EmbeddedImage, String> {
    let raw_path = image.path.trim();
    if raw_path.is_empty() {
        return Err("图片路径为空，请重新选择图片".into());
    }
    let relative = Path::new(raw_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err("图片路径必须是 assets/ 内的相对路径".into());
    }
    let source = assets_root.join(relative);
    let bytes = std::fs::read(&source)
        .map_err(|error| format!("读不到图片 {}：{error}", source.display()))?;
    let format = image::guess_format(&bytes).map_err(|_| "无法识别图片格式".to_string())?;
    let decoded =
        image::load_from_memory(&bytes).map_err(|error| format!("图片解码失败：{error}"))?;
    let (width, height) = decoded.dimensions();
    let (bytes, extension) = match format {
        image::ImageFormat::Png => (bytes, "png"),
        image::ImageFormat::Jpeg => (bytes, "jpeg"),
        image::ImageFormat::Gif | image::ImageFormat::WebP => {
            let mut encoded = Cursor::new(Vec::new());
            decoded
                .write_to(&mut encoded, image::ImageFormat::Png)
                .map_err(|error| format!("图片转 PNG 失败：{error}"))?;
            (encoded.into_inner(), "png")
        }
        _ => return Err("仅支持 PNG、JPEG、GIF、WebP 图片".into()),
    };
    Ok(EmbeddedImage {
        title,
        caption: image.caption.trim().to_string(),
        media_name: format!("image{}.{}", index + 1, extension),
        bytes,
        width,
        height,
    })
}

fn fitted_size(width: u32, height: u32, max_cx: u64, max_cy: u64) -> (u64, u64) {
    let width = u64::from(width.max(1));
    let height = u64::from(height.max(1));
    let native_cx = width.saturating_mul(EMU_PER_PIXEL).max(1);
    let native_cy = height.saturating_mul(EMU_PER_PIXEL).max(1);
    if native_cx <= max_cx && native_cy <= max_cy {
        return (native_cx, native_cy);
    }
    let by_width = (native_cy as u128 * max_cx as u128 / native_cx as u128) as u64;
    if by_width <= max_cy {
        (max_cx, by_width.max(1))
    } else {
        let by_height = (native_cx as u128 * max_cy as u128 / native_cy as u128) as u64;
        (by_height.max(1), max_cy)
    }
}

type Package = Vec<(String, Vec<u8>)>;

fn xml_entry(name: &str, xml: String) -> (String, Vec<u8>) {
    (name.to_string(), xml.into_bytes())
}

fn zip_package(entries: Package) -> Result<Vec<u8>, String> {
    let cursor = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        zip.start_file(name, options)
            .map_err(|error| format!("写入 Office 包失败：{error}"))?;
        zip.write_all(&bytes)
            .map_err(|error| format!("写入 Office 包失败：{error}"))?;
    }
    let cursor = zip
        .finish()
        .map_err(|error| format!("完成 Office 包失败：{error}"))?;
    Ok(cursor.into_inner())
}

fn write_package(target: &Path, entries: Package) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(format!("导出目录不存在：{}", parent.display()));
        }
    }
    let bytes = zip_package(entries)?;
    std::fs::write(target, bytes).map_err(|error| format!("保存导出文件失败：{error}"))
}

// ---------------------------------------------------------------------------
// DOCX
// ---------------------------------------------------------------------------

fn docx_text_run(text: &str, bold: bool) -> String {
    let properties = if bold { "<w:rPr><w:b/></w:rPr>" } else { "" };
    let mut run = format!("<w:r>{properties}");
    for (index, line) in text.replace('\r', "").split('\n').enumerate() {
        if index > 0 {
            run.push_str("<w:br/>");
        }
        run.push_str(&format!(
            "<w:t xml:space=\"preserve\">{}</w:t>",
            xml_escape(line)
        ));
    }
    run.push_str("</w:r>");
    run
}

fn docx_paragraph(text: &str, style: Option<&str>) -> String {
    let properties = style
        .map(|style| format!("<w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr>"))
        .unwrap_or_default();
    format!("<w:p>{properties}{}</w:p>", docx_text_run(text, false))
}

fn docx_labeled_paragraph(label: &str, text: &str) -> String {
    format!(
        "<w:p>{}{}</w:p>",
        docx_text_run(label, true),
        docx_text_run(text, false)
    )
}

fn docx_item_xml(body: &mut String, heading: &str, item: &HandoutItem) {
    body.push_str(&docx_paragraph(heading, Some("Heading2")));
    body.push_str(&docx_paragraph(&item.stem, None));
    for (index, step) in item.steps.iter().enumerate() {
        body.push_str(&docx_paragraph(
            &format!("步骤 {}：{}", index + 1, step),
            Some("ListParagraph"),
        ));
    }
    body.push_str(&docx_labeled_paragraph("答案：", &item.answer));
    body.push_str(&docx_labeled_paragraph("解析：", &item.analysis));
}

fn docx_point_xml(body: &mut String, index: usize, point: &PointSpec) {
    body.push_str(&docx_paragraph(
        &format!("{}. {}", index + 1, point.name),
        Some("Heading2"),
    ));
    if !point.summary.trim().is_empty() {
        body.push_str(&docx_labeled_paragraph("摘要：", &point.summary));
    }
    if !point.detail.trim().is_empty() {
        body.push_str(&docx_labeled_paragraph("讲解：", &point.detail));
    }
    if !point.formula.trim().is_empty() {
        body.push_str(&docx_labeled_paragraph("公式 / 关系：", &point.formula));
    }
    if !point.misconception.trim().is_empty() {
        body.push_str(&docx_labeled_paragraph("易错提醒：", &point.misconception));
    }
}

fn docx_thinking_xml(body: &mut String, index: usize, item: &ThinkSpec) {
    body.push_str(&docx_paragraph(
        &format!("思维训练 {}", index + 1),
        Some("Heading2"),
    ));
    body.push_str(&docx_paragraph(&item.prompt, None));
    if !item.hint.trim().is_empty() {
        body.push_str(&docx_labeled_paragraph("提示：", &item.hint));
    }
    if !item.answer.trim().is_empty() {
        body.push_str(&docx_labeled_paragraph("参考答案：", &item.answer));
    }
}

fn docx_image_xml(image: &EmbeddedImage, index: usize) -> String {
    let (cx, cy) = fitted_size(image.width, image.height, 5_486_400, 5_486_400);
    let drawing_id = index + 1;
    let relationship_id = index + 2;
    let description = if image.caption.is_empty() {
        image.title.as_str()
    } else {
        image.caption.as_str()
    };
    format!(
        "<w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:drawing>\
         <wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
         <wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
         <wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>\
         <wp:docPr id=\"{drawing_id}\" name=\"{}\" descr=\"{}\"/>\
         <wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>\
         <a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
         <pic:pic><pic:nvPicPr><pic:cNvPr id=\"{drawing_id}\" name=\"{}\" descr=\"{}\"/>\
         <pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"rId{relationship_id}\"/>\
         <a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr>\
         <a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>\
         </a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>",
        xml_escape(&image.title),
        xml_escape(description),
        xml_escape(&image.media_name),
        xml_escape(description),
    )
}

fn docx_content_types() -> String {
    format!(
        "{XML_DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Default Extension=\"png\" ContentType=\"image/png\"/>\
         <Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\
         <Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
         <Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
         </Types>"
    )
}

fn docx_styles() -> String {
    format!(
        "{XML_DECL}<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
         <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Aptos\" w:hAnsi=\"Aptos\" w:eastAsia=\"Microsoft YaHei\"/>\
         <w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/></w:rPr></w:rPrDefault>\
         <w:pPrDefault><w:pPr><w:spacing w:after=\"120\" w:line=\"300\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
         <w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/>\
         <w:pPr><w:jc w:val=\"center\"/><w:spacing w:after=\"240\"/></w:pPr><w:rPr><w:b/><w:sz w:val=\"40\"/><w:szCs w:val=\"40\"/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Subtitle\"><w:name w:val=\"Subtitle\"/><w:basedOn w:val=\"Normal\"/>\
         <w:pPr><w:jc w:val=\"center\"/></w:pPr><w:rPr><w:color w:val=\"5B6470\"/><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/>\
         <w:pPr><w:keepNext/><w:spacing w:before=\"360\" w:after=\"160\"/><w:outlineLvl w:val=\"0\"/></w:pPr>\
         <w:rPr><w:b/><w:color w:val=\"26374A\"/><w:sz w:val=\"30\"/><w:szCs w:val=\"30\"/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/><w:basedOn w:val=\"Normal\"/>\
         <w:pPr><w:keepNext/><w:spacing w:before=\"240\" w:after=\"120\"/><w:outlineLvl w:val=\"1\"/></w:pPr>\
         <w:rPr><w:b/><w:color w:val=\"3D536B\"/><w:sz w:val=\"25\"/><w:szCs w:val=\"25\"/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/>\
         <w:pPr><w:ind w:left=\"420\" w:hanging=\"0\"/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Caption\"><w:name w:val=\"caption\"/><w:basedOn w:val=\"Normal\"/>\
         <w:pPr><w:jc w:val=\"center\"/></w:pPr><w:rPr><w:color w:val=\"687482\"/><w:sz w:val=\"19\"/><w:szCs w:val=\"19\"/></w:rPr></w:style>\
         </w:styles>"
    )
}

fn docx_package(spec: &HandoutSpec, images: &[EmbeddedImage]) -> Result<Package, String> {
    let mut body = String::new();
    body.push_str(&docx_paragraph(&spec.title, Some("Title")));
    let metadata = [
        spec.subject.as_str(),
        spec.grade.as_str(),
        spec.chapter.as_str(),
    ]
    .into_iter()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    if !metadata.is_empty() {
        body.push_str(&docx_paragraph(&metadata, Some("Subtitle")));
    }
    if !spec.description.trim().is_empty() {
        body.push_str(&docx_paragraph("课程简介", Some("Heading1")));
        body.push_str(&docx_paragraph(&spec.description, None));
    }
    if !spec.objectives.is_empty() {
        body.push_str(&docx_paragraph("学习目标", Some("Heading1")));
        for (index, objective) in spec.objectives.iter().enumerate() {
            body.push_str(&docx_paragraph(
                &format!("{}. {}", index + 1, objective),
                Some("ListParagraph"),
            ));
        }
    }
    if !spec.warmup.trim().is_empty() {
        body.push_str(&docx_paragraph("课前导入", Some("Heading1")));
        body.push_str(&docx_paragraph(&spec.warmup, None));
    }
    if !spec.knowledge.is_empty() {
        body.push_str(&docx_paragraph("知识清单", Some("Heading1")));
        for knowledge in &spec.knowledge {
            body.push_str(&docx_paragraph(
                &format!("• {knowledge}"),
                Some("ListParagraph"),
            ));
        }
    }
    if !spec.points.is_empty() {
        body.push_str(&docx_paragraph("知识讲解", Some("Heading1")));
        for (index, point) in spec.points.iter().enumerate() {
            docx_point_xml(&mut body, index, point);
        }
    }
    for (heading, items) in [
        ("例题精讲", &spec.examples),
        ("变式练习", &spec.variants),
        ("拔高训练", &spec.challenge),
    ] {
        if items.is_empty() {
            continue;
        }
        body.push_str(&docx_paragraph(heading, Some("Heading1")));
        let item_heading = match heading {
            "例题精讲" => "例题",
            "变式练习" => "变式",
            _ => "拔高",
        };
        for (index, item) in items.iter().enumerate() {
            docx_item_xml(&mut body, &format!("{item_heading} {}", index + 1), item);
        }
    }
    if !spec.thinking.is_empty() {
        body.push_str(&docx_paragraph("思维训练", Some("Heading1")));
        for (index, item) in spec.thinking.iter().enumerate() {
            docx_thinking_xml(&mut body, index, item);
        }
    }
    if !spec.summary.trim().is_empty() {
        body.push_str(&docx_paragraph("课堂小结", Some("Heading1")));
        body.push_str(&docx_paragraph(&spec.summary, None));
    }
    if !spec.homework.is_empty() {
        body.push_str(&docx_paragraph("课后作业", Some("Heading1")));
        for (index, homework) in spec.homework.iter().enumerate() {
            body.push_str(&docx_paragraph(
                &format!("{}. {}", index + 1, homework),
                Some("ListParagraph"),
            ));
        }
    }
    if !images.is_empty() {
        body.push_str(&docx_paragraph("课件插图", Some("Heading1")));
        for (index, image) in images.iter().enumerate() {
            body.push_str(&docx_paragraph(&image.title, Some("Heading2")));
            if !image.caption.is_empty() {
                body.push_str(&docx_paragraph(&image.caption, Some("Caption")));
            }
            body.push_str(&docx_image_xml(image, index));
        }
    }
    body.push_str(
        "<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
         <w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/>\
         </w:sectPr>",
    );
    let document = format!(
        "{XML_DECL}<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"\
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"\
         xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\"\
         xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"\
         xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{body}</w:body></w:document>"
    );
    let root_relationships = format!(
        "{XML_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
         </Relationships>"
    );
    let mut document_relationships = format!(
        "{XML_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>"
    );
    for (index, image) in images.iter().enumerate() {
        document_relationships.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/{}\"/>",
            index + 2,
            image.media_name
        ));
    }
    document_relationships.push_str("</Relationships>");

    let mut package = vec![
        xml_entry("[Content_Types].xml", docx_content_types()),
        xml_entry("_rels/.rels", root_relationships),
        xml_entry("word/document.xml", document),
        xml_entry("word/_rels/document.xml.rels", document_relationships),
        xml_entry("word/styles.xml", docx_styles()),
    ];
    for image in images {
        package.push((
            format!("word/media/{}", image.media_name),
            image.bytes.clone(),
        ));
    }
    Ok(package)
}

// ---------------------------------------------------------------------------
// PPTX
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct PptSlide {
    title: String,
    lines: Vec<String>,
    image: Option<usize>,
}

fn split_long_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }
    let chars = text.chars().collect::<Vec<_>>();
    chars
        .chunks(max_chars)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect()
}

fn push_paginated(slides: &mut Vec<PptSlide>, title: &str, lines: Vec<String>) {
    const MAX_LINES: usize = 7;
    const MAX_CHARS: usize = 640;
    let mut pages: Vec<Vec<String>> = Vec::new();
    let mut current = Vec::new();
    let mut current_chars = 0usize;
    for line in lines
        .into_iter()
        .filter(|line| !line.trim().is_empty())
        .flat_map(|line| split_long_text(&line, 150))
    {
        let line_chars = line.chars().count();
        if !current.is_empty()
            && (current.len() >= MAX_LINES || current_chars + line_chars > MAX_CHARS)
        {
            pages.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current_chars += line_chars;
        current.push(line);
    }
    if !current.is_empty() {
        pages.push(current);
    }
    for (index, lines) in pages.into_iter().enumerate() {
        slides.push(PptSlide {
            title: if index == 0 {
                title.to_string()
            } else {
                format!("{title}（续）")
            },
            lines,
            image: None,
        });
    }
}

fn push_ppt_item(slides: &mut Vec<PptSlide>, title: String, item: &HandoutItem) {
    let mut lines = vec![format!("题目：{}", item.stem)];
    for (index, step) in item.steps.iter().enumerate() {
        lines.push(format!("步骤 {}：{}", index + 1, step));
    }
    lines.push(format!("答案：{}", item.answer));
    lines.push(format!("解析：{}", item.analysis));
    push_paginated(slides, &title, lines);
}

fn ppt_text_runs(text: &str, font_size: u32, bold: bool, color: &str) -> String {
    let bold = if bold { " b=\"1\"" } else { "" };
    let properties = format!("lang=\"zh-CN\" sz=\"{font_size}\"{bold} dirty=\"0\"");
    let mut runs = String::new();
    for (index, line) in text.replace('\r', "").split('\n').enumerate() {
        if index > 0 {
            runs.push_str(&format!("<a:br><a:rPr {properties}/></a:br>"));
        }
        runs.push_str(&format!(
            "<a:r><a:rPr {properties}><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill>\
             <a:latin typeface=\"Aptos\"/><a:ea typeface=\"Microsoft YaHei\"/></a:rPr><a:t>{}</a:t></a:r>",
            xml_escape(line)
        ));
    }
    runs
}

#[allow(clippy::too_many_arguments)]
fn ppt_text_shape(
    id: usize,
    name: &str,
    x: u64,
    y: u64,
    cx: u64,
    cy: u64,
    paragraphs: &[String],
    font_size: u32,
    bold: bool,
    color: &str,
    align: &str,
) -> String {
    let mut body = String::new();
    for paragraph in paragraphs {
        body.push_str(&format!(
            "<a:p><a:pPr algn=\"{align}\"/>{}<a:endParaRPr lang=\"zh-CN\" sz=\"{font_size}\"/></a:p>",
            ppt_text_runs(paragraph, font_size, bold, color)
        ));
    }
    if paragraphs.is_empty() {
        body.push_str("<a:p><a:endParaRPr lang=\"zh-CN\"/></a:p>");
    }
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr>\
         <p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr>\
         <p:txBody><a:bodyPr wrap=\"square\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\"><a:normAutofit/></a:bodyPr>\
         <a:lstStyle/>{body}</p:txBody></p:sp>",
        xml_escape(name)
    )
}

fn ppt_image_xml(image: &EmbeddedImage, relationship_id: usize) -> String {
    const IMAGE_Y: u64 = 1_250_000;
    const IMAGE_MAX_CY: u64 = 4_400_000;
    let (cx, cy) = fitted_size(image.width, image.height, 9_700_000, IMAGE_MAX_CY);
    let x = (SLIDE_CX - cx) / 2;
    let y = IMAGE_Y + (IMAGE_MAX_CY - cy) / 2;
    let description = if image.caption.is_empty() {
        image.title.as_str()
    } else {
        image.caption.as_str()
    };
    format!(
        "<p:pic><p:nvPicPr><p:cNvPr id=\"4\" name=\"{}\" descr=\"{}\"/>\
         <p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr>\
         <p:blipFill><a:blip r:embed=\"rId{relationship_id}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
         <p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
        xml_escape(&image.media_name),
        xml_escape(description)
    )
}

fn ppt_slide_xml(slide: &PptSlide, index: usize, images: &[EmbeddedImage]) -> String {
    let mut shapes = String::new();
    if index == 0 {
        shapes.push_str(&ppt_text_shape(
            2,
            "课件标题",
            750_000,
            1_650_000,
            10_692_000,
            1_500_000,
            &[slide.title.clone()],
            4_000,
            true,
            "26374A",
            "ctr",
        ));
        shapes.push_str(&ppt_text_shape(
            3,
            "课程信息",
            1_150_000,
            3_450_000,
            9_892_000,
            1_250_000,
            &slide.lines,
            2_200,
            false,
            "5B6470",
            "ctr",
        ));
    } else if let Some(image_index) = slide.image {
        shapes.push_str(&ppt_text_shape(
            2,
            "插图标题",
            600_000,
            330_000,
            10_992_000,
            650_000,
            &[slide.title.clone()],
            2_800,
            true,
            "26374A",
            "l",
        ));
        shapes.push_str(&ppt_image_xml(&images[image_index], 2));
        shapes.push_str(&ppt_text_shape(
            3,
            "图片说明",
            850_000,
            5_800_000,
            10_492_000,
            650_000,
            &slide.lines,
            1_600,
            false,
            "5B6470",
            "ctr",
        ));
    } else {
        shapes.push_str(&ppt_text_shape(
            2,
            "页面标题",
            600_000,
            330_000,
            10_992_000,
            700_000,
            &[slide.title.clone()],
            3_000,
            true,
            "26374A",
            "l",
        ));
        let font_size = if slide.lines.len() > 5 { 1_800 } else { 2_000 };
        shapes.push_str(&ppt_text_shape(
            3,
            "正文",
            800_000,
            1_300_000,
            10_592_000,
            5_050_000,
            &slide.lines,
            font_size,
            false,
            "26374A",
            "l",
        ));
    }
    format!(
        "{XML_DECL}<p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"\
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"\
         xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:spTree>\
         <p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
         <p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/>\
         <a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>{shapes}</p:spTree></p:cSld>\
         <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    )
}

fn ppt_group_tree() -> &'static str {
    "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
     <p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/>\
     <a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>"
}

fn ppt_master() -> String {
    format!(
        "{XML_DECL}<p:sldMaster xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"\
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"\
         xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:bg><p:bgPr>\
         <a:solidFill><a:srgbClr val=\"F4F6F8\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{}</p:spTree></p:cSld>\
         <p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
         <p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst>\
         <p:txStyles><p:titleStyle><a:lvl1pPr algn=\"l\"><a:defRPr sz=\"3200\" b=\"1\"/></a:lvl1pPr></p:titleStyle>\
         <p:bodyStyle><a:lvl1pPr><a:defRPr sz=\"2000\"/></a:lvl1pPr></p:bodyStyle>\
         <p:otherStyle><a:lvl1pPr><a:defRPr sz=\"1800\"/></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>",
        ppt_group_tree()
    )
}

fn ppt_layout() -> String {
    format!(
        "{XML_DECL}<p:sldLayout xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"\
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"\
         xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" type=\"blank\" preserve=\"1\">\
         <p:cSld name=\"空白\"><p:spTree>{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>",
        ppt_group_tree()
    )
}

fn ppt_theme() -> String {
    format!(
        "{XML_DECL}<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"NeuralKB\">\
         <a:themeElements><a:clrScheme name=\"NeuralKB\"><a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1>\
         <a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1><a:dk2><a:srgbClr val=\"26374A\"/></a:dk2>\
         <a:lt2><a:srgbClr val=\"F4F6F8\"/></a:lt2><a:accent1><a:srgbClr val=\"527A9E\"/></a:accent1>\
         <a:accent2><a:srgbClr val=\"7C96AB\"/></a:accent2><a:accent3><a:srgbClr val=\"8CA68F\"/></a:accent3>\
         <a:accent4><a:srgbClr val=\"B69B72\"/></a:accent4><a:accent5><a:srgbClr val=\"7D8BA3\"/></a:accent5>\
         <a:accent6><a:srgbClr val=\"9B879F\"/></a:accent6><a:hlink><a:srgbClr val=\"356A96\"/></a:hlink>\
         <a:folHlink><a:srgbClr val=\"765B83\"/></a:folHlink></a:clrScheme>\
         <a:fontScheme name=\"NeuralKB\"><a:majorFont><a:latin typeface=\"Aptos Display\"/><a:ea typeface=\"Microsoft YaHei\"/><a:cs typeface=\"\"/></a:majorFont>\
         <a:minorFont><a:latin typeface=\"Aptos\"/><a:ea typeface=\"Microsoft YaHei\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme>\
         <a:fmtScheme name=\"NeuralKB\"><a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
         <a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst>\
         <a:lnStyleLst><a:ln w=\"9525\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln>\
         <a:ln w=\"19050\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln>\
         <a:ln w=\"28575\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln></a:lnStyleLst>\
         <a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle>\
         <a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst>\
         <a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
         <a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements>\
         <a:objectDefaults/><a:extraClrSchemeLst/></a:theme>"
    )
}

fn ppt_content_types(slide_count: usize) -> String {
    let mut overrides = String::new();
    for index in 0..slide_count {
        overrides.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>",
            index + 1
        ));
    }
    format!(
        "{XML_DECL}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/>\
         <Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\
         <Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
         <Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
         <Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
         <Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>{overrides}</Types>"
    )
}

fn pptx_package(spec: &HandoutSpec, images: &[EmbeddedImage]) -> Result<Package, String> {
    let metadata = [
        spec.subject.as_str(),
        spec.grade.as_str(),
        spec.chapter.as_str(),
    ]
    .into_iter()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    let mut slides = vec![PptSlide {
        title: spec.title.clone(),
        lines: if metadata.is_empty() {
            vec![]
        } else {
            vec![metadata]
        },
        image: None,
    }];
    if !spec.description.trim().is_empty() {
        push_paginated(&mut slides, "课程简介", vec![spec.description.clone()]);
    }
    if !spec.objectives.is_empty() {
        push_paginated(
            &mut slides,
            "学习目标",
            spec.objectives
                .iter()
                .enumerate()
                .map(|(index, value)| format!("{}. {value}", index + 1))
                .collect(),
        );
    }
    if !spec.warmup.trim().is_empty() {
        push_paginated(&mut slides, "课前导入", vec![spec.warmup.clone()]);
    }
    if !spec.knowledge.is_empty() {
        push_paginated(
            &mut slides,
            "知识清单",
            spec.knowledge
                .iter()
                .map(|value| format!("• {value}"))
                .collect(),
        );
    }
    for (index, point) in spec.points.iter().enumerate() {
        let mut lines = Vec::new();
        if !point.summary.trim().is_empty() {
            lines.push(format!("摘要：{}", point.summary));
        }
        if !point.detail.trim().is_empty() {
            lines.push(format!("讲解：{}", point.detail));
        }
        if !point.formula.trim().is_empty() {
            lines.push(format!("公式 / 关系：{}", point.formula));
        }
        if !point.misconception.trim().is_empty() {
            lines.push(format!("易错提醒：{}", point.misconception));
        }
        push_paginated(
            &mut slides,
            &format!("知识讲解 {} · {}", index + 1, point.name),
            lines,
        );
    }
    for (index, item) in spec.examples.iter().enumerate() {
        push_ppt_item(&mut slides, format!("例题 {}", index + 1), item);
    }
    for (index, item) in spec.variants.iter().enumerate() {
        push_ppt_item(&mut slides, format!("变式练习 {}", index + 1), item);
    }
    for (index, item) in spec.challenge.iter().enumerate() {
        push_ppt_item(&mut slides, format!("拔高训练 {}", index + 1), item);
    }
    for (index, item) in spec.thinking.iter().enumerate() {
        let mut lines = vec![format!("题目：{}", item.prompt)];
        if !item.hint.trim().is_empty() {
            lines.push(format!("提示：{}", item.hint));
        }
        if !item.answer.trim().is_empty() {
            lines.push(format!("参考答案：{}", item.answer));
        }
        push_paginated(&mut slides, &format!("思维训练 {}", index + 1), lines);
    }
    for (index, image) in images.iter().enumerate() {
        slides.push(PptSlide {
            title: format!("课件插图 · {}", image.title),
            lines: if image.caption.is_empty() {
                vec![]
            } else {
                split_long_text(&image.caption, 150)
            },
            image: Some(index),
        });
    }
    if !spec.summary.trim().is_empty() || !spec.homework.is_empty() {
        let mut lines = Vec::new();
        if !spec.summary.trim().is_empty() {
            lines.push(format!("课堂小结：{}", spec.summary));
        }
        if !spec.homework.is_empty() {
            lines.push("课后作业：".into());
            for (index, homework) in spec.homework.iter().enumerate() {
                lines.push(format!("{}. {homework}", index + 1));
            }
        }
        push_paginated(&mut slides, "总结与作业", lines);
    }

    let slide_count = slides.len();
    let mut presentation_ids = String::new();
    let mut presentation_relationships = format!(
        "{XML_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>"
    );
    for index in 0..slide_count {
        presentation_ids.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            256 + index,
            index + 2
        ));
        presentation_relationships.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{}.xml\"/>",
            index + 2,
            index + 1
        ));
    }
    presentation_relationships.push_str("</Relationships>");
    let presentation = format!(
        "{XML_DECL}<p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"\
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"\
         xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
         <p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>\
         <p:sldIdLst>{presentation_ids}</p:sldIdLst><p:sldSz cx=\"{SLIDE_CX}\" cy=\"{SLIDE_CY}\" type=\"screen16x9\"/>\
         <p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
    );
    let root_relationships = format!(
        "{XML_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/>\
         </Relationships>"
    );
    let master_relationships = format!(
        "{XML_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
         <Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/>\
         </Relationships>"
    );
    let layout_relationships = format!(
        "{XML_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/>\
         </Relationships>"
    );

    let mut package = vec![
        xml_entry("[Content_Types].xml", ppt_content_types(slide_count)),
        xml_entry("_rels/.rels", root_relationships),
        xml_entry("ppt/presentation.xml", presentation),
        xml_entry(
            "ppt/_rels/presentation.xml.rels",
            presentation_relationships,
        ),
        xml_entry("ppt/slideMasters/slideMaster1.xml", ppt_master()),
        xml_entry(
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            master_relationships,
        ),
        xml_entry("ppt/slideLayouts/slideLayout1.xml", ppt_layout()),
        xml_entry(
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            layout_relationships,
        ),
        xml_entry("ppt/theme/theme1.xml", ppt_theme()),
    ];
    for (index, slide) in slides.iter().enumerate() {
        package.push(xml_entry(
            &format!("ppt/slides/slide{}.xml", index + 1),
            ppt_slide_xml(slide, index, images),
        ));
        let mut relationships = format!(
            "{XML_DECL}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
             <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>"
        );
        if let Some(image_index) = slide.image {
            relationships.push_str(&format!(
                "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/{}\"/>",
                images[image_index].media_name
            ));
        }
        relationships.push_str("</Relationships>");
        package.push(xml_entry(
            &format!("ppt/slides/_rels/slide{}.xml.rels", index + 1),
            relationships,
        ));
    }
    for image in images {
        package.push((
            format!("ppt/media/{}", image.media_name),
            image.bytes.clone(),
        ));
    }
    Ok(package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    struct TestDir(PathBuf);

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    use std::path::PathBuf;

    fn fresh_dir(label: &str) -> TestDir {
        let serial = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("nb-office-{label}-{}-{serial}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets/courseware")).unwrap();
        TestDir(dir)
    }

    fn encoded_image(format: image::ImageFormat, color: [u8; 3]) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(8, 5, image::Rgb(color));
        let mut output = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut output, format)
            .unwrap();
        output.into_inner()
    }

    fn sample_spec() -> HandoutSpec {
        HandoutSpec {
            title: "几何 <A&B> \"导出\"".into(),
            subject: "初中数学".into(),
            grade: "八年级".into(),
            chapter: "三角形 <全等>".into(),
            description: "用观察 & 推理认识图形。".into(),
            knowledge: vec!["全等三角形".into(), "SAS & ASA".into()],
            objectives: vec!["识别对应边与对应角".into(), "能写出规范证明".into()],
            warmup: "两个图形怎样才算完全重合？".into(),
            points: vec![PointSpec {
                name: "全等的性质".into(),
                summary: "对应边相等，对应角相等。".into(),
                detail: "先确定对应顶点，再找对应边。".into(),
                misconception: "把位置相同误当对应关系。".into(),
                formula: "△ABC ≌ △DEF".into(),
            }],
            examples: vec![HandoutItem {
                stem: "已知 AB = DE，∠A = ∠D，还需什么条件？".into(),
                answer: "AC = DF，可用 SAS。".into(),
                analysis: "两边及其夹角对应相等。".into(),
                steps: vec!["找夹角".into(), "补另一边".into()],
                ..Default::default()
            }],
            variants: vec![HandoutItem {
                stem: "把 SAS 换成 ASA，条件怎样改？".into(),
                answer: "两角及其夹边对应相等。".into(),
                analysis: "注意夹边必须是两角公共边。".into(),
                ..Default::default()
            }],
            challenge: vec![],
            thinking: vec![ThinkSpec {
                prompt: "SSA 能判定全等吗？".into(),
                hint: "尝试画两个不同的三角形。".into(),
                answer: "一般不能。".into(),
            }],
            summary: "对应关系是证明全等的起点。".into(),
            homework: vec!["完成教材练习 1 & 2".into()],
            theme: "paper".into(),
            ..Default::default()
        }
    }

    fn attach_png_and_jpeg(spec: &mut HandoutSpec, root: &Path) -> (Vec<u8>, Vec<u8>) {
        let png = encoded_image(image::ImageFormat::Png, [30, 120, 200]);
        let jpeg = encoded_image(image::ImageFormat::Jpeg, [200, 100, 30]);
        std::fs::write(root.join("assets/courseware/diagram.png"), &png).unwrap();
        std::fs::write(root.join("assets/courseware/photo.jpg"), &jpeg).unwrap();
        spec.images = vec![
            HandoutImage {
                id: "图1".into(),
                path: "courseware/diagram.png".into(),
                name: "对应顶点".into(),
                caption: "观察 <对应边> & 对应角".into(),
                required: true,
                ..Default::default()
            },
            HandoutImage {
                id: "图2".into(),
                path: "courseware/photo.jpg".into(),
                name: "实例".into(),
                required: true,
                ..Default::default()
            },
        ];
        (png, jpeg)
    }

    fn archive(path: &Path) -> zip::ZipArchive<std::fs::File> {
        zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap()
    }

    fn text_entry(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> String {
        let mut entry = archive.by_name(name).unwrap();
        let mut text = String::new();
        entry.read_to_string(&mut text).unwrap();
        text
    }

    fn bytes_entry(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Vec<u8> {
        let mut entry = archive.by_name(name).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn docx_is_real_ooxml_with_escaped_text_and_original_images() {
        let dir = fresh_dir("docx");
        let assets = dir.0.join("assets");
        let target = dir.0.join("讲义.docx");
        let mut spec = sample_spec();
        let (png, jpeg) = attach_png_and_jpeg(&mut spec, &dir.0);
        export_docx(&spec, &assets, &target).unwrap();

        let mut archive = archive(&target);
        for required in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/_rels/document.xml.rels",
            "word/styles.xml",
        ] {
            archive.by_name(required).unwrap();
        }
        let document = text_entry(&mut archive, "word/document.xml");
        assert!(document.contains("几何 &lt;A&amp;B&gt; &quot;导出&quot;"));
        assert!(!document.contains("几何 <A&B>"));
        assert!(document.contains("w:pStyle w:val=\"Heading1\""));
        assert!(document.contains("答案："));
        assert!(document.contains("两边及其夹角对应相等。"));
        assert!(document.contains("观察 &lt;对应边&gt; &amp; 对应角"));
        let relationships = text_entry(&mut archive, "word/_rels/document.xml.rels");
        assert!(relationships.contains("Target=\"media/image1.png\""));
        assert!(relationships.contains("Target=\"media/image2.jpeg\""));
        assert_eq!(bytes_entry(&mut archive, "word/media/image1.png"), png);
        assert_eq!(bytes_entry(&mut archive, "word/media/image2.jpeg"), jpeg);
        let content_types = text_entry(&mut archive, "[Content_Types].xml");
        assert!(content_types.contains("Extension=\"png\" ContentType=\"image/png\""));
        assert!(content_types.contains("Extension=\"jpeg\" ContentType=\"image/jpeg\""));
    }

    #[test]
    fn pptx_is_16_by_9_ooxml_with_slide_rels_and_original_images() {
        let dir = fresh_dir("pptx");
        let assets = dir.0.join("assets");
        let target = dir.0.join("课件.pptx");
        let mut spec = sample_spec();
        let (png, jpeg) = attach_png_and_jpeg(&mut spec, &dir.0);
        export_pptx(&spec, &assets, &target).unwrap();

        let mut archive = archive(&target);
        for required in [
            "[Content_Types].xml",
            "_rels/.rels",
            "ppt/presentation.xml",
            "ppt/_rels/presentation.xml.rels",
            "ppt/slideMasters/slideMaster1.xml",
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            "ppt/slideLayouts/slideLayout1.xml",
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            "ppt/theme/theme1.xml",
            "ppt/slides/slide1.xml",
            "ppt/slides/_rels/slide1.xml.rels",
        ] {
            archive.by_name(required).unwrap();
        }
        let presentation = text_entry(&mut archive, "ppt/presentation.xml");
        assert!(presentation.contains("cx=\"12192000\" cy=\"6858000\""));
        assert!(presentation.contains("type=\"screen16x9\""));
        let cover = text_entry(&mut archive, "ppt/slides/slide1.xml");
        assert!(cover.contains("几何 &lt;A&amp;B&gt; &quot;导出&quot;"));
        assert!(!cover.contains("几何 <A&B>"));
        assert_eq!(bytes_entry(&mut archive, "ppt/media/image1.png"), png);
        assert_eq!(bytes_entry(&mut archive, "ppt/media/image2.jpeg"), jpeg);

        let names = archive.file_names().map(str::to_string).collect::<Vec<_>>();
        let mut all_slides = String::new();
        let mut image_relationships = 0;
        for name in names {
            if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") {
                all_slides.push_str(&text_entry(&mut archive, &name));
            }
            if name.starts_with("ppt/slides/_rels/") {
                let rels = text_entry(&mut archive, &name);
                if rels.contains("relationships/image") {
                    image_relationships += 1;
                    assert!(rels.contains("../media/image"));
                }
            }
        }
        assert!(all_slides.contains("学习目标"));
        assert!(all_slides.contains("例题 1"));
        assert!(all_slides.contains("变式练习 1"));
        assert!(all_slides.contains("总结与作业"));
        assert!(all_slides.contains("答案："));
        assert_eq!(image_relationships, 2, "两张图各有一个真实的幻灯片关系");
    }

    #[test]
    fn required_missing_image_errors_but_optional_one_is_skipped() {
        let dir = fresh_dir("missing");
        let assets = dir.0.join("assets");
        let mut spec = sample_spec();
        spec.images = vec![HandoutImage {
            id: "必须有的图".into(),
            path: "courseware/missing.png".into(),
            required: true,
            ..Default::default()
        }];
        let docx = dir.0.join("missing.docx");
        let error = export_docx(&spec, &assets, &docx).unwrap_err();
        assert!(
            error.contains("必需图片") && error.contains("必须有的图"),
            "{error}"
        );
        assert!(!docx.exists());
        let pptx = dir.0.join("missing.pptx");
        let error = export_pptx(&spec, &assets, &pptx).unwrap_err();
        assert!(error.contains("必需图片"), "{error}");
        assert!(!pptx.exists());

        spec.images[0].required = false;
        export_docx(&spec, &assets, &docx).unwrap();
        let mut archive = archive(&docx);
        assert!(archive.by_name("word/media/image1.png").is_err());
    }

    #[test]
    fn gif_is_converted_to_embedded_png() {
        let dir = fresh_dir("gif");
        let assets = dir.0.join("assets");
        let gif = encoded_image(image::ImageFormat::Gif, [10, 160, 80]);
        std::fs::write(dir.0.join("assets/courseware/animated.gif"), gif).unwrap();
        let mut spec = sample_spec();
        spec.images = vec![HandoutImage {
            id: "动图".into(),
            path: "courseware/animated.gif".into(),
            required: true,
            ..Default::default()
        }];
        let target = dir.0.join("gif.docx");
        export_docx(&spec, &assets, &target).unwrap();
        let mut archive = archive(&target);
        let bytes = bytes_entry(&mut archive, "word/media/image1.png");
        assert_eq!(bytes.get(..8), Some(&b"\x89PNG\r\n\x1a\n"[..]));
    }
}
