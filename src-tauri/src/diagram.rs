//! AI 制图：用配置好的模型把题干画成一张内嵌 SVG 示意图。
//!
//! 本地客户端离线运行，不打包 GeoGebra 引擎，也不联网取图，所以制图链路收敛成：
//! 模型产出图形描述 → 本地渲染成 SVG → 前端预览 → 走既有 asset_add 存成题目配图。
//! 这样图形随题目一起进 question_asset，导出 / 备份 / 复制题目都带着走。
//!
//! 模型这一步分两种形态，优先用第一种：
//! 1) **场景 JSON**（points / segments / faces / curves）：模型只给坐标和连法，
//!    SVG 由 `diagram_scene` 自己画。几何一致性、标签摆放、虚线样式都由本地保证，
//!    模型漏画棱或给不存在点标号也能当场发现并让它重画。
//! 2) 老的整张 SVG：模型不听话时还会走这条路，先做净化、截断补齐再用。
//!
//! 整轮失败会带着错误原因重画一次——比原样再问一遍有效得多。

use crate::diagram_scene;
use crate::llm::{self, prompts};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

/// 制图要吐一整张 SVG，4096 在复杂立体几何 / 带坐标轴的函数图上经常不够，
/// 截断比画错更常见，所以单独给一个更宽的预算。
const MAX_TOKENS: u32 = 6144;

/// 制图模式：影响提示词的作图重点，但产物统一是 SVG。
/// general = 各学科通用示意图；math = 强调坐标系 / 几何关系的精确图形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    General,
    Math,
}

impl Mode {
    fn parse(value: &str) -> Mode {
        if value == "math" {
            Mode::Math
        } else {
            Mode::General
        }
    }

    fn label(self) -> &'static str {
        match self {
            Mode::General => "通用示意图：按题意画出能看懂的图形，标号清楚",
            Mode::Math => "数学精确图形：画出坐标系/几何关系，关键点标注准确、比例合理",
        }
    }

    fn code(self) -> &'static str {
        match self {
            Mode::General => "general",
            Mode::Math => "math",
        }
    }
}

/// 调用模型生成 SVG。返回 {svg, mode}；svg 已通过安全清理，可直接放进 data URL 渲染。
pub fn generate(
    conn: &rusqlite::Connection,
    http: &reqwest::blocking::Client,
    subject: &str,
    stem: &str,
    focus: &str,
    mode_value: &str,
) -> Result<Value, String> {
    let stem = stem.trim();
    if stem.is_empty() {
        return Err("先写清楚题干，才能据此作图".into());
    }
    let mode = Mode::parse(mode_value);

    // 两种模式走各自的模型用途路由：通用示意图 / 数学精确图形。
    // 两者产物都是内嵌 SVG，只是提示词重点不同；用途分流让用户能为数学图形
    // 单独排一个更擅长精确绘图的模型，也复用了中心站既有的 geogebra 用途。
    let purpose = if mode == Mode::Math { "geogebra" } else { "diagram" };
    let mut cooldowns: HashMap<String, Instant> = HashMap::new();

    let mut last_error = String::new();
    for attempt in 0..2 {
        let messages = if attempt == 0 {
            prompts::diagram(subject, stem, focus.trim(), mode.label())
        } else {
            prompts::diagram_retry(subject, stem, focus.trim(), mode.label(), &last_error)
        };
        let raw = llm::complete(
            conn,
            http,
            purpose,
            &messages,
            None,
            true,
            MAX_TOKENS,
            &mut cooldowns,
        )?;
        match extract_figure(&raw) {
            Ok(svg) => {
                return Ok(json!({
                    "svg": svg,
                    "mode": mode.code(),
                }))
            }
            Err(error) => last_error = error,
        }
    }

    Err(format!(
        "这次没能画出可用的图形：{last_error}。可以点「重新生成」再试一次；如果连续失败，把题干里的点、线关系和关键位置补进「作图重点」会更稳。"
    ))
}

/// 从模型输出里取出图形：优先按场景 JSON 本地渲染，其次才是模型直接给的 SVG。
///
/// 真实形态有四种，都要接住：
/// - 标准场景 `{"scene":{"points":[...],"segments":[...]}}`（新提示词的产物）；
/// - 模型把场景直接写在顶层（没有 scene 外壳）；
/// - 老形态 `{"svg":"<svg ...></svg>"}`，或无视 JSON 直接吐 `<svg ...>`；
/// - JSON 因为引号没转义 / 被截断解析不了，但里面的 SVG 文本还在。
fn extract_figure(raw: &str) -> Result<String, String> {
    if raw.trim().is_empty() {
        return Err("模型返回了空内容".into());
    }

    let mut scene_error = String::new();
    if let Ok(value) = llm::parse_json(raw, "制图模型") {
        if let Some(scene) = scene_of(&value) {
            match diagram_scene::render(scene) {
                Ok(svg) => return Ok(svg),
                // 场景画不出来（引用了不存在的点等）先记下来：如果同时给了 SVG，
                // 那张图仍然可用，不该让用户白等一轮。
                Err(error) => scene_error = error,
            }
        }
        for key in ["svg", "SVG", "code", "diagram", "content"] {
            if let Some(node) = value.get(key).and_then(|node| node.as_str()) {
                if let Ok(clean) = sanitize_svg(node) {
                    return Ok(clean);
                }
            }
        }
    }

    if let Ok(clean) = salvage_svg(raw) {
        return Ok(clean);
    }

    if scene_error.is_empty() {
        Err(format!(
            "模型没有给出图形内容，它说的是「{}」",
            llm::json::snippet(raw)
        ))
    } else {
        Err(scene_error)
    }
}

/// 认两种场景写法：`{"scene": {...}}` 与直接把 points 放顶层的 `{...}`。
fn scene_of(value: &Value) -> Option<&Value> {
    if let Some(scene) = value.get("scene") {
        if scene.is_object() {
            return Some(scene);
        }
    }
    if value
        .get("points")
        .map(|node| node.is_array())
        .unwrap_or(false)
    {
        return Some(value);
    }
    None
}

/// 从原文里抠出 SVG 文本（模型无视 JSON 要求或 JSON 坏掉时用）。
fn salvage_svg(raw: &str) -> Result<String, String> {
    let head = raw.trim_start();
    if !head.starts_with('{') && !head.starts_with('[') {
        return sanitize_svg(raw);
    }
    let start = find_ci(raw, "<svg").ok_or_else(|| "原文里没有 <svg>".to_string())?;
    sanitize_svg(&unescape_loose(&raw[start..]))
}

/// 把 JSON 字符串里的转义还原成 SVG 文本。
/// 只处理结构上会破坏标签的那几个（\" \\ \n \t），`\sqrt` 这类 LaTeX 命令原样保留。
fn unescape_loose(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('u') => out.push_str("\\u"),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => {}
        }
    }
    out
}

/// 从模型输出里挑出 <svg>...</svg> 片段并做安全清理。
///
/// 模型有时会在 JSON 字符串外带一点说明，或给出多个片段；只取第一个 svg。
/// SVG 会以内联 data URL 显示在应用 WebView 里，所以必须去掉脚本、事件处理器、
/// 外链资源，避免一个坏图形变成注入入口。
///
/// 被截断的图形（没有 `</svg>`）不再直接判失败：补上没闭合的标签，
/// 用户拿到的至少是一张能看的图，可以手动改描述重画。
pub fn sanitize_svg(raw: &str) -> Result<String, String> {
    let start = find_ci(raw, "<svg").ok_or_else(|| "模型没有返回 SVG 图形".to_string())?;
    let fragment = &raw[start..];
    let fragment = match find_ci(fragment, "</svg>") {
        Some(end) => &fragment[..end + "</svg>".len()],
        None => fragment,
    };
    if fragment.len() > 200_000 {
        return Err("SVG 图形过大，请把题干再简化一点".into());
    }

    // 大小写不敏感地剔除危险内容与外链。逐字符扫描，避免正则依赖。
    let mut out = String::with_capacity(fragment.len() + 64);
    let bytes = fragment.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // 注释整段丢掉（模型偶尔在里面留说明）。
        if bytes[i..].starts_with(b"<!--") {
            if let Some(close) = fragment[i + 4..].find("-->") {
                i += 4 + close + 3;
                continue;
            }
        }
        // <script>...</script> 整段丢掉。
        if bytes[i..].len() >= 7 && bytes[i..i + 7].eq_ignore_ascii_case(b"<script") {
            if let Some(close) = find_ci(&fragment[i..], "</script>") {
                i += close + "</script>".len();
                continue;
            }
        }
        // 标签属性里的 on* 事件、href/xlink:href 外链：跳过整个属性值。
        if let Some((name_len, is_attr)) = dangerous_attr(&fragment[i..]) {
            i += name_len;
            if is_attr {
                // 跳到属性结束的引号或 > / 空白之后。
                i = skip_attr_value(fragment, i);
            }
            continue;
        }
        let ch = fragment[i..].chars().next().unwrap();
        // 正文里的裸 & 会让 XML 解析失败、图片整张空白，补成实体。
        if ch == '&' && !entity_at(fragment, i) {
            out.push_str("&amp;");
            i += 1;
            continue;
        }
        out.push(ch);
        i += ch.len_utf8();
    }

    if find_ci(&out, "<svg").is_none() {
        return Err("模型没有返回 SVG 图形".into());
    }
    let out = close_open_tags(strip_incomplete_tail(&out));
    Ok(out)
}

/// 大小写不敏感的字节查找，返回字节下标（`<svg` 全是 ASCII，不会切坏多字节字符）。
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let hay = haystack.as_bytes();
    let pin = needle.as_bytes();
    if pin.is_empty() || hay.len() < pin.len() {
        return None;
    }
    (0..=hay.len() - pin.len()).find(|&i| hay[i..i + pin.len()].eq_ignore_ascii_case(pin))
}

/// `&` 后面是否是一个合法实体（`&amp;` `&#123;` `&#x1F;`），是就别再转义。
fn entity_at(text: &str, at: usize) -> bool {
    let rest = &text[at + 1..];
    let mut chars = rest.char_indices();
    match chars.next() {
        Some((_, '#')) => {
            let mut hex = false;
            let mut count = 0;
            for (index, ch) in chars {
                if index == 0 && (ch == 'x' || ch == 'X') {
                    hex = true;
                    continue;
                }
                if ch == ';' {
                    return count > 0;
                }
                let ok = if hex {
                    ch.is_ascii_hexdigit()
                } else {
                    ch.is_ascii_digit()
                };
                if !ok || count >= 8 {
                    return false;
                }
                count += 1;
            }
            false
        }
        Some((_, ch)) if ch.is_ascii_alphabetic() => {
            let mut count = 0;
            for (_, ch) in chars {
                if ch == ';' {
                    return count >= 1;
                }
                if !ch.is_ascii_alphanumeric() || count >= 31 {
                    return false;
                }
                count += 1;
            }
            false
        }
        _ => false,
    }
}

/// 截断的图形末尾常常挂着半个标签（`<text x="12`），它比缺闭标签更破坏解析：
/// 直接砍到最后一个完整标签之后。
fn strip_incomplete_tail(svg: &str) -> String {
    if let Some(open) = svg.rfind('<') {
        if !svg[open..].contains('>') {
            return svg[..open].to_string();
        }
    }
    svg.to_string()
}

/// 补齐没有闭合的标签，让截断的 SVG 仍然能被浏览器渲染。
fn close_open_tags(svg: String) -> String {
    let mut stack: Vec<String> = Vec::new();
    let bytes = svg.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let Some(rel) = svg[i..].find('>') else {
            break;
        };
        let end = i + rel;
        let tag = svg[i + 1..end].trim();
        if tag.starts_with('!') || tag.starts_with('?') {
            i = end + 1;
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            if let Some(pos) = stack.iter().rposition(|item| item.eq_ignore_ascii_case(name)) {
                stack.truncate(pos);
            }
            i = end + 1;
            continue;
        }
        let self_closing = tag.ends_with('/');
        let name: String = tag
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-'))
            .collect();
        if !name.is_empty() && !self_closing {
            stack.push(name);
        }
        i = end + 1;
    }
    if stack.is_empty() {
        return svg;
    }
    let mut out = svg;
    for name in stack.iter().rev() {
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    }
    out
}

/// 识别当前位置是否为危险属性：返回 (已消耗字节数, 是否还要跳属性值)。
fn dangerous_attr(s: &str) -> Option<(usize, bool)> {
    let bytes = s.as_bytes();
    // 事件属性：onload= / onclick= ...
    if bytes.len() >= 3 && (bytes[0] | 32) == b'o' && (bytes[1] | 32) == b'n' {
        let mut name_end = 2;
        while name_end < bytes.len() && bytes[name_end].is_ascii_alphabetic() {
            name_end += 1;
        }
        if name_end > 2 {
            let after = bytes[name_end..]
                .iter()
                .skip_while(|byte| byte.is_ascii_whitespace())
                .next();
            if after == Some(&b'=') {
                return Some((name_end, true));
            }
        }
    }
    // 外链属性：href= / xlink:href= （内联 svg 不需要任何 href）。
    for prefix in [b"xlink:href".as_slice(), b"href".as_slice()] {
        if bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix) {
            let after = bytes[prefix.len()..]
                .iter()
                .skip_while(|byte| byte.is_ascii_whitespace())
                .next();
            if after == Some(&b'=') {
                return Some((prefix.len(), true));
            }
        }
    }
    // url(...) 里的外链 CSS：直接吞掉整个 url(...)
    if bytes.len() >= 4 && bytes[..4].eq_ignore_ascii_case(b"url(") {
        if let Some(close) = s.find(')') {
            return Some((close + 1, false));
        }
    }
    None
}

fn skip_attr_value(s: &str, from: usize) -> usize {
    let rest = &s[from..];
    let trimmed = rest.trim_start();
    let skipped_ws = rest.len() - trimmed.len();
    let mut i = from + skipped_ws;
    if let Some(quote) = trimmed.chars().next() {
        if quote == '"' || quote == '\'' {
            i += 1;
            if let Some(close) = s[i..].find(quote) {
                return i + close + 1;
            }
            return s.len();
        }
    }
    // 无引号属性值：吃到空白或 >
    while i < s.len() {
        let c = s[i..].chars().next().unwrap();
        if c.is_whitespace() || c == '>' {
            break;
        }
        i += c.len_utf8();
    }
    i
}

/// 给 SVG 根标签补上具体宽高（没有就按 `viewBox` 算）。
///
/// SVG 当 `<img src="data:...">` 用时**必须有固有尺寸**：只有 `viewBox` + `width="100%"`
/// 时浏览器算不出高度，预览框里就是一片空白。老版本生成的图都是这种写法，已经在
/// 题目里的配图没法重生成，所以在读取时补一脚，不用用户重新作图。
pub fn ensure_intrinsic_size(svg: &str) -> String {
    let Some(open) = find_ci(svg, "<svg") else {
        return svg.to_string();
    };
    // 扫到根标签的结束 `>`，跳过属性值里的引号内容。
    let bytes = svg.as_bytes();
    let mut i = open + 4;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let byte = bytes[i];
        match quote {
            Some(current) => {
                if byte == current {
                    quote = None;
                }
            }
            None => match byte {
                b'"' | b'\'' => quote = Some(byte),
                b'>' => break,
                _ => {}
            },
        }
        i += 1;
    }
    if i >= bytes.len() {
        return svg.to_string();
    }
    let attrs = parse_attrs(&svg[open + 4..i]);

    let Some(view_box) = attrs
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("viewBox"))
        .and_then(|(_, value)| view_box_size(value))
    else {
        return svg.to_string();
    };
    let numeric = |name: &str| {
        attrs
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim().parse::<f64>().is_ok())
            .unwrap_or(false)
    };
    if numeric("width") && numeric("height") {
        return svg.to_string();
    }

    let mut out = String::with_capacity(svg.len() + 32);
    out.push_str("<svg");
    for (name, value) in &attrs {
        if name.eq_ignore_ascii_case("width") || name.eq_ignore_ascii_case("height") {
            continue;
        }
        out.push_str(&format!(" {name}=\"{value}\""));
    }
    out.push_str(&format!(" width=\"{}\" height=\"{}\">", trim(view_box.0), trim(view_box.1)));
    out.push_str(&svg[i + 1..]);
    out
}

fn trim(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `viewBox="minX minY w h"` → (w, h)；写错的直接当没有。
fn view_box_size(value: &str) -> Option<(f64, f64)> {
    let parts: Vec<f64> = value
        .split(|ch: char| ch.is_whitespace() || ch == ',')
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<f64>().ok())
        .collect();
    if parts.len() != 4 {
        return None;
    }
    let (w, h) = (parts[2], parts[3]);
    if w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite() {
        Some((w, h))
    } else {
        None
    }
}

/// 把根标签里的属性拆成 (名, 值)，值可能带引号也可能不带。
fn parse_attrs(tag: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let mut rest = tag;
    while let Some(start) = rest.find(|ch: char| !ch.is_whitespace() && ch != '/') {
        rest = &rest[start..];
        let name_end = rest
            .find(|ch: char| ch.is_whitespace() || ch == '=' || ch == '/')
            .unwrap_or(rest.len());
        if name_end == 0 {
            break;
        }
        let name = rest[..name_end].to_string();
        rest = &rest[name_end..];
        let rest_trimmed = rest.trim_start();
        if !rest_trimmed.starts_with('=') {
            attrs.push((name, String::new()));
            rest = rest_trimmed;
            continue;
        }
        rest = rest_trimmed[1..].trim_start();
        let (value, tail) = match rest.chars().next() {
            Some(quote @ ('"' | '\'')) => {
                let body = &rest[1..];
                match body.find(quote) {
                    Some(end) => (body[..end].to_string(), &body[end + 1..]),
                    None => (body.to_string(), ""),
                }
            }
            _ => {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                (rest[..end].to_string(), &rest[end..])
            }
        };
        attrs.push((name, value));
        rest = tail;
    }
    attrs
}

/// 把 SVG 文本转成可直接放进 <img src> 的 data URL。
pub fn svg_data_url(svg: &str) -> String {
    use base64::Engine;
    let svg = ensure_intrinsic_size(svg);
    format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg.as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::{
        close_open_tags, ensure_intrinsic_size, extract_figure, sanitize_svg, svg_data_url,
        unescape_loose,
    };

    #[test]
    fn keeps_valid_svg() {
        let svg = r#"<svg viewBox="0 0 100 100"><circle cx="50" cy="50" r="40"/><text x="10" y="10">△ABC</text></svg>"#;
        let clean = sanitize_svg(svg).unwrap();
        assert!(clean.contains("<svg"));
        assert!(clean.contains("</svg>"));
        assert!(clean.contains("△ABC"));
    }

    #[test]
    fn strips_script_and_event_handlers() {
        let svg = r#"<svg viewBox="0 0 10 10" onload="alert(1)"><script>alert(2)</script><circle onclick="x()" cx="5" cy="5" r="2"/></svg>"#;
        let clean = sanitize_svg(svg).unwrap();
        assert!(!clean.to_lowercase().contains("script"));
        assert!(!clean.to_lowercase().contains("onload"));
        assert!(!clean.to_lowercase().contains("onclick"));
        assert!(clean.contains("<circle"));
    }

    #[test]
    fn strips_external_href() {
        let svg = r#"<svg viewBox="0 0 10 10"><image href="https://evil.example/x.png"/></svg>"#;
        let clean = sanitize_svg(svg).unwrap();
        assert!(!clean.contains("https://"));
        assert!(!clean.to_lowercase().contains("href"));
    }

    #[test]
    fn rejects_missing_svg() {
        assert!(sanitize_svg("hello world").is_err());
    }

    #[test]
    fn data_url_is_base64_svg() {
        let url = svg_data_url("<svg></svg>");
        assert!(url.starts_with("data:image/svg+xml;base64,"));
    }

    /// 被 max_tokens 截断的图形：没有 </svg>，也要补成可渲染的完整文档。
    #[test]
    fn repairs_truncated_svg() {
        let svg = r#"<svg viewBox="0 0 100 100"><g><text x="10" y="20">P"#;
        let clean = sanitize_svg(svg).unwrap();
        assert!(clean.contains("</svg>"), "实际：{clean}");
        assert!(clean.trim_end().ends_with("</svg>"));
    }

    /// 半截标签直接丢掉，别把 `<text x="12` 留在文档里。
    #[test]
    fn drops_half_written_tag() {
        let fixed = close_open_tags(super::strip_incomplete_tail(
            r#"<svg><text x="1" y="2">AB<text x="3"#,
        ));
        assert!(!fixed.contains("x=\"3"));
        assert!(fixed.ends_with("</text></svg>") || fixed.ends_with("</svg></text></svg>"));
    }

    /// 模型直接吐 SVG（没按 JSON 输出）也要能用。
    #[test]
    fn accepts_bare_svg() {
        let raw = "<svg viewBox=\"0 0 10 10\"><line x1=\"0\" y1=\"0\" x2=\"9\" y2=\"9\"/></svg>";
        let clean = extract_figure(raw).unwrap();
        assert!(clean.starts_with("<svg"));
    }

    /// JSON 外壳坏了（引号没转义 / 被截断），从原文里抠出来的 SVG 必须还原本义字符。
    #[test]
    fn salvages_svg_from_broken_json() {
        let raw = r#"{"svg":"<svg viewBox=\"0 0 10 10\"><text x=\"1\">AB</text></svg>"#;
        let clean = extract_figure(raw).unwrap();
        assert!(clean.contains(r#"viewBox="0 0 10 10""#), "实际：{clean}");
        assert!(!clean.contains("\\\""), "实际：{clean}");
    }

    /// 模型这次的场景 JSON 断在 fill 值中间（真实遇到过）：补齐后仍要出一张能用的图，
    /// 而且缺顶点的那个面要整面丢掉，不能画成一个错的三角形。
    #[test]
    fn renders_scene_truncated_mid_value() {
        let raw = r#"{"scene":{"title":"四棱锥 P-ABCD","points":[{"id":"A","x":28,"y":78},{"id":"B","x":68,"y":78},{"id":"C","x":58,"y":58},{"id":"D","x":18,"y":58},{"id":"P","x":43,"y":12}],"segments":[{"a":"P","b":"A"},{"a":"P","b":"B"},{"a":"P","b":"C"},{"a":"P","b":"D"},{"a":"A","b":"B"},{"a":"B","b":"C"},{"a":"C","b":"D"},{"a":"D","b":"A"}],"faces":[{"points":["A","B","C","D"]},{"points":["A","E","C"],"fill":"rgba(220,24"#;
        let svg = extract_figure(raw).unwrap();
        assert!(svg.starts_with("<svg"), "实际：{svg}");
        assert_eq!(svg.matches("<line").count(), 8);
        assert_eq!(svg.matches("<polygon").count(), 1, "缺 E 的那个面要整面丢掉");
    }

    /// 老图只有 width="100%"，当 <img> 用时高度算不出来、整张空白。
    #[test]
    fn adds_intrinsic_size_from_view_box() {
        let old = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 480 430\" width=\"100%\"><line/></svg>";
        let fixed = ensure_intrinsic_size(old);
        assert!(fixed.contains("width=\"480\" height=\"430\""), "实际：{fixed}");
        assert!(!fixed.contains("width=\"100%\""));
        assert!(fixed.ends_with("</svg>"));
        // 已经带了具体宽高的不要改。
        let done = "<svg viewBox=\"0 0 480 430\" width=\"480\" height=\"430\"></svg>";
        assert_eq!(ensure_intrinsic_size(done), done);
        // 没有 viewBox 就无从推算，原样返回，别造一个错的尺寸。
        let no_box = "<svg width=\"100%\"></svg>";
        assert_eq!(ensure_intrinsic_size(no_box), no_box);
    }

    #[test]
    fn unescapes_only_structural_escapes() {
        let text = r#"<text>$\sqrt{2}$</text>\"#;
        assert_eq!(unescape_loose(text), r#"<text>$\sqrt{2}$</text>"#);
    }
}
