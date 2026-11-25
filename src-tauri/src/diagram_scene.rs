//! 制图场景渲染：模型只负责「画什么」，本地代码负责「怎么画」。
//!
//! 直接让模型吐整张 SVG，问题不在格式（那些还能修），而在几何一致性：它会漏画棱、
//! 给题干里没有的点标号、把标签压在线段上、把顶点画在底面某个角正上方。这些都是
//! 「生成一段文本」没法自我检查的部分，靠提示词反复叮嘱也治不好。
//!
//! 所以制图拆成两段：模型输出一个坐标化的场景 JSON（0–100 归一化坐标系），
//! 这里把它渲染成 SVG。好处是实打实的：
//! 1) 输出只有原来的几分之一，不会再写到一半被 max_tokens 截断；
//! 2) 标签位置由布局算法算出来，自动避让线段和其它文字——不再有压线、重叠、错位；
//! 3) 引用了不存在的点、坐标越界、线段退化，渲染前就能发现，直接让模型重画。
//!
//! 场景 JSON 形状（全部字段除 points 外都可省）：
//! ```json
//! {
//!   "title": "四棱锥 P-ABCD",
//!   "points":  [{"id":"A","x":22,"y":78},{"id":"P","x":50,"y":12}],
//!   "faces":   [{"points":["A","B","C","D"]}],
//!   "segments":[{"a":"P","b":"A"},{"a":"P","b":"C","dashed":true}],
//!   "curves":  [{"points":[[10,50],[50,20],[90,50]],"dashed":false}],
//!   "circles": [{"x":50,"y":50,"r":12}],
//!   "marks":   [{"type":"right","at":"A","a":"B","b":"D"}],
//!   "labels":  [{"x":50,"y":96,"text":"图1"}]
//! }
//! ```

use serde_json::Value;
use std::collections::HashMap;

const WIDTH: f64 = 480.0;
const PAD: f64 = 46.0;
const MIN_HEIGHT: f64 = 250.0;
const MAX_HEIGHT: f64 = 430.0;
const LINE: &str = "#22303c";
const HIDDEN: &str = "#8c98a4";
const ACCENT: &str = "#c0392b";
/// 多个面叠在一起时用不同深浅区分，否则截面和底面糊成一块。
const FACE_TINTS: [&str; 3] = ["#e8f0fb", "#dce8f7", "#f2f6fc"];
const FONT: &str = "sans-serif";
const LABEL_SIZE: f64 = 15.0;

struct Node {
    id: String,
    x: f64,
    y: f64,
    label: Option<String>,
    dot: bool,
}

struct Segment {
    a: usize,
    b: usize,
    dashed: bool,
    color: String,
    arrow: bool,
}

struct Curve {
    points: Vec<(f64, f64)>,
    dashed: bool,
    color: String,
    arrow: bool,
}

struct Circle {
    x: f64,
    y: f64,
    r: f64,
    dashed: bool,
}

struct Face {
    points: Vec<usize>,
    fill: Option<String>,
}

struct Mark {
    at: usize,
    a: usize,
    b: usize,
}

/// 归一化坐标 → 画布像素：等比缩放后居中，四周留出放标签的空白。
struct Fit {
    scale: f64,
    ox: f64,
    oy: f64,
    height: f64,
}

impl Fit {
    fn px(&self, x: f64, y: f64) -> (f64, f64) {
        (self.ox + x * self.scale, self.oy + y * self.scale)
    }
}

/// 把场景渲染成完整 SVG。返回的字符串已经可以直接塞进 data URL。
pub fn render(scene: &Value) -> Result<String, String> {
    let nodes = parse_points(scene)?;
    if nodes.is_empty() {
        return Err("场景里没有 points，画不出图".into());
    }
    let index = build_index(&nodes);

    // 单个元素引用了不存在的点（模型漏写点，或输出被截断）不该让整张图报废：
    // 记下问题、跳过它继续画；只有剩下能画的东西太少时才判失败，让模型重画。
    let mut problems: Vec<String> = Vec::new();
    let faces = parse_faces(scene, &index, &mut problems);
    let mut segments = parse_segments(scene, &index, &mut problems);
    let curves = parse_curves(scene)?;
    let circles = parse_circles(scene)?;
    let marks = parse_marks(scene, &index, &mut problems);
    let fixed_labels = parse_labels(scene);
    // 直四棱柱 A-B-C-D-A1B1C1D1 是立体几何里最常见的图，也是模型最容易画歪的图：
    // C1 忘了上移、漏一条侧棱、把右侧棱画成虚线。这里按「顶面 = 底面向上平移」
    // 统一校正一遍，再补齐 12 条棱，免得一张图因为一个坐标错就全废。
    let mut nodes = nodes;
    normalize_prism(&mut nodes, &mut segments);
    let title = scene
        .get("title")
        .and_then(|node| node.as_str())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());

    let drawable = faces.len() + segments.len() + curves.len() + circles.len();
    if drawable == 0 {
        if problems.is_empty() {
            return Err("场景里只有点，没有线、面或曲线；至少给出 segments 或 curves".into());
        }
        return Err(problems.join("；"));
    }
    // 只剩一两条线时图已经没意义了，退回让模型重画，比给用户一张残缺图好。
    if drawable < 2 && !problems.is_empty() {
        return Err(problems.join("；"));
    }

    let fit = fit_view(
        &nodes,
        &curves,
        &circles,
        &fixed_labels,
        if title.is_some() { PAD + 22.0 } else { PAD },
    );
    let height = fit.height;

    let mut svg = String::with_capacity(4096);
    // 宽高给**具体数字**（不是 100%）：这张图是当 <img src="data:..."> 用的，
    // 只给 viewBox + width="100%" 时浏览器没有固有尺寸，容易算成 0 高或默认 300×150，
    // 预览框里就只剩一条白边。具体像素 + CSS 的 max-width:100% 缩放才是可靠组合。
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {WIDTH:.0} {height:.0}\" \
         width=\"{WIDTH:.0}\" height=\"{height:.0}\" preserveAspectRatio=\"xMidYMid meet\">"
    ));
    svg.push_str(&format!(
        "<rect width=\"{WIDTH:.0}\" height=\"{height:.0}\" fill=\"#ffffff\"/>"
    ));
    if let Some(title) = &title {
        svg.push_str(&format!(
            "<text x=\"{:.0}\" y=\"26\" font-family=\"{FONT}\" font-size=\"14\" fill=\"#5b6773\">{}</text>",
            20.0,
            escape(title)
        ));
    }

    // 面先铺底，线段再压上去：模型常把底面写成 face + 4 条 segment，这样不会出现双线。
    for (order, face) in faces.iter().enumerate() {
        let fill = face.fill.clone().unwrap_or_else(|| FACE_TINTS[order % FACE_TINTS.len()].to_string());
        let pts: Vec<String> = face
            .points
            .iter()
            .map(|&i| {
                let (x, y) = fit.px(nodes[i].x, nodes[i].y);
                format!("{x:.1},{y:.1}")
            })
            .collect();
        svg.push_str(&format!(
            "<polygon points=\"{}\" fill=\"{}\" stroke=\"none\"/>",
            pts.join(" "),
            fill
        ));
    }

    for curve in &curves {
        if curve.points.len() < 2 {
            continue;
        }
        let pts: Vec<String> = curve
            .points
            .iter()
            .map(|&(x, y)| {
                let (px, py) = fit.px(x, y);
                format!("{px:.1},{py:.1}")
            })
            .collect();
        svg.push_str(&polyline(&pts.join(" "), curve.color.as_str(), curve.dashed));
        if curve.arrow {
            if let (Some(&from), Some(&to)) = (curve.points.get(curve.points.len() - 2), curve.points.last()) {
                let a = fit.px(from.0, from.1);
                let b = fit.px(to.0, to.1);
                svg.push_str(&arrow_head(a, b, curve.color.as_str()));
            }
        }
    }

    for circle in &circles {
        let (cx, cy) = fit.px(circle.x, circle.y);
        let r = (circle.r * fit.scale).max(2.0);
        svg.push_str(&format!(
            "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"none\" stroke=\"{LINE}\" \
             stroke-width=\"1.8\"{}/>",
            if circle.dashed {
                " stroke-dasharray=\"6,4\""
            } else {
                ""
            }
        ));
    }

    for segment in &segments {
        let a = fit.px(nodes[segment.a].x, nodes[segment.a].y);
        let b = fit.px(nodes[segment.b].x, nodes[segment.b].y);
        svg.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"{}\" stroke-width=\"1.8\" \
             stroke-linecap=\"round\"{}/>",
            a.0,
            a.1,
            b.0,
            b.1,
            segment.color,
            if segment.dashed {
                " stroke-dasharray=\"6,4\""
            } else {
                ""
            }
        ));
        if segment.arrow {
            svg.push_str(&arrow_head(a, b, segment.color.as_str()));
        }
    }

    // 直角标记：题干里的「⊥」在图上看得见，比只在文字里说更可信。
    for mark in &marks {
        let p = fit.px(nodes[mark.at].x, nodes[mark.at].y);
        let a = fit.px(nodes[mark.a].x, nodes[mark.a].y);
        let b = fit.px(nodes[mark.b].x, nodes[mark.b].y);
        if let Some(path) = right_angle(p, a, b) {
            svg.push_str(&format!(
                "<polyline points=\"{path}\" fill=\"none\" stroke=\"{LINE}\" stroke-width=\"1.4\"/>"
            ));
        }
    }

    for node in &nodes {
        if !node.dot {
            continue;
        }
        let (x, y) = fit.px(node.x, node.y);
        svg.push_str(&format!("<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"2.6\" fill=\"{LINE}\"/>"));
    }

    // 标注统一由布局算法摆放：先按「背离图形中心」定方向，再逐个试候选位置避让。
    let mut placed_labels: Vec<Rect> = Vec::new();
    let labels = layout_labels(&nodes, &segments, &curves, &fit, &mut placed_labels);
    for label in &labels {
        svg.push_str(&format!(
            "<text x=\"{:.1}\" y=\"{:.1}\" font-family=\"{FONT}\" font-size=\"{LABEL_SIZE:.0}\" \
             fill=\"{LINE}\" text-anchor=\"middle\" dominant-baseline=\"central\">{}</text>",
            label.0,
            label.1,
            escape(&label.2)
        ));
    }
    // 模型常常一边给 points 自动上标、一边又把同一个点名写进 labels，
    // 直接画就变成 `P P`、`D D` 这种重影。凡是和已摆好的点名同字的自由标注一律丢掉。
    // 比较前先归一化（A1 / A₁ / a_1 视为同一个），否则下标写法不同就漏过去。
    let mut placed: Vec<String> = labels.iter().map(|label| label_key(&label.2)).collect();
    for (x, y, text) in &fixed_labels {
        let text = text.trim();
        let key = label_key(text);
        if text.is_empty() || placed.iter().any(|name| name == &key) {
            continue;
        }
        placed.push(key);
        // 数值标注的位置是模型给的，经常会压在点名上（例如 ∠BAD=60° 正好盖住 D）。
        // 在附近试几个位置，找不到不重叠的就干脆不画——少一条文字比糊成一团好。
        let Some((px, py)) = place_fixed_label(*x, *y, text, &fit, &mut placed_labels) else {
            continue;
        };
        svg.push_str(&format!(
            "<text x=\"{px:.1}\" y=\"{py:.1}\" font-family=\"{FONT}\" font-size=\"13\" \
             fill=\"#5b6773\" text-anchor=\"middle\">{}</text>",
            escape(text)
        ));
    }

    svg.push_str("</svg>");
    Ok(svg)
}

/// 解析 points。支持两种写法：
/// - 直接给坐标：`{"id":"A","x":20,"y":80}`；
/// - 中点：`{"id":"E","midpoint":["B","C"]}`（坐标由本地算，模型不用自己求平均）。
///
/// 中点值得单独支持：模型算 A₁D 中点时经常写成 A₁D₁ 的中点，图上看不出来但整道题
/// 就错了。让它只声明关系、由这里求值，比在提示词里反复叮嘱「要真算」可靠。
fn parse_points(scene: &Value) -> Result<Vec<Node>, String> {
    let Some(list) = scene.get("points").and_then(|node| node.as_array()) else {
        return Ok(Vec::new());
    };

    struct Raw {
        id: String,
        x: Option<f64>,
        y: Option<f64>,
        midpoint: Option<(String, String)>,
        label: Option<String>,
        dot: bool,
    }

    let mut raws: Vec<Raw> = Vec::new();
    for (position, item) in list.iter().enumerate() {
        let id = item
            .get("id")
            .or_else(|| item.get("name"))
            .and_then(|node| node.as_str())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .or_else(|| {
                item.get("label")
                    .and_then(|node| node.as_str())
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty())
            });
        let Some(id) = id else {
            return Err(format!("第 {} 个点没有 id 或 label", position + 1));
        };
        let midpoint = item
            .get("midpoint")
            .and_then(|node| node.as_array())
            .and_then(|pair| {
                let a = pair.first()?.as_str()?.trim().to_string();
                let b = pair.get(1)?.as_str()?.trim().to_string();
                if a.is_empty() || b.is_empty() {
                    None
                } else {
                    Some((a, b))
                }
            });
        let x = number(item.get("x"));
        let y = number(item.get("y"));
        if midpoint.is_none() {
            let (Some(x), Some(y)) = (x, y) else {
                return Err(format!("点 {id} 缺少 x/y 坐标，或写成 midpoint 形式"));
            };
            if !x.is_finite() || !y.is_finite() {
                return Err(format!("点 {id} 的坐标不是有效数字"));
            }
        }
        // 标注规则：显式 label 优先；`"label": ""` 或 hideLabel 表示不标；
        // 其余只给「短名字」（A、B、P、O、A1 这种）上标——模型常造 Og/On/Of
        // 这类箭头端点当内部锚点，全标出来就变成一堆噪音。
        let explicit = item
            .get("label")
            .and_then(|node| node.as_str())
            .map(|text| text.trim().to_string());
        let hidden = item
            .get("hideLabel")
            .and_then(|node| node.as_bool())
            .unwrap_or(false);
        let label = match explicit {
            Some(text) if !text.is_empty() => Some(text),
            Some(_) => None,
            None if hidden => None,
            None => point_label(&id),
        };
        let dot = item.get("dot").and_then(|node| node.as_bool()).unwrap_or(false);
        raws.push(Raw {
            id,
            x,
            y,
            midpoint,
            label,
            dot,
        });
    }

    let mut resolved: Vec<Option<Node>> = raws.iter().map(|_| None).collect();
    loop {
        let mut progress = false;
        let known: HashMap<String, (f64, f64)> = raws
            .iter()
            .zip(resolved.iter())
            .filter_map(|(raw, node)| node.as_ref().map(|node| (raw.id.clone(), (node.x, node.y))))
            .collect();
        for (position, raw) in raws.iter().enumerate() {
            if resolved[position].is_some() {
                continue;
            }
            let (x, y) = match &raw.midpoint {
                Some((a, b)) => match (known.get(a), known.get(b)) {
                    (Some(pa), Some(pb)) => ((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0),
                    _ => continue,
                },
                None => match (raw.x, raw.y) {
                    (Some(x), Some(y)) => (x, y),
                    _ => continue,
                },
            };
            resolved[position] = Some(Node {
                id: raw.id.clone(),
                x: clamp(x, 5.0, 95.0),
                y: clamp(y, 5.0, 95.0),
                label: raw.label.clone(),
                dot: raw.dot,
            });
            progress = true;
        }
        if !progress {
            break;
        }
    }

    let mut nodes = Vec::new();
    for (raw, node) in raws.into_iter().zip(resolved) {
        match node {
            Some(node) => nodes.push(node),
            None => {
                return Err(match &raw.midpoint {
                    Some((a, b)) => format!("点 {} 的中点引用了不存在的点（{a}、{b}）", raw.id),
                    None => format!("点 {} 缺少坐标", raw.id),
                })
            }
        }
    }
    Ok(nodes)
}

/// 自动标注用的显示名：A1 → A₁，P_2 → P₂。不是「点名」时返回 None。
/// 两个字母以上且没有数字的多半是模型自造的辅助点（Og、On、Of），不自动上标。
fn point_label(id: &str) -> Option<String> {
    let id = id.trim();
    let chars: Vec<char> = id.chars().collect();
    if chars.len() == 1 {
        return chars[0].is_alphabetic().then(|| id.to_string());
    }
    let split = chars
        .iter()
        .position(|ch| ch.is_ascii_digit() || *ch == '_')
        .unwrap_or(chars.len());
    if split == 0 || split == chars.len() {
        return None;
    }
    let head: String = chars[..split].iter().collect();
    if !head.chars().all(|ch| ch.is_alphabetic()) {
        return None;
    }
    let tail: String = chars[split..].iter().collect();
    let tail = tail.trim_start_matches('_');
    if tail.is_empty() || !tail.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let mut out = head;
    for ch in tail.chars() {
        out.push(match ch {
            '0' => '₀',
            '1' => '₁',
            '2' => '₂',
            '3' => '₃',
            '4' => '₄',
            '5' => '₅',
            '6' => '₆',
            '7' => '₇',
            '8' => '₈',
            '9' => '₉',
            _ => ch,
        });
    }
    Some(out)
}

/// 比较点名用的归一化：A1、A₁、a_1 视为同一个，避免「A₁ A1」重影。
fn label_key(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.trim().chars() {
        if ch.is_whitespace() || ch == '_' || ch == '{' || ch == '}' {
            continue;
        }
        let mapped = match ch {
            '₀' => '0',
            '₁' => '1',
            '₂' => '2',
            '₃' => '3',
            '₄' => '4',
            '₅' => '5',
            '₆' => '6',
            '₇' => '7',
            '₈' => '8',
            '₉' => '9',
            other => other,
        };
        out.push(mapped);
    }
    out
}

fn build_index(nodes: &[Node]) -> HashMap<String, usize> {
    let mut index = HashMap::new();
    for (position, node) in nodes.iter().enumerate() {
        index.entry(node.id.clone()).or_insert(position);
    }
    index
}

fn lookup(index: &HashMap<String, usize>, value: Option<&Value>, what: &str) -> Result<usize, String> {
    let raw = value
        .and_then(|node| node.as_str())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("{what} 缺少点 id"))?;
    index
        .get(&raw)
        .copied()
        .ok_or_else(|| format!("{what} 引用了不存在的点「{raw}」，points 里没有这个点"))
}

fn parse_faces(scene: &Value, index: &HashMap<String, usize>, problems: &mut Vec<String>) -> Vec<Face> {
    let Some(list) = scene.get("faces").and_then(|node| node.as_array()) else {
        return Vec::new();
    };
    let mut faces = Vec::new();
    for item in list {
        let Some(ids) = item.get("points").and_then(|node| node.as_array()) else {
            continue;
        };
        let mut points = Vec::new();
        let mut broken = false;
        for value in ids {
            match lookup(index, Some(value), "面") {
                Ok(point) => points.push(point),
                Err(error) => {
                    // 面缺一个顶点就会变成错误的三角形，宁可整面不画。
                    problems.push(error);
                    broken = true;
                }
            }
        }
        if broken || points.len() < 3 {
            continue;
        }
        let fill = item
            .get("fill")
            .and_then(|node| node.as_str())
            .filter(|text| valid_color(text))
            .map(|text| text.to_string());
        faces.push(Face { points, fill });
    }
    faces
}

/// 直四棱柱的确定性校正：顶面必须由底面向上平移，12 条棱一条不少。
///
/// 只认最常见的命名（底面 A、B、C、D，顶面 A1、B1、C1、D1）；对不上就原样返回，
/// 不碰其它图形。虚线按「从右前上方看」的可见性重算：后底边、后左底边、后左侧棱
/// 被挡住，其余实线——比模型自己标注可靠。
fn normalize_prism(nodes: &mut [Node], segments: &mut Vec<Segment>) {
    let index = build_index(nodes);
    let base_ids = ["A", "B", "C", "D"];
    let mut base: Vec<usize> = Vec::new();
    let mut top: Vec<usize> = Vec::new();
    for id in base_ids {
        let Some(&b) = index.get(id) else {
            return;
        };
        let Some(&t) = index.get(&format!("{id}1")) else {
            return;
        };
        base.push(b);
        top.push(t);
    }

    // 侧棱高度取各对的中位数：个别点（常见是 C1）漏了平移，不该把整张图带偏。
    let mut heights: Vec<f64> = base
        .iter()
        .zip(top.iter())
        .map(|(&b, &t)| nodes[b].y - nodes[t].y)
        .filter(|height| *height > 2.0)
        .collect();
    if heights.len() < 3 {
        return;
    }
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let height = heights[heights.len() / 2];
    for (&b, &t) in base.iter().zip(top.iter()) {
        nodes[t].x = nodes[b].x;
        nodes[t].y = nodes[b].y - height;
    }

    // 观察方向固定为右前上方：y 越小越靠后。后排两个点里 x 小的那个是「后左」，
    // 它的竖棱和两条后底边被挡住。
    let mut order: Vec<usize> = base.clone();
    order.sort_by(|&a, &b| {
        nodes[a]
            .y
            .partial_cmp(&nodes[b].y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(nodes[a].x.partial_cmp(&nodes[b].x).unwrap_or(std::cmp::Ordering::Equal))
    });
    let back = &order[..2];
    let front = &order[2..];
    let back_left = if nodes[back[0]].x <= nodes[back[1]].x { back[0] } else { back[1] };
    let front_left = if nodes[front[0]].x <= nodes[front[1]].x { front[0] } else { front[1] };
    let top_of = |point: usize| {
        base.iter()
            .position(|&item| item == point)
            .map(|position| top[position])
    };

    let pairs = |a: usize, b: usize| -> (usize, usize) {
        if a <= b {
            (a, b)
        } else {
            (b, a)
        }
    };
    let mut hidden: Vec<(usize, usize)> = vec![pairs(back[0], back[1]), pairs(back_left, front_left)];
    if let Some(t) = top_of(back_left) {
        hidden.push(pairs(back_left, t));
    }

    let mut ensure = |a: usize, b: usize| {
        let dashed = hidden.contains(&pairs(a, b));
        if let Some(existing) = segments
            .iter_mut()
            .find(|segment| pairs(segment.a, segment.b) == pairs(a, b))
        {
            existing.dashed = dashed;
            return;
        }
        segments.push(Segment {
            a,
            b,
            dashed,
            color: if dashed { HIDDEN } else { LINE }.to_string(),
            arrow: false,
        });
    };

    // 底面四条、顶面四条、侧棱四条。
    for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
        ensure(base[a], base[b]);
        ensure(top[a], top[b]);
        ensure(base[a], top[a]);
    }
}

fn parse_segments(scene: &Value, index: &HashMap<String, usize>, problems: &mut Vec<String>) -> Vec<Segment> {
    let Some(list) = scene.get("segments").and_then(|node| node.as_array()) else {
        return Vec::new();
    };
    let mut segments: Vec<Segment> = Vec::new();
    for item in list {
        let a = match lookup(index, item.get("a").or_else(|| item.get("from")), "线段") {
            Ok(point) => point,
            Err(error) => {
                problems.push(error);
                continue;
            }
        };
        let b = match lookup(index, item.get("b").or_else(|| item.get("to")), "线段") {
            Ok(point) => point,
            Err(error) => {
                problems.push(error);
                continue;
            }
        };
        if a == b {
            continue;
        }
        let dashed = item
            .get("dashed")
            .and_then(|node| node.as_bool())
            .or_else(|| item.get("style").and_then(|node| node.as_str()).map(|s| s == "dashed"))
            .unwrap_or(false);
        let color = color_of(item, if dashed { HIDDEN } else { LINE });
        let arrow = item.get("arrow").and_then(|node| node.as_bool()).unwrap_or(false);
        // 同一段线模型常写两遍（A→M 与 M→A），画两次线会变粗。合并成一条；
        // 只要有一次标了 dashed，就按虚线算，宁可画虚线也别把被挡住的棱画实。
        if let Some(existing) = segments.iter_mut().find(|segment| {
            (segment.a == a && segment.b == b) || (segment.a == b && segment.b == a)
        }) {
            existing.dashed = existing.dashed || dashed;
            existing.arrow = existing.arrow || arrow;
            continue;
        }
        segments.push(Segment {
            a,
            b,
            dashed,
            color,
            arrow,
        });
    }
    segments
}

fn parse_curves(scene: &Value) -> Result<Vec<Curve>, String> {
    let Some(list) = scene.get("curves").and_then(|node| node.as_array()) else {
        return Ok(Vec::new());
    };
    let mut curves = Vec::new();
    for item in list {
        let Some(raw) = item.get("points").and_then(|node| node.as_array()) else {
            continue;
        };
        let mut points = Vec::new();
        for entry in raw {
            let pair = if let Some(array) = entry.as_array() {
                (
                    array.first().and_then(|node| number(Some(node))),
                    array.get(1).and_then(|node| number(Some(node))),
                )
            } else {
                (number(entry.get("x")), number(entry.get("y")))
            };
            if let (Some(x), Some(y)) = pair {
                if x.is_finite() && y.is_finite() {
                    points.push((clamp(x, -40.0, 140.0), clamp(y, -40.0, 140.0)));
                }
            }
        }
        if points.len() < 2 {
            continue;
        }
        let dashed = item.get("dashed").and_then(|node| node.as_bool()).unwrap_or(false);
        let color = color_of(item, ACCENT);
        let arrow = item.get("arrow").and_then(|node| node.as_bool()).unwrap_or(false);
        curves.push(Curve {
            points,
            dashed,
            color,
            arrow,
        });
    }
    Ok(curves)
}

fn parse_circles(scene: &Value) -> Result<Vec<Circle>, String> {
    let Some(list) = scene.get("circles").and_then(|node| node.as_array()) else {
        return Ok(Vec::new());
    };
    let mut circles = Vec::new();
    for item in list {
        let x = number(item.get("x").or_else(|| item.get("cx")));
        let y = number(item.get("y").or_else(|| item.get("cy")));
        let r = number(item.get("r")).or_else(|| number(item.get("radius")));
        if let (Some(x), Some(y), Some(r)) = (x, y, r) {
            if r <= 0.0 {
                continue;
            }
            circles.push(Circle {
                x: clamp(x, -40.0, 140.0),
                y: clamp(y, -40.0, 140.0),
                r,
                dashed: item.get("dashed").and_then(|node| node.as_bool()).unwrap_or(false),
            });
        }
    }
    Ok(circles)
}

fn parse_marks(scene: &Value, index: &HashMap<String, usize>, problems: &mut Vec<String>) -> Vec<Mark> {
    let Some(list) = scene.get("marks").and_then(|node| node.as_array()) else {
        return Vec::new();
    };
    let mut marks = Vec::new();
    for item in list {
        let kind = item.get("type").and_then(|node| node.as_str()).unwrap_or("right");
        if !kind.eq_ignore_ascii_case("right") && !kind.eq_ignore_ascii_case("perpendicular") {
            continue;
        }
        let (Ok(at), Ok(a), Ok(b)) = (
            lookup(index, item.get("at"), "直角标记"),
            lookup(index, item.get("a"), "直角标记"),
            lookup(index, item.get("b"), "直角标记"),
        ) else {
            problems.push("直角标记引用了不存在的点".to_string());
            continue;
        };
        if at == a || at == b || a == b {
            continue;
        }
        marks.push(Mark { at, a, b });
    }
    marks
}

fn parse_labels(scene: &Value) -> Vec<(f64, f64, String)> {
    let mut out = Vec::new();
    if let Some(list) = scene.get("labels").and_then(|node| node.as_array()) {
        for item in list {
            let text = item
                .get("text")
                .and_then(|node| node.as_str())
                .map(|text| text.trim().to_string())
                .unwrap_or_default();
            if text.is_empty() {
                continue;
            }
            if let (Some(x), Some(y)) = (number(item.get("x")), number(item.get("y"))) {
                out.push((clamp(x, 0.0, 100.0), clamp(y, 0.0, 100.0), text));
            }
        }
    }
    out
}

fn fit_view(
    nodes: &[Node],
    curves: &[Curve],
    circles: &[Circle],
    fixed_labels: &[(f64, f64, String)],
    top: f64,
) -> Fit {
    let mut min_x = f64::MAX;
    let mut max_x = f64::MIN;
    let mut min_y = f64::MAX;
    let mut max_y = f64::MIN;
    let mut stretch = |x: f64, y: f64| {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    };
    for node in nodes {
        stretch(node.x, node.y);
    }
    for curve in curves {
        for &(x, y) in &curve.points {
            stretch(x, y);
        }
    }
    for circle in circles {
        stretch(circle.x - circle.r, circle.y - circle.r);
        stretch(circle.x + circle.r, circle.y + circle.r);
    }
    // 模型给的自由标注（θ=30°、刻度、坐标）也算画面的一部分，否则会被排到标题带上。
    for (x, y, _) in fixed_labels {
        stretch(*x, *y);
    }
    if !min_x.is_finite() || !max_x.is_finite() || !min_y.is_finite() || !max_y.is_finite() {
        return Fit {
            scale: 1.0,
            ox: PAD,
            oy: PAD,
            height: MIN_HEIGHT,
        };
    }
    let span_x = (max_x - min_x).max(1.0);
    let span_y = (max_y - min_y).max(1.0);
    let inner_w = WIDTH - PAD * 2.0;
    let wanted = inner_w * span_y / span_x + top + PAD;
    // 有标题时顶上多留一截，图不会钻到标题底下。
    let height = clamp(wanted, MIN_HEIGHT + (top - PAD), MAX_HEIGHT);
    let usable_h = (height - top - PAD).max(40.0);
    let scale = (inner_w / span_x).min(usable_h / span_y);
    // 按实际用量重新居中，横向纵向都落在可用区域内，标签才有地方放。
    let ox = (WIDTH - span_x * scale) / 2.0 - min_x * scale;
    let oy = top + (usable_h - span_y * scale) / 2.0 - min_y * scale;
    Fit {
        scale,
        ox,
        oy,
        height,
    }
}

/// 标注布局：方向取「背离图形中心」和「背离相邻线段」的合成方向，
/// 再沿该方向的几个半径 / 偏角里挑第一个不压线、不撞别的位置。
fn layout_labels(
    nodes: &[Node],
    segments: &[Segment],
    curves: &[Curve],
    fit: &Fit,
    placed: &mut Vec<Rect>,
) -> Vec<(f64, f64, String)> {
    let mut out = Vec::new();

    if nodes.is_empty() {
        return out;
    }
    let mut cx = 0.0;
    let mut cy = 0.0;
    for node in nodes {
        cx += node.x;
        cy += node.y;
    }
    cx /= nodes.len() as f64;
    cy /= nodes.len() as f64;

    // 所有线段在像素坐标下的端点，供「别压在线上」检查。
    let mut strokes: Vec<((f64, f64), (f64, f64))> = Vec::new();
    for segment in segments {
        strokes.push((
            fit.px(nodes[segment.a].x, nodes[segment.a].y),
            fit.px(nodes[segment.b].x, nodes[segment.b].y),
        ));
    }
    for curve in curves {
        for pair in curve.points.windows(2) {
            strokes.push((fit.px(pair[0].0, pair[0].1), fit.px(pair[1].0, pair[1].1)));
        }
    }

    for (position, node) in nodes.iter().enumerate() {
        let Some(text) = &node.label else {
            continue;
        };
        let (px, py) = fit.px(node.x, node.y);
        let width = text_width(text, LABEL_SIZE);
        let height = LABEL_SIZE * 1.15;

        let mut dx = node.x - cx;
        let mut dy = node.y - cy;
        let away = dx.hypot(dy);
        if away < 0.001 {
            dx = 0.0;
            dy = 1.0;
        } else {
            dx /= away;
            dy /= away;
        }
        // 有相邻线段时，取其反方向（从邻居指向本点）当作主方向，标注才落在图形外侧。
        let mut nx = 0.0;
        let mut ny = 0.0;
        for segment in segments {
            let other = if segment.a == position {
                Some(segment.b)
            } else if segment.b == position {
                Some(segment.a)
            } else {
                None
            };
            if let Some(other) = other {
                nx += node.x - nodes[other].x;
                ny += node.y - nodes[other].y;
            }
        }
        let norm = (nx * nx + ny * ny).sqrt();
        if norm > 0.001 {
            // 背离图形中心占四成五（保证落在图形外侧），背离相邻线段占五成五（避开棱）。
            dx = dx * 0.45 + nx / norm * 0.55;
            dy = dy * 0.45 + ny / norm * 0.55;
            let len = dx.hypot(dy).max(0.001);
            dx /= len;
            dy /= len;
        }
        let base = dy.atan2(dx);
        let base = if base.is_finite() {
            base
        } else {
            -std::f64::consts::FRAC_PI_2
        };

        let mut best: Option<(f64, f64)> = None;
        'outer: for radius in [14.0_f64, 19.0, 25.0] {
            for offset in [0.0_f64, 0.35, -0.35, 0.7, -0.7, 1.15, -1.15, std::f64::consts::PI] {
                let angle = base + offset;
                let x = px + angle.cos() * radius;
                let y = py + angle.sin() * radius;
                let rect = Rect {
                    x: x - width / 2.0,
                    y: y - height / 2.0,
                    w: width,
                    h: height,
                };
                if rect.x < 4.0 || rect.y < 4.0 || rect.x + rect.w > WIDTH - 4.0 || rect.y + rect.h > fit.height - 4.0 {
                    continue;
                }
                if placed.iter().any(|other| rect.hits(other)) {
                    continue;
                }
                if strokes
                    .iter()
                    .any(|(a, b)| point_segment_distance((x, y), *a, *b) < 11.0)
                {
                    continue;
                }
                best = Some((x, y));
                placed.push(rect);
                break 'outer;
            }
        }
        let (x, y) = best.unwrap_or((px, py - 16.0));
        if best.is_none() {
            placed.push(Rect {
                x: x - width / 2.0,
                y: y - height / 2.0,
                w: width,
                h: height,
            });
        }
        out.push((x, y, text.clone()));
    }
    out
}

struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// 给模型指定的数值标注找一个不压点名/不压别的字的位置。
/// 先试原位置，再沿上下左右挪；都放不下就不画（返回 None）。
fn place_fixed_label(
    x: f64,
    y: f64,
    text: &str,
    fit: &Fit,
    placed: &mut Vec<Rect>,
) -> Option<(f64, f64)> {
    let width = text_width(text, 13.0);
    let height = 13.0 * 1.15;
    let (px, py) = fit.px(x, y);
    let offsets = [
        (0.0, 0.0),
        (0.0, -14.0),
        (0.0, 14.0),
        (-width / 2.0 - 10.0, 0.0),
        (width / 2.0 + 10.0, 0.0),
        (0.0, -26.0),
        (0.0, 26.0),
        (-width / 2.0 - 10.0, -14.0),
        (width / 2.0 + 10.0, -14.0),
    ];
    for (dx, dy) in offsets {
        let cx = px + dx;
        let cy = py + dy;
        let rect = Rect {
            x: cx - width / 2.0,
            y: cy - height / 2.0,
            w: width,
            h: height,
        };
        if rect.x < 4.0 || rect.y < 4.0 || rect.x + rect.w > WIDTH - 4.0 || rect.y + rect.h > fit.height - 4.0 {
            continue;
        }
        if placed.iter().any(|other| rect.hits(other)) {
            continue;
        }
        placed.push(rect);
        return Some((cx, cy));
    }
    None
}

impl Rect {
    fn hits(&self, other: &Rect) -> bool {
        self.x < other.x + other.w + 1.0
            && other.x < self.x + self.w + 1.0
            && self.y < other.y + other.h + 1.0
            && other.y < self.y + self.h + 1.0
    }
}

fn point_segment_distance(point: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let vx = b.0 - a.0;
    let vy = b.1 - a.1;
    let len2 = vx * vx + vy * vy;
    if len2 < 0.0001 {
        return (point.0 - a.0).hypot(point.1 - a.1);
    }
    let t = (((point.0 - a.0) * vx + (point.1 - a.1) * vy) / len2).clamp(0.0, 1.0);
    let proj = (a.0 + t * vx, a.1 + t * vy);
    (point.0 - proj.0).hypot(point.1 - proj.1)
}

fn right_angle(at: (f64, f64), a: (f64, f64), b: (f64, f64)) -> Option<String> {
    let u = unit(at, a)?;
    let v = unit(at, b)?;
    let size = 9.0;
    let p1 = (at.0 + u.0 * size, at.1 + u.1 * size);
    let p2 = (p1.0 + v.0 * size, p1.1 + v.1 * size);
    let p3 = (at.0 + v.0 * size, at.1 + v.1 * size);
    Some(format!(
        "{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
        p1.0, p1.1, p2.0, p2.1, p3.0, p3.1
    ))
}

fn unit(from: (f64, f64), to: (f64, f64)) -> Option<(f64, f64)> {
    let dx = to.0 - from.0;
    let dy = to.1 - from.1;
    let len = dx.hypot(dy);
    if len < 0.5 {
        return None;
    }
    Some((dx / len, dy / len))
}

fn arrow_head(from: (f64, f64), to: (f64, f64), color: &str) -> String {
    let Some(dir) = unit(from, to) else {
        return String::new();
    };
    let size = 8.5;
    let base = (to.0 - dir.0 * size, to.1 - dir.1 * size);
    let half = 3.6;
    let p1 = (base.0 - dir.1 * half, base.1 + dir.0 * half);
    let p2 = (base.0 + dir.1 * half, base.1 - dir.0 * half);
    format!(
        "<polygon points=\"{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}\" fill=\"{}\"/>",
        to.0, to.1, p1.0, p1.1, p2.0, p2.1, color
    )
}

fn polyline(points: &str, color: &str, dashed: bool) -> String {
    format!(
        "<polyline points=\"{points}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\" \
         stroke-linecap=\"round\" stroke-linejoin=\"round\"{}/>",
        if dashed {
            " stroke-dasharray=\"6,4\""
        } else {
            ""
        }
    )
}

fn color_of(item: &Value, fallback: &str) -> String {
    item.get("color")
        .and_then(|node| node.as_str())
        .filter(|text| valid_color(text))
        .unwrap_or(fallback)
        .to_string()
}

/// 只接受 #rgb / #rrggbb，别把 `red`、`url(...)` 之类带进 SVG。
fn valid_color(text: &str) -> bool {
    let text = text.trim();
    if !text.starts_with('#') || (text.len() != 4 && text.len() != 7) {
        return false;
    }
    text[1..].chars().all(|ch| ch.is_ascii_hexdigit())
}

fn number(node: Option<&Value>) -> Option<f64> {
    let node = node?;
    if let Some(value) = node.as_f64() {
        return Some(value);
    }
    node.as_str()
        .and_then(|text| text.trim().trim_end_matches('%').parse::<f64>().ok())
}

fn clamp(value: f64, low: f64, high: f64) -> f64 {
    if value.is_nan() {
        return low;
    }
    value.max(low).min(high)
}

fn text_width(text: &str, size: f64) -> f64 {
    text.chars()
        .map(|ch| if ch.is_ascii() { size * 0.58 } else { size })
        .sum::<f64>()
        + 2.0
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::render;
    use serde_json::json;

    fn pyramid() -> serde_json::Value {
        json!({
            "title": "四棱锥 P-ABCD",
            "points": [
                {"id": "A", "x": 20, "y": 80},
                {"id": "B", "x": 62, "y": 88},
                {"id": "C", "x": 84, "y": 66},
                {"id": "D", "x": 42, "y": 58},
                {"id": "P", "x": 52, "y": 12}
            ],
            "faces": [{"points": ["A", "B", "C", "D"]}],
            "segments": [
                {"a": "P", "b": "A"}, {"a": "P", "b": "B"},
                {"a": "P", "b": "C", "dashed": true}, {"a": "P", "b": "D"},
                {"a": "A", "b": "B"}, {"a": "B", "b": "C"},
                {"a": "C", "b": "D", "dashed": true}, {"a": "D", "b": "A"}
            ]
        })
    }

    #[test]
    fn renders_pyramid_with_all_edges_and_labels() {
        let svg = render(&pyramid()).unwrap();
        assert!(svg.starts_with("<svg"), "实际：{svg}");
        assert!(svg.ends_with("</svg>"));
        assert_eq!(svg.matches("<line").count(), 8, "五条棱 + 四条底边");
        assert!(svg.contains(">P</text>"));
        assert!(svg.contains(">A</text>"));
        assert!(svg.contains("viewBox=\"0 0 480 "));
        // 当 <img> 显示的图必须有具体宽高，否则浏览器算不出固有尺寸，预览一片空白。
        assert!(svg.contains("width=\"480\" height=\""), "实际：{}", &svg[..120.min(svg.len())]);
        assert!(!svg.contains("width=\"100%\""));
    }

    /// 模型既在 points 里给点名、又在 labels 里重复写一遍时，图上只能出现一次。
    #[test]
    fn duplicate_point_labels_are_dropped() {
        let svg = render(&json!({
            "points": [
                {"id": "A", "x": 20, "y": 80},
                {"id": "B", "x": 60, "y": 80}
            ],
            "segments": [{"a": "A", "b": "B"}],
            "labels": [
                {"x": 20, "y": 88, "text": "A"},
                {"x": 60, "y": 88, "text": "B"},
                {"x": 40, "y": 96, "text": "AB=2"}
            ]
        }))
        .unwrap();
        assert_eq!(svg.matches(">A</text>").count(), 1, "实际：{svg}");
        assert_eq!(svg.matches(">B</text>").count(), 1, "实际：{svg}");
        assert!(svg.contains(">AB=2</text>"));
    }

    /// 标签不许压在一起：每个字的中心点两两间距要大于字号。
    #[test]
    fn labels_do_not_overlap() {
        let svg = render(&json!({
            "points": [
                {"id": "A", "x": 30, "y": 40},
                {"id": "B", "x": 34, "y": 44},
                {"id": "C", "x": 38, "y": 48}
            ],
            "segments": [{"a": "A", "b": "B"}, {"a": "B", "b": "C"}]
        }))
        .unwrap();
        let mut centers = Vec::new();
        for chunk in svg.split("<text").skip(1) {
            let x: f64 = chunk
                .split("x=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap()
                .parse()
                .unwrap();
            let y: f64 = chunk
                .split("y=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap()
                .parse()
                .unwrap();
            centers.push((x, y));
        }
        assert_eq!(centers.len(), 3);
        for (i, a) in centers.iter().enumerate() {
            for b in centers.iter().skip(i + 1) {
                let distance = (a.0 - b.0).hypot(a.1 - b.1);
                assert!(distance > 12.0, "两个标签挨太近：{distance}");
            }
        }
    }

    #[test]
    fn rejects_unknown_point_reference() {
        let error = render(&json!({
            "points": [{"id": "A", "x": 10, "y": 10}],
            "segments": [{"a": "A", "b": "Z"}]
        }))
        .unwrap_err();
        assert!(error.contains('Z'), "报错要点名是哪个点：{error}");
    }

    #[test]
    fn rejects_scene_without_geometry() {
        let error = render(&json!({"points": [{"id": "A", "x": 10, "y": 10}]})).unwrap_err();
        assert!(error.contains("segments"));
    }

    /// 坐标系 + 曲线 + 箭头：函数题的基本形态。
    #[test]
    fn renders_axes_curve_and_arrows() {
        let svg = render(&json!({
            "points": [
                {"id": "O", "x": 50, "y": 70, "hideLabel": false},
                {"id": "X", "x": 95, "y": 70},
                {"id": "Y", "x": 50, "y": 10}
            ],
            "segments": [
                {"a": "O", "b": "X", "arrow": true},
                {"a": "O", "b": "Y", "arrow": true}
            ],
            "curves": [{"points": [[50, 70], [60, 52], [70, 44], [80, 44], [90, 50]]}],
            "labels": [{"x": 96, "y": 64, "text": "x"}, {"x": 54, "y": 8, "text": "y"}]
        }))
        .unwrap();
        assert!(svg.contains("<polyline"));
        assert!(svg.contains("<polygon"), "箭头要有实心三角");
        assert!(svg.contains(">x</text>"));
    }

    #[test]
    fn clips_absurd_coordinates_instead_of_panicking() {
        let svg = render(&json!({
            "points": [
                {"id": "A", "x": -9000, "y": 5000},
                {"id": "B", "x": 9000, "y": -5000}
            ],
            "segments": [{"a": "A", "b": "B"}]
        }))
        .unwrap();
        assert!(svg.contains("<line"));
        assert!(!svg.contains("NaN"));
    }

    #[test]
    fn escapes_text_and_drops_hostile_colors() {
        let svg = render(&json!({
            "points": [
                {"id": "A", "x": 10, "y": 10, "label": "A<B"},
                {"id": "B", "x": 90, "y": 90}
            ],
            "segments": [{"a": "A", "b": "B", "color": "url(#evil)"}],
            "title": "∠A & ∠B"
        }))
        .unwrap();
        assert!(!svg.contains("url("));
        assert!(svg.contains("&lt;"));
        assert!(svg.contains("&amp;"));
    }

    /// 零长度线段不该画出来（模型偶尔给 a、b 同一个点）。
    #[test]
    fn skips_degenerate_segment() {
        let svg = render(&json!({
            "points": [{"id": "A", "x": 10, "y": 10}, {"id": "B", "x": 90, "y": 90}],
            "segments": [{"a": "A", "b": "A"}, {"a": "A", "b": "B"}]
        }))
        .unwrap();
        assert_eq!(svg.matches("<line").count(), 1);
    }

    /// 中点由本地求值：模型只声明关系，坐标不该再手算（手算最容易错一半）。
    #[test]
    fn resolves_midpoint_before_rendering() {
        let svg = render(&json!({
            "points": [
                {"id": "A1", "x": 20, "y": 20},
                {"id": "D", "x": 60, "y": 80},
                {"id": "N", "midpoint": ["A1", "D"]}
            ],
            "segments": [{"a": "A1", "b": "N"}, {"a": "N", "b": "D"}]
        }))
        .unwrap();
        assert_eq!(svg.matches("<line").count(), 2);
        // N 应该画在 A1D 的中点 (40,50)，换算成像素后落在两段之间。
        assert!(svg.contains(">N</text>"));
        assert!(svg.contains(">A₁</text>"), "A1 要显示成下标：{svg}");
    }

    /// 模型把 A1 和 A₁ 各写一遍时，图上只能有一个标注。
    #[test]
    fn duplicate_labels_ignore_subscript_spelling() {
        let svg = render(&json!({
            "points": [
                {"id": "A1", "x": 20, "y": 20},
                {"id": "B1", "x": 60, "y": 20}
            ],
            "segments": [{"a": "A1", "b": "B1"}],
            "labels": [
                {"x": 20, "y": 28, "text": "A₁"},
                {"x": 60, "y": 28, "text": "B1"}
            ]
        }))
        .unwrap();
        assert_eq!(svg.matches(">A₁</text>").count(), 1, "实际：{svg}");
        assert_eq!(svg.matches(">B₁</text>").count(), 1, "实际：{svg}");
    }

    /// midpoint 引用了不存在的点要明确报错，而不是画到 (0,0)。
    #[test]
    fn rejects_midpoint_with_unknown_point() {
        let error = render(&json!({
            "points": [{"id": "A", "x": 10, "y": 10}, {"id": "E", "midpoint": ["A", "Z"]}],
            "segments": [{"a": "A", "b": "E"}]
        }))
        .unwrap_err();
        assert!(error.contains("midpoint") || error.contains("不存在"), "实际：{error}");
    }

    /// 模型把 C1 漏了平移、又漏掉 C-C1 时，直四棱柱校正要把 12 条棱补齐，
    /// 并且虚线只留在后底边 AD、后底边 DC、后左侧棱 DD1 上。
    #[test]
    fn normalizes_broken_right_prism() {
        let svg = render(&json!({
            "points": [
                {"id": "A", "x": 15, "y": 70},
                {"id": "B", "x": 45, "y": 70},
                {"id": "C", "x": 60, "y": 45},
                {"id": "D", "x": 30, "y": 45},
                {"id": "A1", "x": 15, "y": 18},
                {"id": "B1", "x": 45, "y": 18},
                {"id": "C1", "x": 60, "y": 45},
                {"id": "D1", "x": 30, "y": 18}
            ],
            "segments": [
                {"a": "A", "b": "B"}, {"a": "B", "b": "C"}, {"a": "A", "b": "A1"},
                {"a": "B", "b": "B1"}
            ]
        }))
        .unwrap();
        assert_eq!(svg.matches("<line").count(), 12, "实际：{svg}");
        assert_eq!(svg.matches("stroke-dasharray").count(), 3, "实际：{svg}");
        // C1 被校正到和 D1 同一排（顶面后排），并且比 A1 高（y 更小）。
        let label_y = |label: &str| -> f64 {
            svg.split(label)
                .next()
                .and_then(|head| head.rsplit(" y=\"").next())
                .and_then(|rest| rest.split('"').next())
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or_else(|| panic!("找不到 {label} 的 y：{svg}"))
        };
        let (c1_y, d1_y, a1_y) = (label_y(">C₁</text>"), label_y(">D₁</text>"), label_y(">A₁</text>"));
        assert!((c1_y - d1_y).abs() < 2.0, "C1 在 {c1_y}，D1 在 {d1_y}");
        assert!(c1_y < a1_y, "C1 在 {c1_y}，A1 在 {a1_y}");
    }
}
