/**
 * 家长版学情周报：把「掌握度 + 薄弱点 + 本周安排 + 作业考试」压成一页可打印的纸。
 *
 * 为什么在浏览器里拼 HTML 而不是后端出 PDF：
 * 打印通道已经有现成的（自包含 HTML → 系统浏览器 → ⌘P 存 PDF），排版与预览是同一份，
 * 不会出现「屏幕上挺好、打出来错位」。这里只负责把数据翻译成静态 HTML，不引第三方库。
 *
 * 三条底线：
 * - 所有用户输入（姓名、知识点、作业标题）都过 `esc`，宁可少一个样式也不要坏一段 HTML；
 * - 不出现题目原文、不出现聊天记录，只放数字和结论（与 AI 诊断的护栏一致）；
 * - 没有数据就明确写「本周没有可分析的作答」，不拿 0 分凑数。
 */

function esc(raw) {
    return String(raw == null ? '' : raw)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;');
}

function num(value, digits = 0) {
    if (value == null || Number.isNaN(Number(value))) return '–';
    const factor = 10 ** digits;
    return Math.round(Number(value) * factor) / factor;
}

function today() {
    const now = new Date();
    const pad = (value) => String(value).padStart(2, '0');
    return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

const LEVEL_LABEL = {
    EMPTY: '待启动',
    CRITICAL: '重点关注',
    WATCH: '需要止损',
    STEADY: '稳步提升',
    SOLID: '稳固',
};

/** 掌握度折线：纯 SVG，85 分线画虚一点当目标线。 */
function masteryChart(points) {
    if (!points || points.length < 2) {
        return '<p class="rp-muted">作答次数还不够画曲线。</p>';
    }
    const width = 330;
    const height = 96;
    const pad = {left: 22, right: 8, top: 10, bottom: 15};
    const innerW = width - pad.left - pad.right;
    const innerH = height - pad.top - pad.bottom;
    const stepX = innerW / (points.length - 1);
    const y = (value) => pad.top + (1 - value / 100) * innerH;
    const coords = points.map((point, index) => [pad.left + index * stepX, y(point.mastery)]);

    const grid = [0, 50, 85, 100]
        .map(
            (line) =>
                `<line x1="${pad.left}" x2="${width - pad.right}" y1="${y(line)}" y2="${y(line)}" class="${
                    line === 85 ? 'rp-grid strong' : 'rp-grid'
                }"/><text x="${pad.left - 5}" y="${y(line) + 3}" text-anchor="end" class="rp-tick">${line}</text>`,
        )
        .join('');

    const path = coords.map(([x, value]) => `${x.toFixed(1)},${value.toFixed(1)}`).join(' ');
    const dots = coords
        .map(
            ([x, value], index) =>
                `<circle cx="${x.toFixed(1)}" cy="${value.toFixed(1)}" r="${index === coords.length - 1 ? 3.4 : 2.4}" class="rp-dot"/>`,
        )
        .join('');

    return `<svg viewBox="0 0 ${width} ${height}" class="rp-chart" role="img" aria-label="近八周掌握度">
        ${grid}
        <polyline points="${path}" class="rp-line"/>
        ${dots}
        <text x="${pad.left}" y="${height - 5}" class="rp-tick">${esc(points[0].week)}</text>
        <text x="${width - pad.right}" y="${height - 5}" text-anchor="end" class="rp-tick">本周 · ${points[points.length - 1].mastery}</text>
    </svg>`;
}

/** 作业（圆点）与考试（方块）混排，统一百分制；60 分画一条及格线。 */
function progressChart(points) {
    if (!points || points.length < 2) {
        return '<p class="rp-muted">作业和考试的次数还不够画趋势。</p>';
    }
    const width = 330;
    const height = 106;
    const pad = {left: 22, right: 8, top: 12, bottom: 18};
    const innerW = width - pad.left - pad.right;
    const innerH = height - pad.top - pad.bottom;
    const stepX = innerW / (points.length - 1);
    const y = (value) => pad.top + (1 - Math.min(Math.max(value, 0), 100) / 100) * innerH;

    const grid = [0, 60, 80, 100]
        .map(
            (line) =>
                `<line x1="${pad.left}" x2="${width - pad.right}" y1="${y(line)}" y2="${y(line)}" class="${
                    line === 60 ? 'rp-grid strong' : 'rp-grid'
                }"/><text x="${pad.left - 5}" y="${y(line) + 3}" text-anchor="end" class="rp-tick">${line}</text>`,
        )
        .join('');

    const coords = points.map((point, index) => [pad.left + index * stepX, y(point.percent)]);
    const path = coords.map(([x, value]) => `${x.toFixed(1)},${value.toFixed(1)}`).join(' ');
    const marks = points
        .map((point, index) => {
            const [x, value] = coords[index];
            const exam = point.kind !== 'ASSIGNMENT';
            return exam
                ? `<rect x="${(x - 3).toFixed(1)}" y="${(value - 3).toFixed(1)}" width="6" height="6" class="rp-square"/>`
                : `<circle cx="${x.toFixed(1)}" cy="${value.toFixed(1)}" r="2.8" class="rp-dot"/>`;
        })
        .join('');
    // 只标首尾与峰值：横轴全标会挤成一团。
    const peak = points.reduce((best, point, index) => (point.percent > points[best].percent ? index : best), 0);
    const peakX = coords[peak][0];
    const peakY = coords[peak][1];

    return `<svg viewBox="0 0 ${width} ${height}" class="rp-chart" role="img" aria-label="作业与考试趋势">
        ${grid}
        <polyline points="${path}" class="rp-line soft"/>
        ${marks}
        <text x="${peakX.toFixed(1)}" y="${(peakY - 7).toFixed(1)}" text-anchor="middle" class="rp-tick">最高 ${num(points[peak].percent)}</text>
        <text x="${pad.left}" y="${height - 6}" class="rp-tick">${esc(points[0].at)}</text>
        <text x="${width - pad.right}" y="${height - 6}" text-anchor="end" class="rp-tick">${esc(points[points.length - 1].at)}</text>
    </svg>`;
}

function kpi(label, value, hint) {
    return `<div class="rp-kpi-item">
        <span class="rp-kpi-label">${esc(label)}</span>
        <b>${esc(value)}</b>
        <em>${esc(hint || '')}</em>
    </div>`;
}

function listOf(items, empty) {
    if (!items || items.length === 0) return `<p class="rp-muted">${esc(empty)}</p>`;
    return `<ul class="rp-list">${items.map((item) => `<li>${item}</li>`).join('')}</ul>`;
}

function detailRows(points) {
    if (!points || points.length === 0) {
        return '<tr><td colspan="5" class="rp-muted">还没有作业或考试记录。</td></tr>';
    }
    return points
        .slice()
        .reverse()
        .slice(0, 4)
        .map((point) => {
            const exam = point.kind !== 'ASSIGNMENT';
            const kindText = exam ? (point.kind === 'QUIZ' ? '小测' : point.kind === 'UNIT' ? '单元测' : '考试') : '作业';
            const low = point.percent < 60;
            return `<tr>
                <td class="rp-dim">${esc(point.at)}</td>
                <td><span class="rp-tag ${exam ? 'exam' : ''}">${kindText}</span>${esc(point.title)}</td>
                <td class="rp-right">${num(point.score, 1)} / ${num(point.full, 0)}</td>
                <td class="rp-right ${low ? 'rp-low' : ''}">${num(point.percent)}%</td>
                <td class="rp-right rp-dim">${
                    point.rank ? `第 ${point.rank}/${point.classSize}` : point.pending > 0 ? `${point.pending} 题待批` : '—'
                }</td>
            </tr>`;
        })
        .join('');
}

const STYLE = `
:root { color-scheme: light; }
* { box-sizing: border-box; }
body {
    margin: 0; background: #eef1f5; color: #1c1f23;
    font-family: -apple-system, "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", sans-serif;
    font-size: 12px; line-height: 1.45;
}
.sheet {
    width: 210mm; min-height: 297mm; margin: 10mm auto; padding: 14mm 15mm;
    background: #fff; box-shadow: 0 2px 18px rgba(20, 30, 50, .12);
}
.rp-head { border-bottom: 2px solid #1c1f23; padding-bottom: 6px; margin-bottom: 10px; }
.rp-brand { font-size: 11px; letter-spacing: .22em; color: #6b7280; }
.rp-head h1 { margin: 2px 0 1px; font-size: 21px; letter-spacing: .02em; }
.rp-meta { color: #6b7280; font-size: 11px; }
.rp-kpi { display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; margin-bottom: 8px; }
.rp-kpi-item { border: 1px solid #dde2e8; border-radius: 4px; padding: 4px 8px; }
.rp-kpi-label { display: block; font-size: 10px; color: #6b7280; letter-spacing: .06em; }
.rp-kpi-item b { font-size: 19px; }
.rp-kpi-item em { display: block; font-style: normal; font-size: 10px; color: #6b7280; }
h2 { font-size: 11.5px; margin: 9px 0 4px; letter-spacing: .1em; color: #374151; }
h2::after { content: ""; display: block; height: 1px; background: #e5e9ef; margin-top: 4px; }
.rp-lead { font-size: 13px; margin: 0 0 2px; font-weight: 600; }
.rp-lead .rp-level { display: inline-block; font-size: 10px; font-weight: 700; color: #fff;
    background: #2f6fd0; border-radius: 3px; padding: 1px 6px; margin-right: 6px; vertical-align: 2px; }
.rp-cols { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
.rp-list { margin: 0; padding-left: 16px; }
.rp-list li { margin-bottom: 2px; }
.rp-muted { color: #9aa3af; }
.rp-cap { margin: 3px 0 0; font-size: 9.5px; color: #9aa3af; }
.rp-charts > div + div { border-left: 1px solid #eef1f5; padding-left: 12px; }
.rp-chart { width: 100%; height: auto; display: block; }
.rp-grid { stroke: #e5e9ef; stroke-width: 1; }
.rp-grid.strong { stroke: #c8d0da; stroke-dasharray: 3 3; }
.rp-line { fill: none; stroke: #2f6fd0; stroke-width: 1.8; stroke-linejoin: round; stroke-linecap: round; }
.rp-line.soft { stroke: #8aa6c8; }
.rp-dot { fill: #fff; stroke: #2f6fd0; stroke-width: 1.5; }
.rp-square { fill: #2f6fd0; }
.rp-tick { fill: #9aa3af; font-size: 9px; }
table.rp-table { width: 100%; border-collapse: collapse; font-size: 11px; }
table.rp-table th { text-align: left; font-weight: 600; color: #6b7280; border-bottom: 1px solid #dde2e8; padding: 3px 4px; }
table.rp-table td { border-bottom: 1px dashed #eef1f5; padding: 2.5px 4px; }
.rp-right { text-align: right; }
.rp-dim { color: #6b7280; }
.rp-low { color: #c0392b; font-weight: 600; }
.rp-tag { display: inline-block; font-size: 9px; padding: 0 4px; margin-right: 5px; border-radius: 3px;
    border: 1px solid #dde2e8; color: #6b7280; }
.rp-tag.exam { border-color: #b9cdea; color: #2f6fd0; background: #f2f7ff; }
.rp-plan td { padding: 2.5px 4px; border-bottom: 1px dashed #eef1f5; }
.rp-plan .day { width: 42px; color: #374151; font-weight: 600; }
.rp-plan .min { width: 46px; text-align: right; color: #6b7280; }
.rp-note { border-left: 3px solid #2f6fd0; background: #f7f9fc; padding: 6px 9px; margin-top: 8px; font-size: 11px; }
.rp-sign { margin-top: 6px; display: grid; grid-template-columns: 1fr 1fr; gap: 18px; }
.rp-sign div { border-bottom: 1px solid #dde2e8; height: 20px; font-size: 10px; color: #9aa3af; }
.rp-foot { margin-top: 5px; padding-top: 4px; border-top: 1px solid #eef1f5; font-size: 9.5px; color: #9aa3af; }
section { page-break-inside: avoid; }
@page { size: A4; margin: 10mm; }
@media print {
    body { background: #fff; font-size: 11.5px; }
    .sheet { width: auto; min-height: 0; margin: 0; padding: 0; box-shadow: none; }
}
`;

/**
 * @param {{insight: object, advice?: object, progress?: object}} data
 * @returns {string} 自包含 HTML（含内联样式与 SVG）
 */
export function buildReportHtml({insight, advice, progress}) {
    const student = insight || {};
    const plan = advice?.plan;
    const points = progress?.points || [];
    const recent = points.slice(-8);
    const kps = student.kps || [];
    const hasData = (student.answered || 0) > 0;
    const level = LEVEL_LABEL[advice?.level] || '';

    const kpis = [
        kpi('掌握度', hasData ? student.overall : '–', hasData ? `已判 ${student.answered} 题` : '还没有已判分的作答'),
        kpi(
            '班内位置',
            student.rank && student.classSize ? `${student.rank}/${student.classSize}` : '–',
            student.classAvg != null ? `班均 ${student.classAvg}` : '没有可比的同学',
        ),
        kpi('得分率', hasData ? `${num(student.scoreRate * 100)}%` : '–', '作业按题累计'),
        kpi('待复习错题', student.wrongCount ?? 0, (student.pending || 0) > 0 ? `${student.pending} 题待批` : '错题按记忆节奏提醒'),
    ].join('');

    const strengths = (advice?.strengths || []).map(
        (item) => `<b>${esc(item.point)}</b>${item.evidence ? ` · ${esc(item.evidence)}` : ''}`,
    );
    const issues = (advice?.issues || []).map(
        (item) => `<b>${esc(item.kp)}</b>${item.why ? `<div class="rp-muted">${esc(item.why)}</div>` : ''}${
            item.action ? `<div>怎么做：${esc(item.action)}</div>` : ''
        }`,
    );

    const planRows = (plan?.days || [])
        .map(
            (day) => `<tr><td class="day">${esc(day.day)}</td><td>${esc(day.task)}</td><td class="min">${day.minutes} 分</td></tr>`,
        )
        .join('');

    const masteredCount = kps.filter((kp) => kp.answered > 0 && kp.mastery >= 85).length;
    const touched = kps.filter((kp) => kp.answered > 0).length;

    return `<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>${esc(student.name || '学生')} · 学情周报</title>
<style>${STYLE}</style>
</head>
<body>
<div class="sheet">
    <header class="rp-head">
        <div class="rp-brand">学 情 周 报</div>
        <h1>${esc(student.name || '学生')}</h1>
        <div class="rp-meta">${[
            student.studentNo ? `学号 ${esc(student.studentNo)}` : '',
            esc(student.grade || '未填年级'),
            (student.classNames || []).map(esc).join(' / ') || '不在任何班级',
            `数据截至 ${today()}`,
        ]
            .filter(Boolean)
            .join(' · ')}</div>
    </header>

    <div class="rp-kpi">${kpis}</div>

    <section>
        <h2>本周一句话</h2>
        <p class="rp-lead">${level ? `<span class="rp-level">${esc(level)}</span>` : ''}${esc(
            advice?.headline || '还没有诊断结论。',
        )}</p>
        <p class="rp-muted">${esc(progress?.note || '')}</p>
    </section>

    <section class="rp-cols">
        <div>
            <h2>已经站住的地方</h2>
            ${listOf(strengths, '还没有稳定在 85 分以上的知识点。')}
        </div>
        <div>
            <h2>需要补的地方</h2>
            ${listOf(issues, '没有明显短板，保持节奏即可。')}
        </div>
    </section>

    <section class="rp-cols rp-charts">
        <div>
            <h2>掌握度走势（近 8 周）</h2>
            ${masteryChart(student.trend || [])}
            <p class="rp-cap">已练 ${touched} 个知识点，${masteredCount} 个到 85 分以上。</p>
        </div>
        <div>
            <h2>作业与考试（百分制）</h2>
            ${progressChart(recent)}

        </div>
    </section>

    <section>
        <h2>最近记录</h2>
        <table class="rp-table">
            <thead><tr><th>日期</th><th>内容</th><th class="rp-right">得分</th><th class="rp-right">得分率</th><th class="rp-right">位置</th></tr></thead>
            <tbody>${detailRows(points)}</tbody>
        </table>
    </section>

    <section>
        <h2>本周安排${plan ? ` · 共 ${plan.totalMinutes} 分钟，平均每天 ${plan.dailyMinutes} 分钟` : ''}</h2>
        <table class="rp-table rp-plan"><tbody>${planRows || '<tr><td class="rp-muted">还没有安排。</td></tr>'}</tbody></table>
    </section>

    ${
        advice?.parentNote
            ? `<section class="rp-note"><b>给家长的话　</b>${esc(advice.parentNote)}</section>`
            : ''
    }

    <div class="rp-sign">
        <div>家长反馈</div>
        <div>签字 / 日期</div>
    </div>

    <div class="rp-foot">
        掌握度按「越新的作答权重越高」计算，只统计已判分的题；薄弱点指掌握度低于 70 的知识点。
        本页由本机生成，数据不出电脑。生成时间 ${today()}。
    </div>
</div>
</body>
</html>`;
}
