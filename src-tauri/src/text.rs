//! 文本处理：题干指纹、题型别名、Markdown 图片引用。
//!
//! 题干指纹与中心系统同一口径：全半角归一 → 去空白 → 去标点 → 小写。
//! 本地查重先用它做精确匹配，再用字符 bigram 的 Jaccard 算相似度。

/// 生成题干指纹。公式定界符、标点、空白都去掉，只留内容字符。
pub fn normalize_stem(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let ch = halfwidth(ch);
        if ch.is_whitespace() {
            continue;
        }
        if is_noise(ch) {
            continue;
        }
        for lower in ch.to_lowercase() {
            out.push(lower);
        }
    }
    out
}

fn halfwidth(ch: char) -> char {
    match ch {
        '\u{3000}' => ' ',
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(ch as u32 - 0xFEE0).unwrap_or(ch),
        _ => ch,
    }
}

fn is_noise(ch: char) -> bool {
    matches!(
        ch,
        ',' | '.'
            | ';'
            | ':'
            | '!'
            | '?'
            | '"'
            | '\''
            | '`'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | '~'
            | '|'
            | '/'
            | '\\'
            | '$'
            | '&'
            | '#'
            | '@'
            | '、'
            | '。'
            | '，'
            | '；'
            | '：'
            | '！'
            | '？'
            | '…'
            | '—'
            | '–'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '《'
            | '》'
            | '〈'
            | '〉'
            | '【'
            | '】'
            | '「'
            | '」'
            | '『'
            | '』'
            | '·'
            | '～'
            | '％'
            | '＋'
    )
}

/// 题型别名 → 内部枚举值。兼容中文与英文写法。
pub fn normalize_type(raw: &str) -> Option<&'static str> {
    let key: String = raw
        .trim()
        .to_uppercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let key = key.replace('題', "题");
    match key.as_str() {
        "SINGLE" | "SINGLE_CHOICE" | "CHOICE" | "单选" | "单选题" | "选择题" | "单项选择" => {
            Some("SINGLE")
        }
        "MULTI" | "MULTIPLE" | "MULTI_CHOICE" | "多选" | "多选题" | "不定项选择" => {
            Some("MULTI")
        }
        "FILL" | "BLANK" | "填空" | "填空题" => Some("FILL"),
        "SOLVED" | "ANSWER" | "SUBJECTIVE" | "解答" | "解答题" | "计算" | "计算题" | "证明"
        | "证明题" | "简答" | "简答题" | "作文" => Some("SOLVED"),
        _ => None,
    }
}

pub fn type_label(kind: &str) -> &'static str {
    match kind {
        "SINGLE" => "单选题",
        "MULTI" => "多选题",
        "FILL" => "填空题",
        _ => "解答题",
    }
}

/// Markdown 里的图片引用：`![alt](images/x.png)`。
pub struct MdImage {
    /// 整段原文（含 `![` 与 `)`），用于替换
    pub whole: String,
    pub alt: String,
    pub path: String,
}

pub fn extract_md_images(text: &str) -> Vec<MdImage> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'!' && bytes[i + 1] == b'[' {
            if let Some((end, alt, path)) = parse_md_image(text, i) {
                out.push(MdImage {
                    whole: text[i..end].to_string(),
                    alt,
                    path,
                });
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn parse_md_image(text: &str, start: usize) -> Option<(usize, String, String)> {
    let rest = &text[start..];
    let close_bracket = rest.find(']')?;
    if !rest[close_bracket + 1..].starts_with('(') {
        return None;
    }
    let alt = rest[2..close_bracket].to_string();
    let path_start = close_bracket + 2;
    let close_paren = rest[path_start..].find(')')? + path_start;
    let raw_path = rest[path_start..close_paren].trim();
    // 去掉可选标题：![](x.png "图1")
    let path = raw_path
        .split_whitespace()
        .next()?
        .trim_matches('"')
        .to_string();
    if path.is_empty() {
        return None;
    }
    Some((start + close_paren + 1, alt, path))
}

/// 按文件头嗅探图片格式。SVG 是文本，额外按开头内容判断。
/// AI 制图产出的是 SVG，必须在这里放行，否则 asset_add / asset_data / 入库都会当成非图片。
pub fn sniff_image(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.len() < 12 {
        return None;
    }
    // SVG 前面可能带 BOM、空白或 XML 声明；只在开头一小段里找，避免整篇扫描。
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("<svg")
        || (head.starts_with("<?xml") && head.contains("<svg"))
    {
        return Some(("image/svg+xml", "svg"));
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(("image/jpeg", "jpg"));
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(("image/png", "png"));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some(("image/gif", "gif"));
    }
    if &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(("image/webp", "webp"));
    }
    None
}

/// 字符 bigram 集合，用于本地相似度；不依赖分词与模型。
pub fn bigrams(text: &str) -> std::collections::HashSet<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut set = std::collections::HashSet::new();
    if chars.len() < 2 {
        if !chars.is_empty() {
            set.insert(chars[0].to_string());
        }
        return set;
    }
    for pair in chars.windows(2) {
        set.insert(pair.iter().collect());
    }
    set
}

pub fn jaccard(
    a: &std::collections::HashSet<String>,
    b: &std::collections::HashSet<String>,
) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

pub fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// 用户可见的短摘要：去掉图片引用、压平换行，用于列表标题。
pub fn excerpt(text: &str, max: usize) -> String {
    let mut flat = text.to_string();
    for md in extract_md_images(text) {
        flat = flat.replace(&md.whole, "【图】");
    }
    let flat = flat.replace('\n', " ").replace('\r', " ");
    let trimmed = flat.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_removes_space_punct_and_fullwidth() {
        assert_eq!(
            normalize_stem("若 Ａ＝２，则 2A 等于多少？"),
            normalize_stem("若A=2,则2a等于多少")
        );
    }

    #[test]
    fn type_alias() {
        assert_eq!(normalize_type("单选题"), Some("SINGLE"));
        assert_eq!(normalize_type("multiple"), Some("MULTI"));
        assert_eq!(normalize_type("解答题"), Some("SOLVED"));
        assert_eq!(normalize_type("判断题"), None);
    }

    #[test]
    fn md_images() {
        let items = extract_md_images("如图![](images/q1-1.png)，求角 A。");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, "images/q1-1.png");
        assert_eq!(items[0].whole, "![](images/q1-1.png)");
    }
}
