//! 讲义产物渲染：把一份 Handout Spec 变成一个自包含的单文件 HTML 讲义。
//!
//! 讲义 ≠ 3D 仿真课件（那是 `courseware-lab` 的活）。这里产出的是「讲授 + 练题」
//! 的教案/学案：知识清单、例题精讲、变式拓展、拔高、思维训练、小结与作业。
//! 单文件、断网可看、A4 打印分页友好，题干与解析全部内联（生成时就不依赖运行时取数）。

use super::HandoutSpec;

/// 产物自检锚点：第五道闸靠它确认运行时确实写进去了。
pub const MARKER: &str = "window.__NBHANDOUT__";

pub fn render(spec: &HandoutSpec) -> Result<String, String> {
    // HTML 只需要内嵌图片，不把老师最初选择文件的绝对路径带进可分发的产物。
    // 数据库里的 HandoutSpec 仍保留 source_path，供本机校对时打开原图。
    let mut portable = spec.clone();
    for image in &mut portable.images {
        image.source_path.clear();
    }
    let json = serde_json::to_string(&portable).map_err(|e| format!("讲义数据序列化失败：{e}"))?;
    // 防止文案里出现 </script> 把脚本提前截断
    let json = json.replace("</", "<\\/");
    let theme = match spec.theme.as_str() {
        "chalk" => "chalk",
        "science" => "science",
        _ => "paper",
    };
    let html = TEMPLATE
        .replace("__TITLE__", &escape(&spec.title))
        .replace("__THEME__", theme)
        .replace("__LESSON__", &json);
    Ok(html)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const TEMPLATE: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>__TITLE__ · 讲义</title>
<style>
*{box-sizing:border-box}
html,body{margin:0}
body{
  --bg:#f4f0e8; --paper:#fffdf8; --ink:#211e18; --ink2:#4b453a; --muted:#8b8272;
  --line:#e5dcc8; --line2:#d3c7ac; --accent:#b0522c; --accent-soft:#f8e8df; --accent-ink:#8c3d1e;
  --ok:#2c7a52; --ok-soft:#e7f4ec; --bad:#ad3a3a; --bad-soft:#fbeaea;
  background:var(--bg); color:var(--ink);
  font:15px/1.75 -apple-system,BlinkMacSystemFont,'PingFang SC','Hiragino Sans GB','Microsoft YaHei',sans-serif;
  -webkit-font-smoothing:antialiased;
}
body.t-chalk{
  --bg:#1b2723; --paper:#22312c; --ink:#eef4ef; --ink2:#cfe0d6; --muted:#93a89c;
  --line:#33463e; --line2:#43594f; --accent:#7fd4a8; --accent-soft:#2b453a; --accent-ink:#a5e6c6;
  --ok:#8bdcae; --ok-soft:#2a4436; --bad:#f0a3a3; --bad-soft:#472e2e;
}
body.t-science{
  --bg:#eef3f9; --paper:#ffffff; --ink:#141e2a; --ink2:#3f4d5e; --muted:#7d8b9c;
  --line:#dde5ef; --line2:#c6d2e2; --accent:#2f6fed; --accent-soft:#e8efff; --accent-ink:#1f4fae;
  --ok:#1d7a4f; --ok-soft:#e5f5ec; --bad:#b23b3b; --bad-soft:#fbe9e9;
}
.bar{position:sticky;top:0;z-index:5;background:var(--bg);border-bottom:1px solid var(--line);
  display:flex;align-items:center;gap:12px;padding:10px 22px}
.bar .brand{display:flex;align-items:center;gap:8px;min-width:150px}
.bar .dot{width:9px;height:9px;border-radius:50%;background:var(--accent);box-shadow:0 0 0 4px var(--accent-soft)}
.bar .brand-text{font-size:12px;letter-spacing:.08em;color:var(--muted)}
.bar .title{flex:1;text-align:center;font-size:14px;font-weight:620;color:var(--ink2);
  white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.bar .acts{display:flex;gap:6px;min-width:150px;justify-content:flex-end}
.bar button{appearance:none;border:1px solid var(--line2);background:transparent;color:var(--ink2);
  border-radius:9px;padding:5px 12px;font-size:12.5px;cursor:pointer}
.bar button:hover{border-color:var(--accent);color:var(--accent-ink);background:var(--accent-soft)}
.sheet{max-width:920px;margin:26px auto 60px;padding:0 18px}
.page{background:var(--paper);border:1px solid var(--line);border-radius:16px;
  padding:38px 44px 44px;box-shadow:0 2px 4px rgba(20,20,20,.03),0 18px 40px rgba(20,20,20,.05)}
.cover .kicker{font-size:11.5px;letter-spacing:.16em;color:var(--accent-ink);font-weight:600}
.cover h1{font-size:clamp(24px,3vw,34px);line-height:1.3;margin:10px 0 12px;letter-spacing:-.01em}
.cover .meta{display:flex;flex-wrap:wrap;gap:8px;margin:0 0 22px}
.chip{font-size:11.5px;padding:3px 10px;border-radius:20px;background:var(--accent-soft);color:var(--accent-ink)}
.chip.line{background:transparent;border:1px solid var(--line2);color:var(--muted)}
.goals{border:1px solid var(--line);border-radius:12px;padding:14px 18px;background:var(--bg);margin:0 0 8px}
.goals h4{margin:0 0 6px;font-size:11.5px;letter-spacing:.12em;color:var(--muted);font-weight:600}
.goals ol{margin:0;padding-left:19px}
.goals li{margin:3px 0;color:var(--ink2);font-size:14px}
.warmup{margin:18px 0 0;border-left:3px solid var(--accent);background:var(--accent-soft);
  border-radius:0 12px 12px 0;padding:12px 18px}
.warmup span.lbl{font-size:11px;letter-spacing:.12em;color:var(--accent-ink);display:block;margin-bottom:3px}
.warmup p{margin:0;font-size:14.5px;color:var(--ink2);line-height:1.85}
section.block{margin:30px 0 0}
section.block > h2{font-size:17px;margin:0 0 12px;padding-bottom:8px;border-bottom:2px solid var(--line);
  display:flex;align-items:baseline;gap:10px}
section.block > h2 .no{font:650 12px/1 'SF Mono',Menlo,Consolas,monospace;color:var(--accent)}
section.block > h2 .hint{font-size:11.5px;color:var(--muted);font-weight:400;margin-left:auto}
.point{border:1px solid var(--line);border-radius:12px;padding:15px 18px;margin-bottom:10px;background:var(--bg)}
.point h3{margin:0 0 6px;font-size:15px}
.point .summary{margin:0 0 8px;font-size:14.5px;color:var(--ink2);line-height:1.8}
.point .detail{margin:0 0 8px;font-size:13.5px;color:var(--ink2);line-height:1.8}
.point .detail p{margin:0 0 7px}
.point .formula{margin:8px 0;font-family:var(--mono,'SF Mono',Menlo,Consolas,monospace);font-size:13px;
  background:var(--paper);border:1px solid var(--line2);border-radius:8px;padding:8px 12px;color:var(--accent-ink)}
.point .warn{margin:8px 0 0;border:1px dashed var(--bad);background:var(--bad-soft);border-radius:9px;
  padding:8px 12px;font-size:13px;color:var(--bad)}
.point .warn b{margin-right:6px}
.item{border:1px solid var(--line);border-radius:12px;padding:16px 18px;margin-bottom:10px;background:var(--paper)}
.item .head{display:flex;align-items:center;gap:9px;margin-bottom:8px}
.item .badge{font-size:11px;font-weight:650;padding:2px 9px;border-radius:20px;background:var(--accent-soft);color:var(--accent-ink)}
.item .badge.line{background:transparent;border:1px solid var(--line2);color:var(--muted);font-weight:500}
.item .qno{font:650 11px/1 'SF Mono',Menlo,Consolas,monospace;color:var(--muted)}
.item .stem{margin:0 0 10px;font-size:14.5px;line-height:1.85;white-space:pre-wrap}
.item .steps{margin:0 0 10px;padding-left:19px}
.item .steps li{margin:4px 0;font-size:13.5px;color:var(--ink2);line-height:1.75}
.item .fold{border-top:1px dotted var(--line2);padding-top:10px;margin-top:4px}
.item .fold > summary{cursor:pointer;font-size:12px;letter-spacing:.06em;color:var(--muted);list-style:none;
  user-select:none;display:flex;align-items:center;gap:6px}
.item .fold > summary::-webkit-details-marker{display:none}
.item .fold > summary::before{content:"▸";display:inline-block;transition:transform .15s}
.item .fold[open] > summary::before{transform:rotate(90deg)}
.item .answer{margin:8px 0 0;font-size:13.5px;color:var(--ok);font-weight:600}
.item .analysis{margin:8px 0 0;font-size:13.5px;color:var(--ink2);line-height:1.8}
.item .analysis p{margin:0 0 7px}
.think{border:1px solid var(--line2);border-radius:12px;padding:15px 18px;margin-bottom:10px;
  background:linear-gradient(180deg,var(--bg),var(--paper))}
.think h4{margin:0 0 7px;font-size:14px}
.think .prompt{margin:0 0 8px;font-size:14px;color:var(--ink2);line-height:1.85;white-space:pre-wrap}
.think .hint{margin:0;font-size:13px;color:var(--muted);line-height:1.75}
.summary-box{border:1px solid var(--line);border-radius:12px;padding:15px 18px;background:var(--bg);margin:0 0 10px}
.summary-box p{margin:0;font-size:14px;color:var(--ink2);line-height:1.85;white-space:pre-wrap}
.homework li{margin:5px 0;font-size:14px;color:var(--ink2)}
.figure{margin:0 0 14px;border:1px solid var(--line);border-radius:12px;padding:12px;background:var(--bg);text-align:center}
.figure img{display:block;max-width:100%;max-height:620px;margin:0 auto;object-fit:contain;border-radius:8px}
.figure figcaption{margin-top:8px;color:var(--ink2);font-size:12.5px;line-height:1.6}
.figure .placement{display:inline-block;margin-left:6px;color:var(--muted)}
.empty-note{color:var(--muted);font-size:13px;padding:8px 2px}
.foot{max-width:920px;margin:0 auto 40px;padding:0 18px;color:var(--muted);font-size:11.5px;text-align:center}
@media (max-width:720px){.page{padding:24px 20px 30px;border-radius:12px}.sheet{margin:14px auto 30px}}
@media print{
  body{background:#fff}
  .bar,.foot{display:none}
  .sheet{margin:0;max-width:none;padding:0}
  .page{border:0;box-shadow:none;border-radius:0;padding:0}
  .point,.item,.think,.goals,.summary-box,.figure{break-inside:avoid;page-break-inside:avoid}
  section.block{break-before:auto}
  .item .fold{display:block}
  .item .fold > summary{display:none}
  .item .fold .analysis,.item .fold .answer{display:block!important}
}
</style>
</head>
<body class="t-__THEME__">
<header class="bar">
  <div class="brand"><span class="dot"></span><span class="brand-text">教师讲义</span></div>
  <div class="title">__TITLE__</div>
  <div class="acts">
    <button id="printBtn" type="button">打印 / 导出 PDF</button>
    <button id="toggleBtn" type="button">展开全部解析</button>
  </div>
</header>
<div class="sheet">
  <div class="page">
    <div class="cover">
      <div class="kicker">讲义</div>
      <h1 id="h1">__TITLE__</h1>
      <div class="meta" id="meta"></div>
      <div class="goals" id="goals" hidden><h4>教学目标</h4><ol id="goalList"></ol></div>
      <div class="warmup" id="warmup" hidden><span class="lbl">情境导入</span><p id="warmupText"></p></div>
    </div>
    <section class="block" id="blockPoints" hidden>
      <h2><span class="no">01</span>知识清单<span class="hint">概念 · 要点 · 易错提醒</span></h2>
      <div id="points"></div>
    </section>
    <section class="block" id="blockImages" hidden>
      <h2><span class="no">02</span>教学图片<span class="hint">本地素材 · 已内嵌</span></h2>
      <div id="images"></div>
    </section>
    <section class="block" id="blockExamples" hidden>
      <h2><span class="no">02</span>例题精讲<span class="hint">先做再讲 · 步骤留痕</span></h2>
      <div id="examples"></div>
    </section>
    <section class="block" id="blockVariants" hidden>
      <h2><span class="no">03</span>变式拓展<span class="hint">换条件 / 换问法 · 一题多变</span></h2>
      <div id="variants"></div>
    </section>
    <section class="block" id="blockChallenge" hidden>
      <h2><span class="no">04</span>拔高挑战<span class="hint">给学有余力的学生</span></h2>
      <div id="challenge"></div>
    </section>
    <section class="block" id="blockThinking" hidden>
      <h2><span class="no">05</span>思维训练<span class="hint">开放题 · 只给提示不给答案</span></h2>
      <div id="thinking"></div>
    </section>
    <section class="block" id="blockTail" hidden>
      <h2><span class="no">06</span>小结与作业</h2>
      <div class="summary-box" id="summaryBox" hidden><p id="summaryText"></p></div>
      <ol class="homework" id="homework"></ol>
    </section>
  </div>
</div>
<div class="foot">本讲义由 NeuralKB 本地题库生成 · 单文件离线可用 · 打印时解析自动展开</div>
<script>window.__NBHANDOUT__ = __LESSON__;</script>
<script>
(function () {
  var L = window.__NBHANDOUT__ || {};
  function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text !== undefined && text !== null) node.textContent = String(text);
    return node;
  }
  function prose(text) {
    var box = el('div');
    String(text || '').split(/\n{2,}/).forEach(function (chunk) {
      var trimmed = chunk.trim();
      if (!trimmed) return;
      trimmed.split('\n').forEach(function (line) {
        if (line.trim()) box.appendChild(el('p', null, line.trim()));
      });
    });
    return box;
  }
  function has(list) { return Array.isArray(list) && list.length > 0; }

  document.body.className = 't-' + (L.theme || 'paper');
  document.title = (L.title || '讲义') + ' · 讲义';

  var meta = document.getElementById('meta');
  [L.subject, L.grade, L.chapter].filter(Boolean).forEach(function (text) {
    meta.appendChild(el('span', 'chip', text));
  });
  (L.knowledge || []).forEach(function (text) {
    if (text) meta.appendChild(el('span', 'chip line', text));
  });

  if (has(L.objectives)) {
    var goals = document.getElementById('goals');
    goals.hidden = false;
    var goalList = document.getElementById('goalList');
    L.objectives.forEach(function (text) { if (text) goalList.appendChild(el('li', null, text)); });
  }
  if (L.warmup && String(L.warmup).trim()) {
    document.getElementById('warmup').hidden = false;
    document.getElementById('warmupText').textContent = L.warmup;
  }

  function renderItem(item, index, kind) {
    var box = el('div', 'item');
    var head = el('div', 'head');
    head.appendChild(el('span', 'qno', '例 ' + (index + 1)));
    head.appendChild(el('span', 'badge' + (kind === 'example' ? '' : ' line'),
      kind === 'example' ? '例题' : '变式'));
    if (item.questionId) head.appendChild(el('span', 'badge line', '题库 ' + String(item.questionId).slice(0, 8)));
    box.appendChild(head);
    if (item.stem) box.appendChild(el('p', 'stem', item.stem));
    if (has(item.steps)) {
      var ol = el('ol', 'steps');
      item.steps.forEach(function (line) { if (line) ol.appendChild(el('li', null, line)); });
      box.appendChild(ol);
    }
    var fold = el('details', 'fold');
    fold.appendChild(el('summary', null, '答案与解析'));
    if (item.answer) fold.appendChild(el('p', 'answer', '答案：' + item.answer));
    if (item.analysis) fold.appendChild(prose(item.analysis));
    box.appendChild(fold);
    return box;
  }

  if (has(L.points)) {
    document.getElementById('blockPoints').hidden = false;
    var points = document.getElementById('points');
    L.points.forEach(function (point) {
      var box = el('div', 'point');
      box.appendChild(el('h3', null, point.name || ''));
      if (point.summary) box.appendChild(el('p', 'summary', point.summary));
      if (point.detail) box.appendChild(prose(point.detail));
      if (point.formula) box.appendChild(el('div', 'formula', point.formula));
      if (point.misconception) {
        var warn = el('div', 'warn');
        warn.appendChild(el('b', null, '易错'));
        warn.appendChild(document.createTextNode(point.misconception));
        box.appendChild(warn);
      }
      points.appendChild(box);
    });
  }

  var images = (L.images || []).filter(function (image) { return image && image.dataUrl; });
  if (has(images)) {
    document.getElementById('blockImages').hidden = false;
    var imageHost = document.getElementById('images');
    images.forEach(function (image, index) {
      var figure = el('figure', 'figure');
      var img = el('img');
      img.src = image.dataUrl;
      img.alt = image.caption || image.name || image.id || ('图 ' + (index + 1));
      figure.appendChild(img);
      var caption = el('figcaption');
      caption.appendChild(document.createTextNode(
        (image.id || ('图 ' + (index + 1))) + (image.caption ? ' · ' + image.caption : '')
      ));
      if (image.placement) caption.appendChild(el('span', 'placement', '用于：' + image.placement));
      figure.appendChild(caption);
      imageHost.appendChild(figure);
    });
  }

  function fill(id, block, items, kind) {
    if (!has(items)) return;
    document.getElementById(block).hidden = false;
    var host = document.getElementById(id);
    items.forEach(function (item, index) {
      host.appendChild(renderItem(item, index, kind));
    });
  }
  fill('examples', 'blockExamples', L.examples, 'example');
  fill('variants', 'blockVariants', L.variants, 'variant');
  fill('challenge', 'blockChallenge', L.challenge, 'variant');

  if (has(L.thinking)) {
    document.getElementById('blockThinking').hidden = false;
    var host = document.getElementById('thinking');
    L.thinking.forEach(function (item, index) {
      var box = el('div', 'think');
      box.appendChild(el('h4', null, '思维 ' + (index + 1) + (item.prompt ? '' : '')));
      if (item.prompt) box.appendChild(el('p', 'prompt', item.prompt));
      if (item.hint) box.appendChild(el('p', 'hint', '提示：' + item.hint));
      if (item.answer) {
        var fold = el('details', 'fold');
        fold.appendChild(el('summary', null, '参考思路'));
        fold.appendChild(el('p', 'analysis', item.answer));
        box.appendChild(fold);
      }
      host.appendChild(box);
    });
  }

  var tail = L.summary || has(L.homework);
  if (tail) {
    document.getElementById('blockTail').hidden = false;
    if (L.summary && String(L.summary).trim()) {
      document.getElementById('summaryBox').hidden = false;
      document.getElementById('summaryText').textContent = L.summary;
    }
    var homework = document.getElementById('homework');
    (L.homework || []).forEach(function (line) { if (line) homework.appendChild(el('li', null, line)); });
  }

  // 段号按「实际显示的段」重排：中间段为空时不留空号（04 拔高没写就跳过它）。
  (function () {
    var no = 0;
    Array.prototype.forEach.call(document.querySelectorAll('section.block'), function (block) {
      if (block.hidden) return;
      no += 1;
      var tag = block.querySelector('h2 .no');
      if (tag) tag.textContent = (no < 10 ? '0' : '') + no;
    });
  })();

  document.getElementById('printBtn').onclick = function () { window.print(); };
  var openAll = false;
  document.getElementById('toggleBtn').onclick = function () {
    openAll = !openAll;
    Array.prototype.forEach.call(document.querySelectorAll('details.fold'), function (node) {
      node.open = openAll;
    });
    this.textContent = openAll ? '收起全部解析' : '展开全部解析';
  };
})();
</script>
</body>
</html>
"#;
