/**
 * 卷面排印：把 PaperDoc 排成一组 A4 页面，输出**自包含 HTML**。
 *
 * 两条硬约束（见 docs/product/local-paper-management-plan.md §8）：
 * 1. 公式与阅读端同一套（splitLooseMath → KaTeX，失败兜底成纯文本），
 *    屏幕上什么样，打印稿就什么样，不允许红色报错进纸面；
 * 2. 答案页强制新开一页：正文最后一个分页结束后另起 `.paper-page`，
 *    哪怕上一页只剩半页空白也不接排。
 *
 * 分页靠真实字体测量（隐藏容器 + getBoundingClientRect），
 * 不依赖 CSS `counter(page)`——WKWebView 对它的支持并不完整。
 */

import katex from 'katex';
import {assetData} from '../api.js';
import {splitLooseMath} from '../latexMath.js';
import {latexToPlain} from '../latexPlain.js';
import {DEFAULT_ANSWER_SHEET_CONFIG} from './answerSheet.js';
import {contentBox, fullSectionTitle, mergeLayout, paperSize} from './model.js';
import {scoreText} from './score.js';

const PX_PER_MM = 96 / 25.4;
const HEADER_MM = 12;
const FOOTER_MM = 10;
const GUTTER_MM = 6;
/** 预览里两张纸之间的间隙（mm）；iframe 高度必须把这段算进去，否则最后一页会被裁掉。 */
export const PREVIEW_PAGE_GAP_MM = 6;
/** 一个选项至少要占这么宽（mm），否则「A．以上都不对」会折成两三行。 */
const MIN_OPTION_MM = 26;
const IMAGE_RE = /!\[([^\]]*)\]\(([^)]+)\)/g;

/** 一栏的可用宽度（mm）：选项列数要按它收敛，窄栏里硬塞 4 列会挤成竖排。 */
function columnWidthMm(layout) {
    const box = contentBox(layout);
    const columns = Math.max(1, Number(layout.columns) || 1);
    return (box.width - (columns - 1) * GUTTER_MM) / columns;
}

/**
 * 选项排几列：先按用户设置，再让栏宽兜底，最后避免「孤零零一个选项」。
 * 4 个选项排 3 列会剩一个 D 独占一行，这种情况宁可 2×2。
 */
function optionColumnsFor(requested, room, count) {
    const cap = Math.max(1, Math.min(requested, room, count));
    let best = 1;
    let bestRows = count;
    let bestSlack = 0;
    for (let columns = 1; columns <= cap; columns += 1) {
        const rows = Math.ceil(count / columns);
        const slack = rows * columns - count;
        const better =
            rows < bestRows ||
            (rows === bestRows && slack < bestSlack) ||
            (rows === bestRows && slack === bestSlack && columns > best);
        if (better) {
            best = columns;
            bestRows = rows;
            bestSlack = slack;
        }
    }
    return best;
}

function esc(value) {
    return String(value ?? '')
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;');
}

/** 一段文字里的公式逐片渲染；坏公式退成可读纯文本，绝不把裸反斜杠印在卷面上。 */
export function mathHtml(text) {
    let fallbacks = 0;
    const html = splitLooseMath(String(text ?? ''))
        .map((part) => {
            if (part.latex === undefined) {
                return esc(part.text).replace(/\n/g, '<br/>');
            }
            try {
                const rendered = katex.renderToString(part.latex, {
                    throwOnError: true,
                    displayMode: part.display,
                    strict: false,
                    trust: false,
                    output: 'html',
                });
                return `<span class="math-${part.display ? 'block' : 'inline'}">${rendered}</span>`;
            } catch {
                fallbacks += 1;
                return `<span class="math-plain">${esc(latexToPlain(part.latex))}</span>`;
            }
        })
        .join('');
    return {html, fallbacks};
}

/** 正文：`![](nbasset:id)` 换成内嵌图片，其余走公式渲染。 */
export function richHtml(text, assets) {
    const source = String(text ?? '');
    let last = 0;
    let fallbacks = 0;
    let missing = 0;
    const chunks = [];
    IMAGE_RE.lastIndex = 0;
    let match;
    while ((match = IMAGE_RE.exec(source)) !== null) {
        const target = match[2].trim();
        const id = target.startsWith('nbasset:') ? target.slice(8) : '';
        if (!id) continue;
        if (match.index > last) {
            const rendered = mathHtml(source.slice(last, match.index));
            chunks.push(rendered.html);
            fallbacks += rendered.fallbacks;
        }
        const data = assets.get(id);
        if (data) {
            chunks.push(
                `<img class="q-img" src="${data}" alt="${esc(match[1] || '')}"/>`,
            );
        } else {
            missing += 1;
            chunks.push(`<span class="math-plain">[图片缺失]</span>`);
        }
        last = match.index + match[0].length;
    }
    if (last < source.length) {
        const rendered = mathHtml(source.slice(last));
        chunks.push(rendered.html);
        fallbacks += rendered.fallbacks;
    }
    return {html: chunks.join(''), fallbacks, missing};
}

export function collectAssetIds(doc) {
    const ids = new Set();
    const scan = (text) => {
        const source = String(text ?? '');
        IMAGE_RE.lastIndex = 0;
        let match;
        while ((match = IMAGE_RE.exec(source)) !== null) {
            const target = match[2].trim();
            if (target.startsWith('nbasset:')) ids.add(target.slice(8));
        }
    };
    for (const section of doc.sections || []) {
        for (const item of section.questions || []) {
            const snapshot = item.snapshot || {};
            scan(snapshot.stem);
            for (const option of snapshot.options || []) scan(option.text);
            scan(snapshot.analysis);
            for (const asset of snapshot.assets || []) {
                if (asset?.id) ids.add(asset.id);
            }
        }
    }
    return [...ids];
}

const assetCache = new Map();

async function loadAssets(ids) {
    const map = new Map();
    await Promise.all(
        ids.map(async (id) => {
            if (assetCache.has(id)) {
                map.set(id, assetCache.get(id));
                return;
            }
            try {
                const data = await assetData(id);
                assetCache.set(id, data);
                map.set(id, data);
            } catch {
                // 图片丢了就印「[图片缺失]」，不让整卷渲染失败。
            }
        }),
    );
    return map;
}

// ---------------------------------------------------------------------------
// 卷面样式（同时注入应用内用于测量，和写进导出 HTML）
// ---------------------------------------------------------------------------

const STYLE_ID = 'paper-print-css';

export const PAPER_CSS = `
.paper-scope, .paper-doc { font-family: "Songti SC", "SimSun", "Source Han Serif SC", "Noto Serif CJK SC", serif; color: #000; }
.paper-scope { font-size: 10.5pt; line-height: 1.6; }
.paper-doc { margin: 0; padding: 0; background: #fff; }
.paper-scope *, .paper-doc * { box-sizing: border-box; }
.paper-page { position: relative; overflow: hidden; background: #fff; page-break-after: always; break-after: page; }
.paper-page:last-child { page-break-after: auto; break-after: auto; }
.paper-head { display: flex; align-items: flex-end; justify-content: space-between; border-bottom: 0.35mm solid #000; padding-bottom: 1.5mm; font-size: 9pt; }
.paper-head .head-title { font-weight: 700; font-size: 11pt; }
.paper-body { overflow: hidden; }
.paper-body.cols { display: flex; gap: ${GUTTER_MM}mm; }
.paper-col { flex: 1 1 0; min-width: 0; }
.paper-foot { position: absolute; left: 0; right: 0; text-align: center; font-size: 9pt; color: #222; }
.section-head { padding: 2mm 0 2mm; }
.section-title { font-weight: 700; font-size: 11.5pt; }
.section-note { font-size: 9.5pt; margin-top: 0.5mm; }
.q { padding-bottom: var(--q-gap, 6mm); }
.q-stem { line-height: var(--q-line, 1.6); }
.q-no { font-weight: 700; margin-right: 0.5mm; }
.q-score { font-size: 9pt; }
.q-options { display: grid; gap: 0.5mm 4mm; margin-top: 1mm; }
.q-option { display: flex; gap: 1mm; align-items: baseline; min-width: 0; }
.q-option-text { min-width: 0; }
.q-option-key { font-weight: 600; }
.q-img { display: block; max-width: 100%; max-height: 90mm; margin: 1.5mm 0; }
.math-block { display: block; text-align: center; margin: 1mm 0; }
.math-inline { display: inline; }
.math-plain { font-family: "Times New Roman", serif; }
.ans-space { width: 100%; }
.ans-lines { background-image: repeating-linear-gradient(to bottom, transparent 0, transparent calc(1em - 0.2mm), #d9d9d9 calc(1em - 0.2mm), #d9d9d9 1em); }
.q-answer, .q-analysis { margin-top: 1mm; font-size: 9.5pt; line-height: 1.5; }
.q-label { font-weight: 700; }
.answer-title { font-size: 14pt; font-weight: 700; text-align: center; padding: 2mm 0 4mm; }
.watermark { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; pointer-events: none; z-index: 0; }
.watermark span { font-size: 34pt; color: rgba(0, 0, 0, 0.07); transform: rotate(-28deg); white-space: nowrap; }
.binding-line { position: absolute; top: 0; bottom: 0; border-left: 0.3mm dashed #999; }
.binding-text { position: absolute; font-size: 8pt; color: #888; letter-spacing: 2mm; }
.sheet-head { font-size: 12pt; font-weight: 700; text-align: center; margin-bottom: 2mm; }
.sheet-meta { font-size: 9.5pt; display: flex; justify-content: space-between; margin-bottom: 3mm; flex-wrap: wrap; gap: 2mm 6mm; }
.sheet-notice { font-size: 8.5pt; line-height: 1.5; border: 0.3mm solid #555; border-radius: 1mm; padding: 2mm 3mm; margin-bottom: 4mm; }
.sheet-section { margin-bottom: 4mm; }
.sheet-title { font-weight: 700; font-size: 10.5pt; margin-bottom: 1.5mm; }
.sheet-grid { display: grid; grid-template-columns: repeat(2, 1fr); gap: 1.5mm 6mm; font-size: 9.5pt; }
.sheet-item { display: flex; align-items: center; gap: 1.5mm; }
.sheet-no { width: 8mm; text-align: right; }
.sheet-bubbles { display: flex; gap: 1.2mm; }
.bubble { width: 4.6mm; height: 4.6mm; border: 0.3mm solid #333; border-radius: 50%; font-size: 6.5pt; line-height: 4mm; text-align: center; }
.sheet-answer-item { margin-bottom: 2mm; break-inside: avoid; }
.sheet-item-head { display: flex; align-items: baseline; justify-content: space-between; gap: 2mm; font-size: 9.5pt; margin-bottom: 1mm; }
.sheet-score { font-size: 8.5pt; color: #333; white-space: nowrap; }
.sheet-box { border: 0.3mm solid #333; height: 22mm; margin-bottom: 1.5mm; }
.sheet-lines {
    border: 0.3mm solid #333;
    margin-bottom: 1.5mm;
    background-image: repeating-linear-gradient(to bottom, transparent 0, transparent calc(6mm - 0.2mm), #d0d0d0 calc(6mm - 0.2mm), #d0d0d0 6mm);
}
`;

/**
 * 预览专用的显示样式：与打印样式一致，但去掉分页（page-break-after: always）。
 * 预览走 iframe + srcDoc，WKWebView 在分页媒体（@page / page-break-after: always）下会把
 * iframe 内的文档只渲染第一页；预览需要完整的页面序列，因此预览样式不带这两类指令，
 * 完整分页仍由打印路径的 printHtml（带 @page + 分页 + KaTeX）承担，走系统浏览器打印。
 */
export const PAPER_PREVIEW_CSS = `${PAPER_CSS.replace(
    /\.paper-page \{[^}]*page-break-after: always;\s*break-after: page;\s*\}/,
    '.paper-page { position: relative; overflow: hidden; background: #fff; }',
)}
/* 预览专用：纸张之间留一道灰缝，几页卷子不会糊成一张长纸。
   间隙高度必须与 PaperPreview 的 iframe 高度计算一致。 */
.paper-doc { background: #e4e7ed; }
.paper-page { margin: 0 auto; box-shadow: 0 0 0 0.2mm #cfd3db; }
.paper-page + .paper-page { margin-top: ${PREVIEW_PAGE_GAP_MM}mm; }`;

/**
 * KaTeX 样式直接从已加载的样式表里取：构建时字体已按 assetsInlineLimit 内联成
 * data URL，取出来就是自包含的，不必再 import 一份（会白白多打 1.5MB 进 JS）。
 */
let katexCssCache = null;
function katexCss() {
    if (katexCssCache !== null) return katexCssCache;
    let text = '';
    if (typeof document !== 'undefined') {
        for (const sheet of document.styleSheets) {
            try {
                const rules = [...sheet.cssRules].map((rule) => rule.cssText).join('\n');
                if (rules.includes('.katex')) text += `${rules}\n`;
            } catch {
                // 跨域样式表读不到 cssRules，跳过——应用内样式都是同源的。
            }
        }
    }
    katexCssCache = text;
    return text;
}

function ensureStyle() {
    if (typeof document === 'undefined') return;
    if (document.getElementById(STYLE_ID)) return;
    const style = document.createElement('style');
    style.id = STYLE_ID;
    style.textContent = PAPER_CSS;
    document.head.appendChild(style);
}

// ---------------------------------------------------------------------------
// 题目块
// ---------------------------------------------------------------------------

const OBJECTIVE_TYPES = new Set(['SINGLE', 'MULTI', 'JUDGE']);

/**
 * 作答留白行数。「解答题留白」设置只作用于解答题类（SOLVED / CALC / PROOF）；
 * 客观题（单选/多选/判断）永远不留白，填空固定 3 行——都不吃这个设置。
 */
function answerLines(kind, layout) {
    if (OBJECTIVE_TYPES.has(kind)) return 0;
    if (kind === 'FILL') return 3;
    if (typeof layout.answerSpace === 'number') return layout.answerSpace;
    return 8;
}

function optionsHtml(snapshot, layout, assets, colWidth = 0) {
    const options = snapshot.options || [];
    if (!options.length) return {html: '', fallbacks: 0};
    const requested = Number(layout.optionColumns) || 4;
    // 栏宽不够就少排几列：宁可两列长一点，也不要把选项压成逐字换行。
    const room = colWidth > 0 ? Math.max(1, Math.floor(colWidth / MIN_OPTION_MM)) : requested;
    const columns = optionColumnsFor(requested, room, options.length);
    let fallbacks = 0;
    const body = options
        .map((option) => {
            const rendered = richHtml(option.text, assets);
            fallbacks += rendered.fallbacks;
            return `<div class="q-option"><span class="q-option-key">${esc(
                option.key,
            )}．</span><span class="q-option-text">${rendered.html}</span></div>`;
        })
        .join('');
    // 列数走内联 grid 模板：1/2/3/4 列都可能出现（见上面的栏宽收敛），
    // 不要用 `.cols-2` 这类固定类名——漏一个类名整排选项就退化成逐行堆叠。
    return {
        html: `<div class="q-options" data-cols="${columns}" style="grid-template-columns:repeat(${columns}, minmax(0, 1fr))">${body}</div>`,
        fallbacks,
    };
}

function questionBlock(snapshot, number, score, layout, variant, assets, colWidth = 0) {
    const stem = richHtml(snapshot.stem, assets);
    let fallbacks = stem.fallbacks;
    let missing = stem.missing;
    const scoreTag = score > 0 ? `<span class="q-score">（${scoreText(score)} 分）</span>` : '';
    let html = `<div class="q"><div class="q-stem"><span class="q-no">${number}．</span>${stem.html}${scoreTag}</div>`;
    const options = optionsHtml(snapshot, layout, assets, colWidth);
    fallbacks += options.fallbacks;
    html += options.html;
    const lines = answerLines(snapshot.type, layout);
    // 作答区只留空白不画横线：高度仍按行数走，打印出来就是一段干净空白。
    if (variant === 'STUDENT' && lines > 0) {
        html += `<div class="ans-space" style="height:${(lines * (Number(layout.lineHeight) || 1.6)).toFixed(
            2,
        )}em"></div>`;
    }
    if (variant === 'TEACHER') {
        const answer = richHtml(snapshot.answer, assets);
        const analysis = richHtml(snapshot.analysis, assets);
        fallbacks += answer.fallbacks + analysis.fallbacks;
        missing += answer.missing + analysis.missing;
        if (answer.html.trim()) {
            html += `<div class="q-answer"><span class="q-label">【答案】</span>${answer.html}</div>`;
        }
        if (analysis.html.trim()) {
            html += `<div class="q-analysis"><span class="q-label">【解析】</span>${analysis.html}</div>`;
        }
    }
    html += '</div>';
    return {html, fallbacks, missing};
}

/** 题目数量（含大题下的题），卷面题号从头到尾连续。 */
export function questionCount(doc) {
    return (doc.sections || []).reduce((sum, section) => sum + (section.questions || []).length, 0);
}

function buildMainBlocks(doc, variant, layout, assets) {
    const blocks = [];
    let number = 0;
    let fallbacks = 0;
    let missing = 0;
    const colWidth = columnWidthMm(layout);
    (doc.sections || []).forEach((section, index) => {
        if (!(section.questions || []).length) return;
        blocks.push({
            kind: 'section',
            keepWithNext: true,
            html: `<div class="section-head"><div class="section-title">${esc(
                fullSectionTitle(section, index),
            )}</div>${section.note ? `<div class="section-note">${esc(section.note)}</div>` : ''}</div>`,
        });
        for (const item of section.questions) {
            number += 1;
            const built = questionBlock(
                item.snapshot || {},
                number,
                Number(item.score || 0),
                layout,
                variant,
                assets,
                colWidth,
            );
            fallbacks += built.fallbacks;
            missing += built.missing;
            blocks.push({kind: 'question', html: built.html});
        }
    });
    return {blocks, fallbacks, missing, count: number};
}

function buildAnswerBlocks(doc, layout, assets) {
    const blocks = [
        {kind: 'title', html: '<div class="answer-title">参考答案与解析</div>'},
    ];
    let number = 0;
    let fallbacks = 0;
    (doc.sections || []).forEach((section, index) => {
        if (!(section.questions || []).length) return;
        blocks.push({
            kind: 'section',
            keepWithNext: true,
            html: `<div class="section-head"><div class="section-title">${esc(
                fullSectionTitle(section, index),
            )}</div></div>`,
        });
        for (const item of section.questions) {
            number += 1;
            const snapshot = item.snapshot || {};
            const answer = richHtml(snapshot.answer, assets);
            const analysis = richHtml(snapshot.analysis, assets);
            fallbacks += answer.fallbacks + analysis.fallbacks;
            const body = [
                answer.html.trim() ? `<div class="q-answer"><span class="q-label">【答案】</span>${answer.html}</div>` : '',
                analysis.html.trim() ? `<div class="q-analysis"><span class="q-label">【解析】</span>${analysis.html}</div>` : '',
            ].join('');
            blocks.push({
                kind: 'question',
                html: `<div class="q"><div class="q-stem"><span class="q-no">${number}．</span>${body || '<span class="math-plain">（暂无答案）</span>'}</div></div>`,
            });
        }
    });
    return {blocks, fallbacks};
}

function normalizeAnswerType(type) {
    const value = String(type || '').toUpperCase();
    return value === 'CALC' ? 'SOLVED' : value;
}

/** 把卷内题目摊平成答题卡条目，题号与卷面一致。 */
function answerSheetEntries(doc) {
    const entries = [];
    let number = 0;
    (doc.sections || []).forEach((section) => {
        if (!(section.questions || []).length) return;
        for (const item of section.questions) {
            number += 1;
            entries.push({
                number,
                type: normalizeAnswerType(item.snapshot?.type),
                sectionKind: normalizeAnswerType(section.kind),
                score: Number(item.score) || 0,
                snapshot: item.snapshot || {},
            });
        }
    });
    return entries;
}

function ruleMatches(entry, rule) {
    const types = (rule.types || []).map(normalizeAnswerType);
    const kinds = (rule.sectionKinds || []).map(normalizeAnswerType);
    return types.includes(entry.type) || kinds.includes(entry.sectionKind);
}

/** 答题卡上不印题干：只保留题号、可选分值和作答区，与真实答题卡一致。 */
function answerItemHtml(entry, rule) {
    const height = Math.max(12, Math.min(160, Number(rule.heightMm) || 36));
    const style = rule.style === 'lines'
        ? `class="sheet-lines" style="height:${height}mm"`
        : `class="sheet-box" style="height:${height}mm"`;
    const score = entry.score > 0 ? `<span class="sheet-score">${scoreText(entry.score)} 分</span>` : '';
    return `<div class="sheet-answer-item">
        <div class="sheet-item-head"><span class="sheet-no">${entry.number}.</span>${score}</div>
        <div ${style}></div>
    </div>`;
}

function clampBubbleColumns(entries, layout, pageColumns, requested) {
    const box = contentBox(layout);
    const pageCols = Math.max(1, Math.min(2, pageColumns || 1));
    const columnWidth = (box.width - GUTTER_MM * (pageCols - 1)) / pageCols;
    const options = Math.max(
        4,
        ...entries.map((entry) => (entry.snapshot?.options || []).length),
    );
    // 题号 + 选项气泡 + 间距的保守宽度，再按栏宽收敛；宁可少一排也不横向溢出。
    const itemWidth = 8 + options * 4.6 + Math.max(0, options - 1) * 1.2 + 1.5;
    return Math.max(1, Math.min(requested, Math.floor((columnWidth + GUTTER_MM) / (itemWidth + GUTTER_MM))));
}

function buildAnswerSheetBlocks(doc, answerSheet, layout) {
    const sheet = answerSheet?.rules?.length ? answerSheet : DEFAULT_ANSWER_SHEET_CONFIG;
    const fields = sheet.fields?.length ? sheet.fields : ['班级', '姓名', '学号'];
    // 答题卡上不印题目信息：标题固定为「答题卡」（不是试卷名），
    // 只保留卷头填写栏 + 题号 + 作答区；须知与分区题型标题一并去掉。
    const blocks = [
        {
            kind: 'head',
            html: `<div class="sheet-head">答题卡</div>
                <div class="sheet-meta">${fields
                    .map((field) => `<span>${esc(field)}：____________</span>`)
                    .join('')}</div>`,
        },
    ];
    const entries = answerSheetEntries(doc);
    const used = new Set();
    const rules = sheet.rules || [];

    for (const rule of rules) {
        const picked = entries.filter((entry) => !used.has(entry.number) && ruleMatches(entry, rule));
        if (!picked.length) continue;
        picked.forEach((entry) => used.add(entry.number));

        if (rule.style === 'bubble') {
            const requested = Math.max(1, Math.min(8, Number(rule.columns) || 4));
            const columns = clampBubbleColumns(picked, layout, sheet.pageColumns, requested);
            const rows = picked.map((entry) => {
                const keys = (entry.snapshot?.options || []).map((option) => option.key);
                const bubbles = (keys.length ? keys : ['A', 'B', 'C', 'D'])
                    .map((key) => `<span class="bubble">${esc(key)}</span>`)
                    .join('');
                return `<div class="sheet-item"><span class="sheet-no">${entry.number}.</span><span class="sheet-bubbles">${bubbles}</span></div>`;
            });
            // 涂卡行按 12 行分块，避免一个超长块把整页撑破。
            const chunk = 12;
            for (let start = 0; start < rows.length; start += chunk) {
                const slice = rows.slice(start, start + chunk).join('');
                blocks.push({
                    kind: 'section',
                    html: `<div class="sheet-section"><div class="sheet-grid" style="grid-template-columns:repeat(${columns}, minmax(0, 1fr))">${slice}</div></div>`,
                });
            }
            continue;
        }

        // 答题框/横线区直接跟第一题，避免被分栏拆散。
        blocks.push({
            kind: 'section',
            html: `<div class="sheet-section">${answerItemHtml(picked[0], rule)}</div>`,
        });
        picked.slice(1).forEach((entry) => {
            blocks.push({kind: 'question', html: answerItemHtml(entry, rule)});
        });
    }

    const leftovers = entries.filter((entry) => !used.has(entry.number));
    if (leftovers.length) {
        blocks.push({
            kind: 'section',
            html: `<div class="sheet-section">${answerItemHtml(
                leftovers[0],
                {style: 'box', heightMm: 36},
            )}</div>`,
        });
        leftovers.slice(1).forEach((entry) => {
            blocks.push({
                kind: 'question',
                html: answerItemHtml(entry, {style: 'box', heightMm: 36}),
            });
        });
    }

    return {blocks};
}

// ---------------------------------------------------------------------------
// 测量与分页
// ---------------------------------------------------------------------------

async function measureBlocks(blocks, layout) {
    ensureStyle();
    const box = contentBox(layout);
    const columns = Math.max(1, Math.min(2, Number(layout.columns) || 1));
    const width = (box.width - GUTTER_MM * (columns - 1)) / columns;
    const host = document.createElement('div');
    host.className = 'paper-scope paper-measure';
    host.setAttribute('aria-hidden', 'true');
    host.style.cssText = `position:fixed;left:-20000px;top:0;visibility:hidden;width:${width}mm;font-size:${layout.fontSize}pt;line-height:${layout.lineHeight};--q-gap:${layout.questionGap}mm;--q-line:${layout.lineHeight};`;
    host.innerHTML = blocks
        .map((block, index) => `<div data-i="${index}">${block.html}</div>`)
        .join('');
    document.body.appendChild(host);
    try {
        if (document.fonts?.ready) {
            try {
                await document.fonts.ready;
            } catch {
                // 字体没就绪就按当前字体量，最多差一点行距。
            }
        }
        await Promise.all(
            [...host.querySelectorAll('img')].map((img) =>
                img.decode ? img.decode().catch(() => {}) : Promise.resolve(),
            ),
        );
        return [...host.children].map(
            (element) => element.getBoundingClientRect().height / PX_PER_MM,
        );
    } finally {
        host.remove();
    }
}

function paginate(blocks, heights, layout) {
    const box = contentBox(layout);
    const columns = Math.max(1, Math.min(2, Number(layout.columns) || 1));
    const bodyHeight = box.height - HEADER_MM - FOOTER_MM;
    const pages = [];
    let page = [];
    let column = {height: 0, items: []};
    // 换栏：当前栏收进本页；页满就把页推走，空栏留给下一页（不能把空栏也塞进旧页，
    // 否则单栏卷会变成「半页正文 + 半页空白」）。
    const nextColumn = () => {
        page.push(column);
        column = {height: 0, items: []};
        if (page.length >= columns) {
            pages.push(page);
            page = [];
        }
    };
    blocks.forEach((block, index) => {
        const height = Math.min(heights[index] || 0, bodyHeight);
        const lookahead = block.keepWithNext ? height + (heights[index + 1] || 0) : height;
        if (column.items.length && column.height + lookahead > bodyHeight) nextColumn();
        if (column.items.length && column.height + height > bodyHeight) nextColumn();
        column.items.push(index);
        column.height += height;
    });
    if (column.items.length || page.length) {
        page.push(column);
        pages.push(page);
    }
    return pages;
}

// ---------------------------------------------------------------------------
// 组装
// ---------------------------------------------------------------------------

function pageHtml({title, layout, columns, headerRight, bodyHtml, pageNo, total, watermark}) {
    const box = contentBox(layout);
    const size = paperSize(layout);
    const margins = layout.margins;
    const bodyHeight = box.height - HEADER_MM - FOOTER_MM;
    const cols = columns.length;
    const inner = columns
        .map((column) => `<div class="paper-col">${column}</div>`)
        .join('');
    const watermarkHtml = watermark
        ? `<div class="watermark"><span>${esc(watermark)}</span></div>`
        : '';
    const left = margins.left + box.binding;
    const bindingHtml = layout.bindingLine
        ? `<div class="binding-line" style="left:6mm"></div><div class="binding-text" style="left:12mm;top:45%;transform:rotate(90deg);transform-origin:left top">装订线内不要答题</div>`
        : '';
    const pageCss = `width:${size.width}mm;height:${size.height}mm;padding:${margins.top}mm ${margins.right}mm ${margins.bottom}mm ${left}mm;`;
    return `<section class="paper-page" style="${pageCss}">
${watermarkHtml}${bindingHtml}
<div class="paper-head" style="height:${HEADER_MM}mm"><span class="head-title">${esc(title)}</span><span>${esc(
        headerRight,
    )}</span></div>
<div class="paper-body ${cols > 1 ? 'cols' : ''}" style="height:${bodyHeight}mm;--q-gap:${layout.questionGap}mm;--q-line:${layout.lineHeight};font-size:${layout.fontSize}pt;line-height:${layout.lineHeight}">${inner}</div>
<div class="paper-foot" style="bottom:${Math.max(
        4,
        margins.bottom / 2,
    )}mm">第 ${pageNo} 页 / 共 ${total} 页</div>
</section>`;
}

/**
 * 主入口：渲染整套卷子。
 * @returns {{html: string, pages: number, warnings: string[]}}
 */
export async function renderPaperHtml(doc, variant = 'STUDENT', options = {}) {
    const layout = mergeLayout(options.layout || doc.layout);
    ensureStyle();
    const assets = await loadAssets(collectAssetIds(doc));
    const warnings = [];

    const main = buildMainBlocks(doc, variant, layout, assets);
    if (main.fallbacks > 0) {
        warnings.push(`${main.fallbacks} 处公式无法排版，已按纯文本输出`);
    }
    if (main.missing > 0) {
        warnings.push(`${main.missing} 张图片缺失，卷面上已标注`);
    }
    const mainHeights = await measureBlocks(main.blocks, layout);
    const mainPages = paginate(main.blocks, mainHeights, layout);

    const answerLayout = {...layout, columns: 1};
    let answerPages = [];
    let answerBlocks = [];
    if (variant === 'TEACHER' && layout.answerKeyPage) {
        const built = buildAnswerBlocks(doc, answerLayout, assets);
        answerBlocks = built.blocks;
        answerPages = paginate(answerBlocks, await measureBlocks(answerBlocks, answerLayout), answerLayout);
    }

    // 答题卡按模板自己的栏数排（默认 2 栏），纸型跟主卷一致：
    // 一份 HTML 只能有一个 @page 尺寸，混纸型打印会把某一种打歪。
    const answerSheet = options.answerSheet?.rules?.length
        ? options.answerSheet
        : DEFAULT_ANSWER_SHEET_CONFIG;
    const sheetColumns = Math.max(
        1,
        Math.min(2, Number(answerSheet.pageColumns) || Number(layout.answerSheet?.columns) || 2),
    );
    const sheetLayout = {...layout, columns: sheetColumns, answerSpace: 0};
    let sheetPages = [];
    let sheetBlocks = [];
    if (layout.answerSheet?.enabled) {
        const built = buildAnswerSheetBlocks(doc, answerSheet, sheetLayout);
        sheetBlocks = built.blocks;
        sheetPages = paginate(sheetBlocks, await measureBlocks(sheetBlocks, sheetLayout), sheetLayout);
    }

    const total = mainPages.length + answerPages.length + sheetPages.length;
    const parts = [];
    mainPages.forEach((columns, index) => {
        parts.push(
            pageHtml({
                title: doc.title,
                layout,
                columns: columns.map((column) =>
                    column.items.map((item) => main.blocks[item].html).join(''),
                ),
                headerRight: variant === 'TEACHER' ? '教师版' : '班级：________ 姓名：________',
                pageNo: index + 1,
                total,
                watermark: layout.watermark,
            }),
        );
    });
    answerPages.forEach((columns, index) => {
        parts.push(
            pageHtml({
                title: doc.title,
                layout: answerLayout,
                columns: columns.map((column) =>
                    column.items.map((item) => answerBlocks[item].html).join(''),
                ),
                headerRight: '参考答案',
                pageNo: mainPages.length + index + 1,
                total,
                watermark: layout.watermark,
            }),
        );
    });
    sheetPages.forEach((columns, index) => {
        parts.push(
            pageHtml({
                title: doc.title,
                layout: sheetLayout,
                columns: columns.map((column) =>
                    column.items.map((item) => sheetBlocks[item].html).join(''),
                ),
                headerRight: '答题卡',
                pageNo: mainPages.length + answerPages.length + index + 1,
                total,
                watermark: layout.watermark,
            }),
        );
    });

    const size = paperSize(layout);
    // 预览用不带分页的显示样式（完整页面序列），打印用带 @page + 分页的样式（由系统浏览器分页）。
    const printCss = `@page { size: ${size.width}mm ${size.height}mm; margin: 0; }\n${PAPER_CSS}\n${katexCss()}`;
    const css = `\n${PAPER_PREVIEW_CSS}\n${katexCss()}`;
    const paperHtml = (style) => `<!doctype html>
    <html lang="zh-CN">
    <head>
    <meta charset="utf-8"/>
    <title>${esc(doc.title || '试卷')}</title>
    <style>${style}</style>
    </head>
    <body class="paper-doc">
    ${parts.join('\n')}
    </body>
    </html>`;
    const html = paperHtml(css);
    const printHtml = paperHtml(printCss);
    return {html, printHtml, pages: total, warnings};
}

/** 供编辑器里做「大概几页」的轻量提示：不测量，按题量估。 */
export function estimatePages(doc) {
    const count = questionCount(doc);
    return Math.max(1, Math.ceil(count / 10));
}
