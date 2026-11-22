//! 模型 JSON 兜底解析（LlmJson 的 Rust 移植）。
//!
//! 模型经常把 `\frac{x^2}{a^2}` 原样塞进 JSON 字符串：`\f` 被当成换页符、
//! `\sqrt` 直接是非法转义、`\times` 被静默吃成制表符。这里在**解析之前**按已知
//! 命令名修反斜杠，而不是等解析报错再兜——`\times` 那种情况严格解析是会「成功」的。
//!
//! 与中心系统 `com.neuralkb.llm.LlmJson` 同一套规则与测试用例。

use serde_json::Value;
use std::collections::HashSet;

const SNIPPET_CHARS: usize = 120;

fn control_letter_commands() -> HashSet<&'static str> {
    [
        "bar",
        "backslash",
        "because",
        "begin",
        "beta",
        "bf",
        "bigcap",
        "bigcup",
        "bigoplus",
        "bigotimes",
        "bigvee",
        "bigwedge",
        "binom",
        "bmod",
        "boldsymbol",
        "bot",
        "bowtie",
        "boxed",
        "bullet",
        "flat",
        "forall",
        "fork",
        "frac",
        "frown",
        "nabla",
        "natural",
        "ncong",
        "ne",
        "neg",
        "neq",
        "newline",
        "ni",
        "nleq",
        "nmid",
        "not",
        "notin",
        "np",
        "nparallel",
        "nrightarrow",
        "nsim",
        "nu",
        "rangle",
        "rceil",
        "rfloor",
        "rho",
        "right",
        "rightarrow",
        "Rightarrow",
        "rightleftharpoons",
        "risingdotseq",
        "rm",
        "root",
        "rtimes",
        "tan",
        "tanh",
        "tau",
        "tbinom",
        "text",
        "textbf",
        "textit",
        "textrm",
        "tfrac",
        "therefore",
        "theta",
        "thickapprox",
        "thinspace",
        "tilde",
        "times",
        "to",
        "top",
        "triangle",
        "triangleleft",
        "triangleright",
    ]
    .into_iter()
    .collect()
}

/// 解析模型返回的 JSON。失败时给出带原文片段的中文错误。
pub fn parse(raw: &str, what: &str) -> Result<Value, String> {
    let label = if what.trim().is_empty() {
        "模型"
    } else {
        what.trim()
    };
    if raw.trim().is_empty() {
        return Err(format!("{label}没有返回内容，请重试"));
    }
    let body = strip_fence(&strip_bom(raw).trim().to_string());

    let candidate = candidate(&body)
        .ok_or_else(|| format!("{label}没有按 JSON 作答，它说的是「{}」", snippet(&body)))?;

    let repaired = repair_escapes(&candidate);
    if let Some(node) = try_parse(&repaired) {
        return Ok(node);
    }
    if let Some(node) = try_parse(&candidate) {
        return Ok(node);
    }
    let fixed = balance(&drop_trailing_commas(&repaired));
    if let Some(node) = try_parse(&fixed) {
        return Ok(node);
    }
    Err(format!(
        "{label}返回的 JSON 不合法，重试一次通常就好；模型原文开头「{}」",
        snippet(&candidate)
    ))
}

fn try_parse(text: &str) -> Option<Value> {
    if text.trim().is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(text) {
        Ok(value @ (Value::Object(_) | Value::Array(_))) => Some(value),
        _ => None,
    }
}

fn strip_bom(text: &str) -> String {
    text.trim_start_matches(['\u{FEFF}', '\u{200B}'])
        .to_string()
}

/// ```json … ``` 或 ``` … ```：只取代码块里的内容。
fn strip_fence(text: &str) -> String {
    let Some(fence) = text.find("```") else {
        return text.to_string();
    };
    let Some(line_start) = text[fence..].find('\n').map(|offset| fence + offset) else {
        return text.to_string();
    };
    let after = &text[line_start + 1..];
    let inside = match after.find("```") {
        Some(close) => &after[..close],
        None => after,
    };
    let trimmed = inside.trim();
    if trimmed.is_empty() {
        text.to_string()
    } else {
        trimmed.to_string()
    }
}

/// 从「这里是 JSON：{...}」里截出第一个配平的对象；括号计数跳过字符串内容。
fn candidate(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let start = chars.iter().position(|c| *c == '{' || *c == '[')?;
    let mut in_string = false;
    let mut stack: Vec<char> = Vec::new();
    let mut i = start;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            if c == '\\' {
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if stack.is_empty() {
                    return Some(chars[start..i].iter().collect());
                }
                stack.pop();
                if stack.is_empty() {
                    return Some(chars[start..=i].iter().collect());
                }
            }
            _ => {}
        }
        i += 1;
    }
    Some(chars[start..].iter().collect())
}

fn repair_escapes(json: &str) -> String {
    let chars: Vec<char> = json.chars().collect();
    let known = control_letter_commands();
    let mut out = String::with_capacity(json.len() + 64);
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !in_string {
            out.push(c);
            if c == '"' {
                in_string = true;
            }
            i += 1;
            continue;
        }
        if c == '\\' {
            if i + 1 >= chars.len() {
                out.push_str("\\\\");
                break;
            }
            let next = chars[i + 1];
            if next == '"' || next == '\\' || next == '/' {
                out.push(c);
                out.push(next);
                i += 2;
                continue;
            }
            if next == 'u' {
                if i + 5 < chars.len() && is_hex4(&chars, i + 2) {
                    for ch in &chars[i..i + 6] {
                        out.push(*ch);
                    }
                    i += 6;
                } else {
                    out.push_str("\\\\u");
                    i += 2;
                }
                continue;
            }
            if matches!(next, 'b' | 'f' | 'n' | 'r' | 't') {
                let run = letter_run(&chars, i + 1);
                if known.contains(run.as_str()) {
                    out.push_str("\\\\");
                    out.push(next);
                } else {
                    out.push('\\');
                    out.push(next);
                }
                i += 2;
                continue;
            }
            out.push_str("\\\\");
            out.push(next);
            i += 2;
            continue;
        }
        if c == '"' {
            let mut probe = i + 1;
            while probe < chars.len() && chars[probe].is_whitespace() {
                probe += 1;
            }
            let closer = probe >= chars.len() || matches!(chars[probe], ',' | '}' | ']' | ':');
            if closer {
                out.push('"');
                in_string = false;
            } else {
                out.push_str("\\\"");
            }
            i += 1;
            continue;
        }
        if (c as u32) < 0x20 {
            match c {
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                other => out.push_str(&format!("\\u{:04x}", other as u32)),
            }
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 输出被截断时把没闭合的字符串和括号补齐。
fn balance(json: &str) -> String {
    let chars: Vec<char> = json.chars().collect();
    let mut in_string = false;
    let mut stack: Vec<char> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            if c == '\\' {
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
    let mut out: Vec<char> = json.trim().chars().collect();
    if in_string {
        out.push('"');
    }
    while let Some(close) = stack.pop() {
        while matches!(out.last(), Some(c) if c.is_whitespace()) {
            out.pop();
        }
        if out.last() == Some(&',') {
            out.pop();
        }
        out.push(close);
    }
    out.into_iter().collect()
}

fn drop_trailing_commas(json: &str) -> String {
    let chars: Vec<char> = json.chars().collect();
    let mut out = String::with_capacity(json.len());
    for (index, c) in chars.iter().enumerate() {
        if *c == ',' {
            let mut probe = index + 1;
            while probe < chars.len() && chars[probe].is_whitespace() {
                probe += 1;
            }
            if probe < chars.len() && (chars[probe] == '}' || chars[probe] == ']') {
                continue;
            }
        }
        out.push(*c);
    }
    out
}

fn letter_run(chars: &[char], from: usize) -> String {
    let mut end = from;
    while end < chars.len() && chars[end].is_ascii_alphabetic() {
        end += 1;
    }
    chars[from..end].iter().collect()
}

fn is_hex4(chars: &[char], from: usize) -> bool {
    if from + 4 > chars.len() {
        return false;
    }
    chars[from..from + 4].iter().all(|c| c.is_ascii_hexdigit())
}

pub fn snippet(raw: &str) -> String {
    let flat: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(SNIPPET_CHARS).collect();
    if flat.chars().count() > SNIPPET_CHARS {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_json() {
        let value = parse(r#"{"a":1}"#, "模型").unwrap();
        assert_eq!(value["a"], 1);
    }

    #[test]
    fn latex_single_backslash() {
        // 模型直接写 \sqrt：非法 json 转义，必须翻倍后才能解析
        let raw = r#"{"stem":"求 $\sqrt{2}$ 的值"}"#;
        let value = parse(raw, "模型").unwrap();
        assert_eq!(value["stem"], "求 $\\sqrt{2}$ 的值");
    }

    #[test]
    fn latex_times_is_not_tab() {
        // \times 的 \t 是合法转义，不修就会静默变成制表符
        let raw = "{\"stem\":\"$2\\times 3$\"}";
        let value = parse(raw, "模型").unwrap();
        let stem = value["stem"].as_str().unwrap();
        assert!(stem.contains("\\times"), "实际：{stem:?}");
        assert!(!stem.contains('\t'));
    }

    #[test]
    fn real_newline_escape_preserved() {
        let value = parse("{\"stem\":\"第一行\\n第二行\"}", "模型").unwrap();
        assert_eq!(value["stem"], "第一行\n第二行");
    }

    #[test]
    fn fence_and_trailing_comma() {
        let raw = "```json\n{\"questions\":[{\"stem\":\"a\"},]}\n```";
        let value = parse(raw, "模型").unwrap();
        assert_eq!(value["questions"][0]["stem"], "a");
    }

    #[test]
    fn truncated_payload_is_balanced() {
        let raw = r#"{"questions":[{"stem":"x","answer":"A""#;
        let value = parse(raw, "模型").unwrap();
        assert_eq!(value["questions"][0]["answer"], "A");
    }

    #[test]
    fn prose_wrapper_is_ignored() {
        let raw = "好的，结果如下：{\"stem\":\"a\"} 以上。";
        let value = parse(raw, "模型").unwrap();
        assert_eq!(value["stem"], "a");
    }
}
