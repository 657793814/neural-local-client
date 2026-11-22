//! 内置浏览器：一个独立的 WebviewWindow 打开第三方试题网址。
//!
//! 页面上注入一块浮动工具：先在 DOM 里找题块，逐题给「收这题」按钮，
//! 右下角再给一个「识别本页 N 题」。抓的是结构化字段（题干 / 选项 / 答案 /
//! 解析 / 知识点 / 难度 / 来源），不再把整页文字丢给模型。
//! 找不到题块时才退回整页文字，截图链路保留，适合图片型题目。
//! 每个会话共享系统 webview 存储；关闭窗口即结束这一会话。

use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Rect, WebviewUrl};

/// 内嵌在「网页取题」页里的子 WebView。标签同时用于 capability 精确授权：
/// 只给它 `browser_collect` / `emit_browser_text` 两条命令。
pub const SITE: &str = "site";
pub const MAX_WEB_ITEMS: usize = 60;
const MAX_OPTIONS: usize = 12;
const MAX_STEM_CHARS: usize = 12_000;
const MAX_OPTION_CHARS: usize = 2_000;
const MAX_ANSWER_CHARS: usize = 4_000;
const MAX_ANALYSIS_CHARS: usize = 12_000;

pub struct PreparedWebItems {
    pub items: Vec<serde_json::Value>,
    pub total: usize,
    pub truncated: bool,
}

fn truncate_chars(raw: &str, max: usize) -> (String, bool) {
    let mut chars = raw.chars();
    let text: String = chars.by_ref().take(max).collect();
    (text, chars.next().is_some())
}

fn push_warning(map: &mut serde_json::Map<String, serde_json::Value>, message: &str) {
    let warnings = map
        .entry("warnings")
        .or_insert_with(|| serde_json::json!([]));
    if !warnings.is_array() {
        *warnings = serde_json::json!([]);
    }
    if let Some(items) = warnings.as_array_mut() {
        items.push(serde_json::json!(message));
    }
}

fn limit_field(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    max: usize,
    label: &str,
) {
    let limited = map
        .get(key)
        .and_then(|value| value.as_str())
        .map(|raw| truncate_chars(raw, max));
    if let Some((text, truncated)) = limited {
        map.insert(key.into(), serde_json::json!(text));
        if truncated {
            push_warning(map, &format!("{label}过长，已截断，请核对原文"));
        }
    }
}

fn limit_string_list(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    max_items: usize,
) {
    let Some(value) = map.get_mut(key) else {
        return;
    };
    match value {
        serde_json::Value::Array(items) => {
            items.truncate(max_items);
            for item in items {
                if let Some(raw) = item.as_str() {
                    let (text, _) = truncate_chars(raw, 200);
                    *item = serde_json::json!(text);
                }
            }
        }
        serde_json::Value::String(raw) => {
            let (text, _) = truncate_chars(raw, 1_200);
            *raw = text;
        }
        _ => {}
    }
}

/// 远程网页是不可信输入：即使注入脚本被替换，题数、字段与来源也不能越过这里的边界。
pub fn prepare_web_items(
    items: Vec<serde_json::Value>,
    source: &str,
) -> Result<PreparedWebItems, String> {
    let total = items.len();
    if total == 0 {
        return Err("这一页没抓到题目".into());
    }
    let truncated = total > MAX_WEB_ITEMS;
    let mut prepared = Vec::with_capacity(total.min(MAX_WEB_ITEMS));
    for item in items.into_iter().take(MAX_WEB_ITEMS) {
        let serde_json::Value::Object(mut map) = item else {
            continue;
        };
        // 只认 SITE 窗口的当前地址，网页自己报的 source / url 一律覆盖。
        map.insert("source".into(), serde_json::json!(source));
        limit_field(&mut map, "stem", MAX_STEM_CHARS, "题干");
        limit_field(&mut map, "answer", MAX_ANSWER_CHARS, "答案");
        limit_field(&mut map, "analysis", MAX_ANALYSIS_CHARS, "解析");
        limit_field(&mut map, "type", 32, "题型");
        limit_field(&mut map, "difficulty", 32, "难度");
        // 先收网页自带的少量 warning，给后面的截断提示留出位置。
        limit_string_list(&mut map, "warnings", 8);

        let mut options_truncated = false;
        if let Some(serde_json::Value::Array(options)) = map.get_mut("options") {
            options_truncated = options.len() > MAX_OPTIONS;
            options.truncate(MAX_OPTIONS);
            for option in options {
                match option {
                    serde_json::Value::String(raw) => {
                        let (text, truncated) = truncate_chars(raw, MAX_OPTION_CHARS);
                        *raw = text;
                        options_truncated |= truncated;
                    }
                    serde_json::Value::Object(option) => {
                        if let Some(raw) = option.get("key").and_then(|value| value.as_str()) {
                            let (text, _) = truncate_chars(raw, 8);
                            option.insert("key".into(), serde_json::json!(text));
                        }
                        if let Some(raw) = option.get("text").and_then(|value| value.as_str()) {
                            let (text, truncated) = truncate_chars(raw, MAX_OPTION_CHARS);
                            option.insert("text".into(), serde_json::json!(text));
                            options_truncated |= truncated;
                        }
                    }
                    _ => {}
                }
            }
        }
        if options_truncated {
            push_warning(&mut map, "选项过多或过长，已截断，请核对原文");
        }
        limit_string_list(&mut map, "knowledgeNames", 6);
        prepared.push(serde_json::Value::Object(map));
    }
    if prepared.is_empty() {
        return Err("这一页没抓到可用的题目".into());
    }
    Ok(PreparedWebItems {
        items: prepared,
        total,
        truncated,
    })
}

/// 校验网址：只放行 http/https，避免 file:// 之类的本地路径被塞进来。
pub fn normalize_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("请填写网址".into());
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let parsed =
        tauri::Url::parse(&with_scheme).map_err(|_| "这个网址看不懂，检查一下拼写".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("只支持 http / https 网址".into());
    }
    Ok(parsed.to_string())
}

/// 内嵌视图的句柄。子 WebView 挂在主窗口上，标签固定为 `SITE`。
pub fn site_view(app: &AppHandle) -> Option<tauri::Webview> {
    app.get_webview(SITE)
}

/// 懒创建内嵌视图：第一次打开网址时才建，之后一直复用同一份登录态。
fn ensure_view(app: &AppHandle, url: &str) -> Result<tauri::Webview, String> {
    if let Some(view) = site_view(app) {
        return Ok(view);
    }
    let main = app
        .get_window("main")
        .ok_or_else(|| "主窗口还没准备好，稍后再试".to_string())?;
    let target = tauri::Url::parse(url).map_err(|_| "这个网址看不懂，检查一下拼写".to_string())?;
    let builder =
        WebviewBuilder::new(SITE, WebviewUrl::External(target)).on_page_load(|webview, payload| {
            if payload.event() != tauri::webview::PageLoadEvent::Finished {
                return;
            }
            // 等 DOM 稳定一下再注入，避免和站点自己的初始化打架。
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(600));
                let _ = webview.eval(OVERLAY_SCRIPT);
            });
        });
    main.add_child(
        builder,
        LogicalPosition::new(0.0, 0.0),
        // 先给一个能正常布局的尺寸；前端随后按占位框校正，避免 0×0 时 WKWebView 不渲染。
        LogicalSize::new(800.0, 600.0),
    )
    .map_err(|e| format!("创建内置浏览视图失败：{e}"))
}

/// 打开（或复用）内嵌试题网页。
pub fn open(app: &AppHandle, raw_url: &str) -> Result<(), String> {
    let url = normalize_url(raw_url)?;
    let view = ensure_view(app, &url)?;
    let current = view
        .url()
        .map(|value| value.to_string())
        .unwrap_or_default();
    if current != url {
        view.navigate(tauri::Url::parse(&url).map_err(|e| e.to_string())?)
            .map_err(|e| format!("跳转失败：{e}"))?;
    }
    let _ = view.show();
    let _ = view.set_focus();
    Ok(())
}

/// 主窗布局变化时把内嵌视图摆到占位框上。坐标是主窗内容区逻辑像素。
pub fn set_bounds(app: &AppHandle, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    let view = site_view(app).ok_or("内置浏览器还没打开")?;
    view.set_bounds(Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(width.max(0.0), height.max(0.0)).into(),
    })
    .map_err(|e| format!("调整内置浏览器位置失败：{e}"))?;
    let _ = view.show();
    Ok(())
}

/// 切走「网页取题」时把内嵌视图藏起来，页面和登录态都留着。
pub fn hide(app: &AppHandle) {
    if let Some(view) = site_view(app) {
        let _ = view.hide();
    }
}

pub fn close(app: &AppHandle) {
    if let Some(view) = site_view(app) {
        let _ = view.close();
    }
}

pub fn is_open(app: &AppHandle) -> bool {
    site_view(app).is_some()
}

/// 让已注入的脚本重新扫一遍页面（站内 SPA 翻页、异步加载时用）。
pub fn rescan(app: &AppHandle) {
    if let Some(view) = site_view(app) {
        let _ = view.eval(OVERLAY_SCRIPT);
    }
}

// ---------------------------------------------------------------------------
// 页面内脚本：结构化抓题 + 逐题 / 整页按钮
// ---------------------------------------------------------------------------
//
// 所有按钮都在页面 DOM 里，点击后直接用 Tauri IPC 调 `browser_collect`，
// Rust 侧归一化后创建一条已完成的 `capture_web` 任务，再通知主窗。
// 公式优先取 MathJax / KaTeX 源码，避免 innerText 拿到渲染后的杂字。

pub const OVERLAY_SCRIPT: &str = r#"
(function() {
    if (window.__nbOverlay) return;
    window.__nbOverlay = true;

    var URL = function() { return location.href; };
    var MAX_ITEMS = 60;
    var MAX_OPTIONS = 12;
    var MAX_STEM = 12000;
    var MAX_OPTION = 2000;
    var MAX_ANSWER = 4000;
    var MAX_ANALYSIS = 12000;

    // ---------- 基础工具 ----------
    // 内嵌视图没有"新窗口"可开：target=_blank 或 ⌘ 点击会静默无反应，这里改成当前页跳转。
    document.addEventListener('click', function (e) {
        var node = e.target;
        while (node && node.nodeType !== 1) node = node.parentNode;
        var link = node && node.closest ? node.closest('a[href]') : null;
        if (!link) return;
        if (link.target === '_blank' || e.metaKey || e.ctrlKey) {
            e.preventDefault();
            location.href = link.href;
        }
    }, true);
    try {
        window.open = function (url) {
            if (url) location.href = url;
            return null;
        };
    } catch (error) {}

    function invoke(cmd, args) {
        if (!window.__TAURI_INTERNALS__) {
            return Promise.reject('这个窗口没有连上本地题库');
        }
        return window.__TAURI_INTERNALS__.invoke(cmd, args);
    }

    function replaceMath(root) {
        try {
            var nodes = root.querySelectorAll('annotation[encoding="application/x-tex"], [data-tex], .katex-mathml, .MathJax');
            for (var i = 0; i < nodes.length; i++) {
                var node = nodes[i];
                var tex = node.getAttribute('data-tex') || '';
                if (!tex && node.tagName && node.tagName.toLowerCase() === 'annotation') {
                    tex = node.textContent || '';
                }
                if (!tex && node.querySelector) {
                    var ann = node.querySelector('annotation[encoding="application/x-tex"]');
                    if (ann) tex = ann.textContent || '';
                }
                if (!tex) continue;
                var span = document.createElement('span');
                span.textContent = ' $' + tex.trim() + '$ ';
                if (node.parentNode) node.parentNode.replaceChild(span, node);
            }
        } catch (e) { /* 公式替换失败不影响文字 */ }
    }

    function isVisible(node) {
        if (!node || node.nodeType !== 1) return false;
        var current = node;
        while (current && current.nodeType === 1) {
            if (current.hidden || current.getAttribute('aria-hidden') === 'true') return false;
            try {
                var style = window.getComputedStyle ? window.getComputedStyle(current) : current.style;
                if (style && (style.display === 'none' || style.visibility === 'hidden' ||
                    style.visibility === 'collapse' || style.contentVisibility === 'hidden')) return false;
            } catch (e) { /* 样式读不到时保留文本 */ }
            current = current.parentElement;
        }
        return true;
    }

    // 克隆前记录原节点的真实可见性：display:none / visibility:hidden 往往来自样式表，
    // 只看克隆节点的 inline style 会把隐藏答案一起带进草稿。
    function visibleClone(node) {
        var clone = node.cloneNode(true);
        var before = [node].concat(Array.prototype.slice.call(node.querySelectorAll('*')));
        var after = [clone].concat(Array.prototype.slice.call(clone.querySelectorAll('*')));
        for (var i = 0; i < before.length; i++) {
            if (!isVisible(before[i]) && after[i] && after[i].parentNode) {
                after[i].parentNode.removeChild(after[i]);
            }
        }
        return clone;
    }

    function cut(raw, max) {
        return String(raw || '').slice(0, max).trim();
    }

    function textOf(node, multiline) {
        if (!node) return '';
        var clone = visibleClone(node);
        replaceMath(clone);
        var junk = clone.querySelectorAll('script, style, noscript, [data-nb-btn], #nb-overlay-host');
        for (var i = 0; i < junk.length; i++) junk[i].remove();
        var text = clone.textContent || '';
        text = text.replace(/\u00a0/g, ' ').replace(/\r/g, '');
        if (multiline) {
            text = text.replace(/[ \t]+/g, ' ').replace(/\n\s*\n+/g, '\n').trim();
        } else {
            text = text.replace(/\s+/g, ' ').trim();
        }
        return text;
    }

    function pickAll(root, selectors) {
        for (var i = 0; i < selectors.length; i++) {
            try {
                var list = Array.prototype.slice.call(root.querySelectorAll(selectors[i])).filter(isVisible);
                if (list.length) return list;
            } catch (e) { /* 选择器不合法就换下一个 */ }
        }
        return [];
    }

    var OPTION_SEL = '.option, .options li, .choice, .answer-option, [class*="option"], li[class*="opt"]';
    var ANSWER_SEL = '.answer, .daan, .correct-answer, .right-answer, [class*="answer"]';
    var ANALYSIS_SEL = '.analysis, .explanation, .jiexi, .jx, [class*="analysis"], [class*="explain"]';
    var KNOWLEDGE_SEL = '.knowledge, .point, .kp, [class*="knowledge"], [class*="point"]';
    var DIFFICULTY_SEL = '.difficulty, [class*="difficulty"]';

    // ---------- 题块定位 ----------
    // 先按常见题库站点的容器 class 找；一个都命不中才退回「以题号开头」的启发式。
    var BLOCK_SELECTORS = [
        '.ques-item', '.question-item', '.questionItem', '.exam-question', '.subject-item',
        '.quesItem', '.questionLi', 'li.question', 'li.ques',
        '[data-questionid]', '[data-quesid]', '[data-qid]',
        '[class*="ques-item"]', '[class*="question-item"]', '[class*="questionItem"]',
        '[class*="quesItem"]', '[class*="exam-item"]', '[class*="paper-question"]'
    ];

    // 题块自己的文字，不带上注入的「收这题」按钮：否则重新扫页时
    // textContent 以「收这题」开头，题号判断会失败，整块被漏掉。
    function blockText(el) {
        if (!el) return '';
        var clone = visibleClone(el);
        var junk = clone.querySelectorAll('[data-nb-btn], #nb-overlay-host');
        for (var i = 0; i < junk.length; i++) junk[i].remove();
        return (clone.textContent || '').trim();
    }

    function looksLikeQuestion(el) {
        if (!el || el.nodeType !== 1) return false;
        if (el === document.body || el === document.documentElement) return false;
        if (el.closest && el.closest('#nb-overlay-host')) return false;
        var text = blockText(el);
        return text.length >= 12 && text.length <= 20000;
    }

    function dedupe(nodes) {
        var list = [];
        for (var i = 0; i < nodes.length; i++) {
            if (list.indexOf(nodes[i]) < 0) list.push(nodes[i]);
        }
        return list.filter(function(node) {
            var len = blockText(node).length;
            for (var i = 0; i < list.length; i++) {
                var other = list[i];
                if (other === node || !other.contains(node)) continue;
                var olen = blockText(other).length;
                // 外层容器和里面那道题字数差不多，说明是同一道题的两层壳，留外层。
                if (olen <= len * 1.35) return false;
            }
            return true;
        });
    }

    function heuristic() {
        var all = document.querySelectorAll('div, li, section, article');
        var hits = [];
        for (var i = 0; i < all.length && i < 3000; i++) {
            var el = all[i];
            if (!looksLikeQuestion(el)) continue;
            var text = blockText(el);
            if (!/^[（(]?\d{1,3}[.、．)）]/.test(text)) continue;
            // 子元素里有字数接近、又以题号开头的，说明它只是壳，交给里面那层。
            var nested = false;
            for (var j = 0; j < el.children.length; j++) {
                var child = el.children[j];
                var ct = blockText(child);
                if (ct.length >= text.length * 0.8 && /^[（(]?\d{1,3}[.、．)）]/.test(ct)) {
                    nested = true;
                    break;
                }
            }
            if (!nested) hits.push(el);
        }
        return dedupe(hits);
    }

    function findBlocks() {
        for (var i = 0; i < BLOCK_SELECTORS.length; i++) {
            var nodes;
            try {
                nodes = Array.prototype.slice.call(document.querySelectorAll(BLOCK_SELECTORS[i]))
                    .filter(looksLikeQuestion);
            } catch (e) { continue; }
            if (nodes.length) return dedupe(nodes);
        }
        return heuristic();
    }

    // ---------- 结构化字段 ----------
    function normalizeStem(raw) {
        var text = String(raw || '').trim();
        text = text.replace(/^[（(]?\d{1,3}[)）.．、]\s*/, '');
        text = text.replace(/^第\s*\d+\s*题[：:、.]?\s*/, '');
        return text.trim();
    }

    function parseOptionText(raw) {
        var match = raw.match(/^[（(]?([A-Ha-h])[)）.．、:：]\s*(.+)$/);
        if (!match) match = raw.match(/^([A-Ha-h])\s+(.+)$/);
        if (!match) return null;
        return {key: match[1].toUpperCase(), text: match[2].trim()};
    }

    function isOptionText(raw) {
        return parseOptionText(String(raw || '').trim()) !== null;
    }

    // 题干里内联字段的切分标记；答案 / 解析没换行时也靠它断开。
    var FIELD_MARK = /(?:答案|正确答案|参考答案|解析|详解|分析|点睛|知识点|考点|难度)\s*[：:]/;

    // 一个节点里塞着整行选项（`A. 甲 B. 乙 C. 丙`），按标记切开。
    // 选项文字本身可能带字母，所以标记前必须是行首、空白、标点或数字。
    function splitInlineOptions(raw) {
        var re = /([（(]?[A-Ha-h])[)）.．、:：]/g;
        var marks = [];
        var match;
        while ((match = re.exec(raw))) {
            var at = match.index;
            if (at > 0 && !/[\s。；;，,、）)0-9]/.test(raw.charAt(at - 1))) continue;
            marks.push({
                key: match[1].replace(/^[（(]/, '').toUpperCase(),
                start: at,
                body: at + match[0].length
            });
        }
        if (marks.length < 2) return null;
        // 最后一项后面常跟着「答案：…解析：…」，切掉再当选项文字。
        var tail = raw.slice(marks[0].start);
        var stop = tail.search(FIELD_MARK);
        if (stop > 0) raw = raw.slice(0, marks[0].start + stop);
        var items = [];
        for (var i = 0; i < marks.length; i++) {
            var end = i + 1 < marks.length ? marks[i + 1].start : raw.length;
            var text = raw.slice(marks[i].body, end).trim();
            if (!text) return null;
            items.push({key: marks[i].key, text: text});
        }
        return {items: items, prefix: raw.slice(0, marks[0].start).trim()};
    }

    function parseOptions(nodes) {
        var out = [];
        var next = 65;
        function push(key, text) {
            text = String(text || '').trim();
            if (!text) return;
            if (!key) key = String.fromCharCode(next);
            out.push({key: key, text: text});
            next = key.charCodeAt(0) + 1;
        }
        function takeText(raw) {
            if (!raw) return;
            var split = splitInlineOptions(raw);
            if (split) {
                for (var i = 0; i < split.items.length; i++) {
                    push(split.items[i].key, split.items[i].text);
                }
                return;
            }
            var single = parseOptionText(raw);
            if (single) push(single.key, single.text);
            else push('', raw);
        }
        function takeNode(node) {
            // 容器里直接排着选项子元素时按子元素拆：textContent 会把 `A. 2`、
            // `B. 3` 粘成 `A. 2B. 3`，整段正则切不准。
            var kids = Array.prototype.slice.call(node.children || []);
            if (kids.length >= 2) {
                var allOptions = kids.every(function(kid) {
                    var text = (kid.textContent || '').trim();
                    return text.length > 0 && isOptionText(text);
                });
                if (allOptions) {
                    for (var i = 0; i < kids.length; i++) takeText(textOf(kids[i], false));
                    return;
                }
            }
            takeText(textOf(node, false));
        }
        for (var i = 0; i < nodes.length; i++) takeNode(nodes[i]);
        return out.slice(0, MAX_OPTIONS).map(function(option) {
            return {key: cut(option.key, 8), text: cut(option.text, MAX_OPTION)};
        });
    }

    function splitNames(text) {
        return String(text || '')
            .replace(/^【?(?:知识点|考点)】?[：:]?\s*/, '')
            .split(/[、,，;；\/|]/)
            .map(function(item) { return item.trim(); })
            .filter(function(item) { return item.length > 0 && item.length < 40; })
            .slice(0, 6);
    }

    function extract(block) {
        var optNodes = pickAll(block, [OPTION_SEL]);
        // 选项容器常嵌套两层，去掉被别的选项节点包住的那些。
        optNodes = optNodes.filter(function(node) {
            return !optNodes.some(function(other) {
                return other !== node && other.contains(node);
            });
        });
        // 没有 class 的站点：<li> 只要像选项，凑够两个也收。
        if (!optNodes.length) {
            var lis = Array.prototype.slice.call(block.querySelectorAll('li'));
            var optionLis = lis.filter(function(li) {
                return isVisible(li) && isOptionText((li.textContent || '').trim());
            });
            if (optionLis.length >= 2) optNodes = optionLis;
        }
        var options = parseOptions(optNodes);

        function outsideOptions(node) {
            if (!node) return false;
            for (var i = 0; i < optNodes.length; i++) {
                // 选项里面、或者包着一串选项的外壳，都不该当字段。
                if (optNodes[i] === node || optNodes[i].contains(node) || node.contains(optNodes[i])) return true;
            }
            return false;
        }

        function field(selectors, names) {
            var list = pickAll(block, selectors);
            for (var i = 0; i < list.length; i++) {
                if (!outsideOptions(list[i])) return list[i];
            }
            // 没有 class 的站点：看子元素文字是不是以「答案：」这类标记开头。
            if (names) {
                var prefix = new RegExp('^【?(?:' + names + ')】?[：:]');
                var nodes = block.querySelectorAll('p, div, li, span, td, strong, b');
                for (var j = 0; j < nodes.length; j++) {
                    if (!isVisible(nodes[j]) || outsideOptions(nodes[j])) continue;
                    if (prefix.test((nodes[j].textContent || '').trim())) return nodes[j];
                }
            }
            return null;
        }

        var answerNode = field([ANSWER_SEL], '答案|正确答案|参考答案');
        var analysisNode = field([ANALYSIS_SEL], '解析|详解|分析|点睛');
        var knowledgeNode = field([KNOWLEDGE_SEL], '知识点|考点');
        var difficultyNode = field([DIFFICULTY_SEL], '难度');

        // 题干 = 整块去掉选项 / 答案 / 解析 / 知识点 / 难度后的部分。
        // 用临时属性标记再克隆，没 class 的站点也能剔干净；读完立刻恢复页面。
        var marked = [];
        function mark(node) {
            if (!node || node.hasAttribute('data-nb-drop')) return;
            node.setAttribute('data-nb-drop', '1');
            marked.push(node);
        }
        for (var m = 0; m < optNodes.length; m++) mark(optNodes[m]);
        mark(answerNode);
        mark(analysisNode);
        mark(knowledgeNode);
        mark(difficultyNode);
        var clone = visibleClone(block);
        var drop = clone.querySelectorAll('[data-nb-drop], [data-nb-btn], #nb-overlay-host');
        for (var i = 0; i < drop.length; i++) drop[i].remove();
        var stem = normalizeStem(textOf(clone, true));
        for (var r = 0; r < marked.length; r++) marked[r].removeAttribute('data-nb-drop');

        var combined = textOf(block, true);
        if (stem.length < 6) stem = normalizeStem(combined).slice(0, 1200);

        var answer = textOf(answerNode, true)
            .replace(/^【?(?:参考|正确)?答案】?[：:]\s*/, '')
            .split(FIELD_MARK)[0]
            .trim();
        var analysis = textOf(analysisNode, true)
            .replace(/^【?(?:解析|详解|分析|点睛)】?[：:]\s*/, '')
            .trim();
        var knowledge = splitNames(textOf(knowledgeNode, false));
        var difficulty = difficultyNode
            ? textOf(difficultyNode, false).replace(/^【?难度】?[：:]\s*/, '').trim()
            : '';

        if (!answer) {
            var answerMatch = combined.match(/(?:答案|正确答案|参考答案)[：:]\s*([\s\S]+)/);
            if (answerMatch) answer = answerMatch[1].split(FIELD_MARK)[0].trim();
        }
        if (!analysis) {
            var analysisMatch = combined.match(/(?:解析|详解|分析|点睛)[：:]\s*([\s\S]+)/);
            if (analysisMatch) analysis = analysisMatch[1].trim();
        }
        if (!difficulty) {
            var difficultyMatch = combined.match(/难度[：:]\s*(容易|简单|基础|中等|较难|难|高|\d)/);
            if (difficultyMatch) difficulty = difficultyMatch[1];
        }

        // 有些站点把选项直接写在题干段落里（没有单独的选项节点），从题干切一次。
        if (!options.length) {
            var inline = splitInlineOptions(stem);
            if (inline && inline.items.length >= 2) {
                options = inline.items;
                if (inline.prefix.length >= 4) stem = inline.prefix;
            }
        }

        var type = options.length ? 'SINGLE' : 'SOLVED';
        if (/多选|多选题|不定项/.test(combined)) type = 'MULTI';
        if (!options.length && /_{3,}|（\s*）|\(\s*\)/.test(stem)) type = 'FILL';

        var year = null;
        var yearMatch = combined.match(/(20\d{2})\s*年/);
        if (yearMatch) year = parseInt(yearMatch[1], 10);

        var warnings = [];
        if (stem.length > MAX_STEM) warnings.push('题干过长，已截断，请核对原文');
        if (answer.length > MAX_ANSWER) warnings.push('答案过长，已截断，请核对原文');
        if (analysis.length > MAX_ANALYSIS) warnings.push('解析过长，已截断，请核对原文');
        if (options.length >= MAX_OPTIONS) warnings.push('选项较多，请核对是否完整');
        var images = Array.prototype.slice.call(block.querySelectorAll('img')).filter(function(img) {
            return isVisible(img) && !!(img.currentSrc || img.getAttribute('src'));
        });
        if (images.length) warnings.push('含图片，请用截图补图');

        return {
            type: type,
            stem: cut(stem, MAX_STEM),
            options: options,
            answer: cut(answer, MAX_ANSWER),
            analysis: cut(analysis, MAX_ANALYSIS),
            difficulty: cut(difficulty, 32),
            knowledgeNames: knowledge.slice(0, 6),
            source: URL(),
            year: year,
            confidence: 0.92,
            warnings: warnings
        };
    }

    function pageText() {
        var selectors = ['article', 'main', '.question-list', '.content', '#content', '.main-content'];
        var root = null;
        for (var i = 0; i < selectors.length; i++) {
            var el = document.querySelector(selectors[i]);
            if (el && isVisible(el) && (el.textContent || '').trim().length > 80) {
                root = el;
                break;
            }
        }
        if (!root) root = document.body;
        return textOf(root, true).slice(0, 20000);
    }

    // ---------- 送题 ----------
    function reportError(error) {
        var message = String(error && error.message || error || '读取失败');
        showToast(message, 'err');
        // 复用已有的文字回传命令通知工具条失败，不扩大远程网页的命令权限。
        invoke('emit_browser_text', {text: '', error: message}).catch(function() {});
    }

    function send(items, allowPageFallback, total) {
        if (!items || !items.length) {
            if (!allowPageFallback) {
                return Promise.reject('这道题没认出题干，改用「识别本页题目」或截图');
            }
            var text = pageText();
            if (!text || text.length < 20) {
                return Promise.reject('页面里没找到题目文字，可能是整页图片或被登录墙挡住了');
            }
            return invoke('emit_browser_text', { text: text }).then(function() {
                showToast('没找到结构化题块，整页文字已填入录入页，请检查后手动整理', 'info');
            });
        }
        return invoke('browser_collect', { items: items }).then(function(result) {
            var count = (result && result.count) || items.length;
            if ((result && result.truncated) || (total && total > items.length)) {
                showToast('本页有 ' + (total || result.total) + ' 道题，已收前 ' + count + ' 道，请翻页后继续', 'info');
            } else {
                showToast('✓ 已收到 ' + count + ' 道题，回主窗右侧核对', 'ok');
            }
        });
    }

    // ---------- 浮动按钮 ----------
    var host = document.createElement('div');
    host.id = 'nb-overlay-host';
    host.style.cssText = 'position:fixed;bottom:28px;right:28px;z-index:2147483647;pointer-events:none;';
    var shadow = host.attachShadow({ mode: 'open' });
    document.documentElement.appendChild(host);

    var style = document.createElement('style');
    style.textContent = [
        '* { box-sizing: border-box; margin: 0; padding: 0; }',
        '.nb-wrap { pointer-events: auto; display: flex; flex-direction: column; align-items: flex-end; gap: 10px; font-family: -apple-system, "PingFang SC", "Segoe UI", sans-serif; }',
        '.nb-btn {',
        '  display: flex; align-items: center; gap: 8px;',
        '  padding: 12px 22px; border: none; border-radius: 50px;',
        '  background: linear-gradient(135deg, #4f6ef7 0%, #7c5cf0 100%);',
        '  color: #fff; font-size: 14px; font-weight: 600; cursor: pointer;',
        '  box-shadow: 0 4px 20px rgba(79,110,247,.35), 0 1px 3px rgba(0,0,0,.12);',
        '  transition: transform .15s, box-shadow .15s;',
        '  user-select: none; -webkit-user-select: none;',
        '}',
        '.nb-btn:hover { transform: translateY(-1px); box-shadow: 0 6px 28px rgba(79,110,247,.45), 0 2px 6px rgba(0,0,0,.15); }',
        '.nb-btn:active { transform: scale(.97); }',
        '.nb-btn.busy { opacity: .75; pointer-events: none; }',
        '.nb-btn .nb-icon { font-size: 18px; line-height: 1; }',
        '.nb-btn .nb-spin { display: none; width: 16px; height: 16px; border: 2.5px solid rgba(255,255,255,.35); border-top-color: #fff; border-radius: 50%; animation: nb-rot .6s linear infinite; }',
        '.nb-btn.busy .nb-spin { display: block; }',
        '.nb-btn.busy .nb-icon { display: none; }',
        '@keyframes nb-rot { to { transform: rotate(360deg); } }',
        '.nb-toast {',
        '  padding: 10px 18px; border-radius: 12px; font-size: 13px; font-weight: 500;',
        '  color: #fff; max-width: 320px; line-height: 1.5;',
        '  box-shadow: 0 4px 20px rgba(0,0,0,.18);',
        '  opacity: 0; transform: translateY(8px); transition: opacity .25s, transform .25s;',
        '  pointer-events: none;',
        '}',
        '.nb-toast.show { opacity: 1; transform: translateY(0); }',
        '.nb-toast.ok { background: #22a06b; }',
        '.nb-toast.err { background: #e5484d; }',
        '.nb-toast.info { background: #3b82f6; }',
    ].join('\n');
    shadow.appendChild(style);

    var wrap = document.createElement('div');
    wrap.className = 'nb-wrap';

    var toast = document.createElement('div');
    toast.className = 'nb-toast';
    wrap.appendChild(toast);

    var mainBtn = document.createElement('button');
    mainBtn.className = 'nb-btn';
    mainBtn.type = 'button';
    mainBtn.innerHTML = '<span class="nb-icon">📝</span><span class="nb-spin"></span><span class="nb-label">识别本页题目</span>';
    wrap.appendChild(mainBtn);
    shadow.appendChild(wrap);

    var mainLabel = mainBtn.querySelector('.nb-label');

    var toastTimer = null;
    function showToast(message, type) {
        toast.textContent = message;
        toast.className = 'nb-toast show ' + (type || 'info');
        clearTimeout(toastTimer);
        toastTimer = setTimeout(function() { toast.className = 'nb-toast'; }, 3600);
    }

    function setBusy(value, label) {
        if (value) {
            mainBtn.classList.add('busy');
            mainLabel.textContent = label || '识别中…';
        } else {
            mainBtn.classList.remove('busy');
            updateLabel();
        }
    }

    var blockCount = 0;
    function updateLabel() {
        if (mainBtn.classList.contains('busy')) return;
        mainLabel.textContent = blockCount > MAX_ITEMS
            ? '读取前 ' + MAX_ITEMS + ' 题（共 ' + blockCount + ' 题）'
            : (blockCount > 0 ? '识别本页 ' + blockCount + ' 题' : '读取整页题目');
    }

    function collectAll() {
        var blocks = findBlocks();
        var total = blocks.length;
        blocks = blocks.slice(0, MAX_ITEMS);
        var items = [];
        for (var i = 0; i < blocks.length; i++) items.push(extract(blocks[i]));
        return send(items, true, total)
            .catch(reportError)
            .then(function() { setBusy(false); });
    }

    mainBtn.addEventListener('click', function() {
        if (mainBtn.classList.contains('busy')) return;
        setBusy(true);
        collectAll();
    });

    // ---------- 逐题按钮 ----------
    function injectOneButton(block) {
        var btn = document.createElement('button');
        btn.setAttribute('data-nb-btn', '1');
        btn.type = 'button';
        btn.textContent = '收这题';
        btn.style.cssText = 'display:block;margin:0 0 6px auto;padding:2px 10px;border:0;border-radius:6px;background:#4f6ef7;color:#fff;font-size:12px;line-height:20px;cursor:pointer;font-family:-apple-system,"PingFang SC",sans-serif;';
        btn.addEventListener('click', function(event) {
            event.preventDefault();
            event.stopPropagation();
            if (btn.disabled) return;
            var item = extract(block);
            if (!item || !item.stem || item.stem.length < 4) {
                showToast('这道题没认出题干，改用「识别本页题目」或截图', 'err');
                return;
            }
            btn.disabled = true;
            btn.textContent = '收取中…';
            send([item])
                .then(function() { btn.textContent = '✓ 已收'; })
                .catch(reportError)
                .then(function() {
                    setTimeout(function() {
                        btn.disabled = false;
                        btn.textContent = '收这题';
                    }, 2200);
                });
        });
        block.insertBefore(btn, block.firstChild);
    }

    var scanTimer = null;
    function scan() {
        var blocks = findBlocks();
        blockCount = blocks.length;
        for (var i = 0; i < blocks.length; i++) {
            var block = blocks[i];
            if (block.getAttribute('data-nb-block') === '1') continue;
            block.setAttribute('data-nb-block', '1');
            injectOneButton(block);
        }
        updateLabel();
    }

    function scheduleScan() {
        if (scanTimer) clearTimeout(scanTimer);
        scanTimer = setTimeout(scan, 700);
    }

    // 供工具条 / Rust 主动触发：返回结构化结果，不直接送。
    window.__nbExtractAll = function() {
        var blocks = findBlocks().slice(0, MAX_ITEMS);
        return blocks.map(function(block) { return extract(block); });
    };
    // 工具条「识别本页」与 Rust 命令都走这里。
    window.__nbCollectAll = function() {
        if (mainBtn.classList.contains('busy')) return;
        setBusy(true);
        collectAll();
    };

    try {
        var observer = new MutationObserver(function() { scheduleScan(); });
        observer.observe(document.body, { childList: true, subtree: true });
    } catch (e) { /* 有些页面没有 body，直接扫一次 */ }

    scheduleScan();
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urls_only_allow_http_and_https() {
        assert_eq!(
            normalize_url("example.com/a").unwrap(),
            "https://example.com/a"
        );
        assert!(normalize_url("file:///tmp/questions.html").is_err());
        assert!(normalize_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn web_items_are_capped_and_source_comes_from_window() {
        let items: Vec<_> = (0..65)
            .map(|index| json!({"stem": format!("第 {index} 题"), "source": "https://fake.test"}))
            .collect();
        let prepared = prepare_web_items(items, "https://real.test/current").unwrap();
        assert_eq!(prepared.total, 65);
        assert_eq!(prepared.items.len(), MAX_WEB_ITEMS);
        assert!(prepared.truncated);
        assert!(prepared
            .items
            .iter()
            .all(|item| { item["source"].as_str() == Some("https://real.test/current") }));
    }

    #[test]
    fn web_fields_and_options_are_bounded_without_splitting_unicode() {
        let options: Vec<_> = (0..15)
            .map(|index| json!({"key": format!("OPTION{index}"), "text": "选".repeat(MAX_OPTION_CHARS + 3)}))
            .collect();
        let item = json!({
            "stem": "题".repeat(MAX_STEM_CHARS + 5),
            "answer": "答".repeat(MAX_ANSWER_CHARS + 5),
            "analysis": "析".repeat(MAX_ANALYSIS_CHARS + 5),
            "options": options,
            "warnings": ["含图片，请用截图补图"]
        });
        let prepared = prepare_web_items(vec![item], "https://real.test/").unwrap();
        let item = &prepared.items[0];
        assert_eq!(
            item["stem"].as_str().unwrap().chars().count(),
            MAX_STEM_CHARS
        );
        assert_eq!(
            item["answer"].as_str().unwrap().chars().count(),
            MAX_ANSWER_CHARS
        );
        assert_eq!(
            item["analysis"].as_str().unwrap().chars().count(),
            MAX_ANALYSIS_CHARS
        );
        assert_eq!(item["options"].as_array().unwrap().len(), MAX_OPTIONS);
        assert_eq!(
            item["options"][0]["text"].as_str().unwrap().chars().count(),
            MAX_OPTION_CHARS
        );
        let warnings = item["warnings"].as_array().unwrap();
        assert!(warnings
            .iter()
            .any(|value| value.as_str().unwrap_or("").contains("题干过长")));
        assert!(warnings
            .iter()
            .any(|value| value.as_str().unwrap_or("").contains("含图片")));
    }
}
