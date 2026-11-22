/**
 * 讲评清单：把 `review_sheet` 返回的数据排成一页可打印的纸。
 *
 * 为什么和界面分成两份：界面用 KaTeX 渲染公式，打印稿不能引外部字体/脚本
 * （离线、自包含），所以公式统一降级成可读写法（`\frac{7}{3}` → `7/3`）。
 * 清单的读者是老师自己，读得懂比排版漂亮重要。
 *
 * 三条底线与家长周报一致：用户输入一律 `esc`；没有数据就写「没有」，不拿 0 凑；
 * 不出现学生账号之类的内部字段，只有姓名。
 */

import {latexToPlain} from './latexPlain.js';
import {splitLooseMath} from './latexMath.js';

function esc(raw) {
    return String(raw == null ? '' : raw)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;');
}

/** 题干 / 答案里可能带 `![](nbasset:id)` 和 `$...$`：图替换成占位说明，公式转纯文本。 */
function plain(raw) {
    const source = String(raw == null ? '' : raw).replace(/!\[[^\]]*\]\([^)]*\)/g, '（图）');
    return splitLooseMath(source)
        .map((part) => (part.latex === undefined ? part.text : latexToPlain(part.latex)))
        .join('')
        .replace(/\s+/g, ' ')
        .trim();
}

function pct(rate) {
    if (rate == null) return '未批';
    return `${Math.round(rate * 100)}%`;
}

function num(value) {
    const number = Number(value || 0);
    return Number.isInteger(number) ? String(number) : number.toFixed(1);
}

function rateClass(rate) {
    if (rate == null) return 'flat';
    if (rate < 0.5) return 'bad';
    if (rate < 0.8) return 'mid';
    return 'good';
}

const STYLE = `
* { box-sizing: border-box; }
body {
  margin: 0; padding: 18mm 15mm 12mm;
  font-family: -apple-system, "PingFang SC", "Songti SC", "Microsoft YaHei", sans-serif;
  color: #1b1b1f; font-size: 10.5pt; line-height: 1.55;
}
h1 { font-size: 17pt; margin: 0 0 2mm; }
.sub { color: #6b6b76; font-size: 9pt; margin: 0 0 4mm; }
.stats { display: flex; gap: 6mm; flex-wrap: wrap; margin: 0 0 5mm; padding: 3mm 4mm;
  border: 1px solid #e3e3e9; border-radius: 2mm; background: #fafafc; }
.stats b { display: block; font-size: 14pt; font-weight: 600; }
.stats span { font-size: 8.5pt; color: #6b6b76; }
h2 { font-size: 11.5pt; margin: 6mm 0 2mm; padding-bottom: 1mm; border-bottom: 1px solid #e3e3e9; }
table { width: 100%; border-collapse: collapse; font-size: 9pt; }
th, td { text-align: left; padding: 1.4mm 2mm; border-bottom: 1px solid #efeff3; vertical-align: top; }
th { color: #6b6b76; font-weight: 500; background: #fafafc; }
td.n, th.n { text-align: right; white-space: nowrap; }
.q { border: 1px solid #e3e3e9; border-radius: 2mm; padding: 3mm; margin: 0 0 3mm;
  break-inside: avoid; page-break-inside: avoid; }
.q-head { display: flex; align-items: baseline; gap: 2.5mm; margin-bottom: 1.5mm; }
.q-no { font-weight: 700; font-size: 11pt; }
.q-tag { font-size: 8pt; color: #6b6b76; border: 1px solid #e3e3e9; border-radius: 1mm; padding: 0 1.2mm; }
.q-rate { margin-left: auto; font-size: 9pt; }
.q-rate b { font-size: 11pt; }
.bad b { color: #b42318; }
.mid b { color: #b54708; }
.good b { color: #0f7b4f; }
.flat b { color: #6b6b76; }
.q-stem { margin: 1mm 0 1.5mm; }
.q-answer { color: #33333a; background: #f6f7fb; border-radius: 1.5mm; padding: 1.5mm 2mm; font-size: 9.5pt; }
.label { color: #6b6b76; font-size: 8.5pt; }
.names { margin: 1mm 0 0; }
.names li { margin: 0; }
ul { margin: 0; padding-left: 5mm; }
.remark { margin: 1mm 0 0; }
.remark b { font-weight: 600; }
.muted { color: #6b6b76; }
.print-tip { margin-top: 6mm; color: #9a9aa6; font-size: 8pt; }
@media print { body { padding: 12mm 12mm 8mm; } .print-tip { display: none; } }
`;

export function buildReviewHtml(sheet) {
    const questions = sheet.questions || [];
    const students = sheet.students || [];
    const missing = students.filter((row) => !row.submitted);
    const pendingStudents = students.filter((row) => row.submitted && row.pending > 0);
    // 最该讲的：排前 3 且有错
    const top = questions
        .filter((row) => row.rank <= 3 && row.wrong > 0)
        .sort((left, right) => left.rank - right.rank);

    const stats = `
<div class="stats">
  <div><b>${sheet.submittedCount}/${sheet.studentCount}</b><span>已交</span></div>
  <div><b>${num(sheet.avgScore)}</b><span>已批完的均分（满分 ${num(sheet.maxScore)}）</span></div>
  <div><b>${sheet.gradedItems}/${sheet.expectedItems}</b><span>已批题数 / 应交题数</span></div>
  <div><b>${questions.length}</b><span>题量</span></div>
</div>`;

    const overview = questions.length
        ? `<h2>题目概览（按题号）</h2>
<table>
  <thead><tr><th class="n">题号</th><th>题型</th><th class="n">难度</th><th>知识点</th>
    <th class="n">错 / 已判</th><th class="n">得分率</th><th class="n">未批</th></tr></thead>
  <tbody>
  ${questions
      .map(
          (row) => `<tr>
    <td class="n">${row.ord + 1}${row.rank <= 3 && row.wrong > 0 ? ' ★' : ''}</td>
    <td>${esc(row.typeLabel)}</td>
    <td class="n">${row.difficulty}</td>
    <td>${esc((row.knowledge || []).join('、') || '—')}</td>
    <td class="n">${row.wrong} / ${Math.max(row.attempted - row.pending, 0)}</td>
    <td class="n ${rateClass(row.rate)}">${pct(row.rate)}</td>
    <td class="n">${row.pending || '—'}</td>
  </tr>`,
      )
      .join('')}
  </tbody>
</table>`
        : '<p class="muted">这份作业里没有题目。</p>';

    const topBlock = top.length
        ? `<h2>先讲这三道</h2>
${top
      .map(
          (row) => `<div class="q">
  <div class="q-head">
    <span class="q-no">第 ${row.ord + 1} 题</span>
    <span class="q-tag">${esc(row.typeLabel)}</span>
    <span class="q-tag">难度 ${row.difficulty}</span>
    <span class="q-rate ${rateClass(row.rate)}">错 <b>${row.wrong}</b> 人 · 得分率 <b>${pct(row.rate)}</b></span>
  </div>
  <div class="q-stem">${esc(plain(row.stem))}</div>
  ${row.answer ? `<div class="q-answer"><span class="label">参考答案：</span>${esc(plain(row.answer))}</div>` : ''}
  ${row.wrongStudents.length ? `<p class="names"><span class="label">错的人：</span>${esc(row.wrongStudents.join('、'))}</p>` : ''}
</div>`,
      )
      .join('')}`
        : '';

    const detail = questions.length
        ? `<h2>逐题记录</h2>
${questions
      .map((row) => {
          const remarks = (row.remarks || [])
              .map(
                  (item) =>
                      `<li><b>${esc(item.studentName)}</b>（${num(item.score)}/${num(item.maxScore)}）：${esc(item.remark)}</li>`,
              )
              .join('');
          return `<div class="q">
  <div class="q-head">
    <span class="q-no">第 ${row.ord + 1} 题</span>
    <span class="q-tag">${esc(row.typeLabel)}</span>
    <span class="q-tag">${num(row.score)} 分</span>
    <span class="q-rate ${rateClass(row.rate)}">错 <b>${row.wrong}</b> 人${row.partial ? `（其中 ${row.partial} 人部分分）` : ''}${
              row.blank ? ` · 空答 ${row.blank} 人` : ''
          } · 得分率 <b>${pct(row.rate)}</b></span>
  </div>
  <div class="q-stem">${esc(plain(row.stem))}</div>
  ${row.answer ? `<div class="q-answer"><span class="label">参考答案：</span>${esc(plain(row.answer))}</div>` : ''}
  ${row.wrongStudents.length ? `<p class="names"><span class="label">错的人：</span>${esc(row.wrongStudents.join('、'))}</p>` : '<p class="names muted">这道题没人错。</p>'}
  ${remarks ? `<div class="remark"><span class="label">批改备注：</span><ul>${remarks}</ul></div>` : ''}
  ${row.pending ? `<p class="names muted">还有 ${row.pending} 人没批。</p>` : ''}
</div>`;
      })
      .join('')}`
        : '';

    const coverBlock =
        missing.length || pendingStudents.length
            ? `<h2>还没到齐</h2>
${missing.length ? `<p><span class="label">未交（${missing.length} 人）：</span>${esc(missing.map((row) => row.name).join('、'))}</p>` : ''}
${pendingStudents.length ? `<p><span class="label">待批（${pendingStudents.length} 人）：</span>${esc(pendingStudents.map((row) => row.name).join('、'))}</p>` : ''}`
            : '';

    return `<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>${esc(sheet.className)} · ${esc(sheet.title)} · 讲评清单</title>
<style>${STYLE}</style>
</head>
<body>
<h1>${esc(sheet.title)} · 讲评清单</h1>
<p class="sub">${esc(sheet.className)}${sheet.subjectName ? ` · ${esc(sheet.subjectName)}` : ''}${
        sheet.dueAt ? ` · 截止 ${esc(String(sheet.dueAt).replace('T', ' ').slice(0, 16))}` : ''
    } · 生成于 ${esc(String(sheet.generatedAt).replace('T', ' ').slice(0, 16))}</p>
${stats}
${topBlock}
${overview}
${detail}
${coverBlock}
<p class="print-tip">在浏览器里按 ⌘P（Windows 用 Ctrl+P）打印，或「另存为 PDF」。</p>
</body>
</html>`;
}

/** 讲评清单导出 CSV：一行一题，方便老师自己再做统计。 */
export function buildReviewCsv(sheet) {
    const header = ['题号', '题型', '难度', '知识点', '满分', '错人数', '部分分人数', '空答', '未批', '得分率'];
    const lines = [header.join(',')];
    (sheet.questions || []).forEach((row) => {
        const cells = [
            row.ord + 1,
            row.typeLabel,
            row.difficulty,
            (row.knowledge || []).join('／'),
            num(row.score),
            row.wrong,
            row.partial,
            row.blank,
            row.pending,
            row.rate == null ? '' : `${Math.round(row.rate * 100)}%`,
        ];
        lines.push(
            cells
                .map((cell) => {
                    const text = String(cell ?? '');
                    return /[",\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
                })
                .join(','),
        );
    });
    return `\ufeff${lines.join('\n')}`;
}
