// 模板 → 打印 HTML：把设计器产出的「毫米绝对定位」模板翻成 HTML，不依赖设计器本身。
// 任何服务只要拿到模板 JSON + 数据，就能自己出纸（浏览器打印 / 转 PDF），
// 与设计器画布、@worm-vue3-print/core 的出纸几何保持同一口径：
//   页眉贴内容区顶部（top = 上边距），页脚贴纸底（top = 纸高 - 下边距 - 页脚高），
//   正文坐标相对内容区左上角。
import {esc, geometry, interpolate, mm, pt} from './util.js';

/** 支持的元素类型；不在此列的（条码/二维码/流式列表）按文本降级并给告警 */
export const SUPPORTED_TYPES = [
    'text',
    'longText',
    'image',
    'hline',
    'vline',
    'rect',
    'oval',
    'html',
    'pageNumber',
    'table',
]

/** 渲染产物自带的样式（宿主原样内联即可，不依赖目标页面的全局样式） */
export const RENDER_CSS = `
.pdt-page { position: relative; box-sizing: border-box; background: #fff; overflow: hidden; }
.pdt-page, .pdt-page * { box-sizing: border-box; }
.pdt-band { position: absolute; overflow: visible; }
.pdt-body { position: relative; }
.pdt-el { position: absolute; overflow: hidden; }
.pdt-el.pdt-text { display: block; }
.pdt-wm { position: absolute; inset: 0; pointer-events: none; }
.pdt-table { border-collapse: collapse; table-layout: fixed; width: 100%; height: 100%; }
.pdt-table td { padding: 0 1mm; vertical-align: middle; overflow: hidden; word-break: break-word; }
`

const V_ALIGN_FLEX = {top: 'flex-start', middle: 'center', bottom: 'flex-end'};
const H_ALIGN_FLEX = {left: 'flex-start', center: 'center', right: 'flex-end'};

function positionStyle(options) {
    const parts = [
        'position:absolute',
        `left:${mm(options.left || 0)}`,
        `top:${mm(options.top || 0)}`,
        `width:${mm(options.width || 0)}`,
    ];
    const height = Number(options.height);
    if (Number.isFinite(height) && height > 0) parts.push(`height:${mm(height)}`);
    return parts.join(';') + ';';
}

/** 文本样式：字号/行距/字距单位都是 pt（与 core 的 textStyle 一致） */
function textStyle(options, defaultLineHeight) {
    const parts = [];
    const fontSize = Number(options.fontSize);
    if (fontSize) parts.push(`font-size:${pt(fontSize)}`);
    if (options.fontFamily) parts.push(`font-family:${esc(options.fontFamily)}, serif`);
    if (options.fontWeight) parts.push(`font-weight:${options.fontWeight}`);
    if (options.color) parts.push(`color:${options.color}`);
    if (options.backgroundColor) parts.push(`background-color:${options.backgroundColor}`);
    const lineHeight = Number(options.lineHeight);
    if (lineHeight) parts.push(`line-height:${pt(lineHeight)}`);
    else if (defaultLineHeight) parts.push(`line-height:${defaultLineHeight}`);
    const letterSpacing = Number(options.letterSpacing);
    if (letterSpacing) parts.push(`letter-spacing:${pt(letterSpacing)}`);
    if (options.verticalAlign) {
        parts.push(`display:flex;align-items:${V_ALIGN_FLEX[options.verticalAlign] || 'flex-start'}`);
        parts.push(`justify-content:${H_ALIGN_FLEX[options.textAlign || 'left'] || 'flex-start'}`);
    }
    if (options.textAlign) parts.push(`text-align:${options.textAlign}`);
    if (options.textAlign === 'justify') parts.push('text-align-last:justify');
    if (options.wordWrap === false) parts.push('white-space:nowrap');
    else if (options.keepLineBreaks) parts.push('white-space:pre-wrap');
    return parts.length ? parts.join(';') + ';' : '';
}

function cellBorder(borders) {
    if (!borders) return '';
    const parts = [];
    for (const side of ['top', 'right', 'bottom', 'left']) {
        const rule = borders[side];
        if (rule === undefined) continue;
        if (rule === null || rule.width === 0) parts.push(`border-${side}:none`);
        else
            parts.push(
                `border-${side}:${Number(rule.width) || 0.75}pt ${rule.style || 'solid'} ${rule.color || '#333'}`,
            );
    }
    return parts.length ? parts.join(';') + ';' : '';
}

function renderTable(options, scope) {
    const rows = Array.isArray(options.tableRows) ? options.tableRows : [];
    if (!rows.length) return null;
    const widths = Array.isArray(options.tableColWidths) ? options.tableColWidths : [];
    const colgroup = widths.length
        ? `<colgroup>${widths.map((w) => `<col style="width:${mm(Number(w) || 0)}" />`).join('')}</colgroup>`
        : '';
    const body = rows
        .map((row) => {
            const height = Number(row.height);
            const trStyle = Number.isFinite(height) && height > 0 ? ` style="height:${mm(height)}"` : '';
            const cells = (row.cells || [])
                .map((cell) => {
                    const span =
                        (Number(cell.colspan) > 1 ? ` colspan="${Number(cell.colspan)}"` : '') +
                        (Number(cell.rowspan) > 1 ? ` rowspan="${Number(cell.rowspan)}"` : '');
                    const style =
                        cellBorder(cell.borders) +
                        (cell.align ? `text-align:${cell.align};` : '') +
                        (cell.valign ? `vertical-align:${cell.valign};` : '');
                    const fontSize = Number(options.tableDefaultFontSize);
                    const size = fontSize ? `font-size:${pt(fontSize)};` : '';
                    const color = options.tableDefaultColor ? `color:${options.tableDefaultColor};` : '';
                    const text =
                        cell.cellType === 'image'
                            ? `<img src="${esc(cell.formatter || '')}" style="max-width:100%;max-height:100%" />`
                            : esc(interpolate(cell.formatter == null ? '' : cell.formatter, scope));
                    return `<td${span} style="${style}${size}${color}">${text}</td>`;
                })
                .join('');
            return `<tr${trStyle}>${cells}</tr>`;
        })
        .join('');
    return `<table class="pdt-table">${colgroup}<tbody>${body}</tbody></table>`;
}

/**
 * 单个元素 → HTML；不认识的类型返回 null 并写告警。
 * `scope` 是插值作用域（业务数据 + pageIndex/totalPages）。
 */
export function renderElement(element, scope, warnings) {
    const type = String(element?.type || element?.printElementType?.type || 'text');
    const options = element?.options || {};
    const base = positionStyle(options);
    switch (type) {
        case 'text':
        case 'longText':
        case 'pageNumber': {
            const raw = options.formatter != null ? options.formatter : options.text != null ? options.text : '';
            const content = esc(interpolate(raw, scope)).replace(/\n/g, '<br />');
            const style =
                base +
                textStyle(options, type === 'longText' ? '1.5' : undefined) +
                (elementBorder(options) ? elementBorder(options) : '');
            return `<div class="pdt-el pdt-text" style="${style}">${content}</div>`;
        }
        case 'image': {
            const src = options.src || '';
            if (!src) return null;
            return `<div class="pdt-el" style="${base}"><img src="${esc(src)}" style="width:100%;height:100%;object-fit:${
                options.fit || 'contain'
            }" /></div>`;
        }
        case 'hline':
            return `<div class="pdt-el" style="${base}border-top:${Number(options.borderWidth) || 0.75}pt ${
                options.borderStyle || 'solid'
            } ${options.borderColor || '#333'};height:0"></div>`;
        case 'vline':
            return `<div class="pdt-el" style="${base}border-left:${Number(options.borderWidth) || 0.75}pt ${
                options.borderStyle || 'solid'
            } ${options.borderColor || '#333'};width:0"></div>`;
        case 'rect':
            return `<div class="pdt-el" style="${base}border:${Number(options.borderWidth) || 1}pt ${
                options.borderStyle || 'solid'
            } ${options.borderColor || '#333'};${options.backgroundColor ? `background:${options.backgroundColor};` : ''}"></div>`;
        case 'oval':
            return `<div class="pdt-el" style="${base}border:${Number(options.borderWidth) || 1}pt ${
                options.borderStyle || 'solid'
            } ${options.borderColor || '#333'};border-radius:50%;"></div>`;
        case 'html':
            return `<div class="pdt-el" style="${base}">${options.testData || options.title || ''}</div>`;
        case 'table': {
            const html = renderTable(options, scope);
            if (!html) {
                warnings.push(`表格元素 ${element.id || ''} 没有行定义，已跳过`);
                return null;
            }
            return `<div class="pdt-el" style="${base}">${html}</div>`;
        }
        case 'barcode':
        case 'qrcode':
        case 'flowList':
            warnings.push(`元素类型 ${type} 需要完整渲染管线，模板渲染层按文本降级`);
            return `<div class="pdt-el pdt-text" style="${base}${textStyle(options)}">${esc(
                interpolate(options.formatter || '', scope),
            )}</div>`;
        default:
            warnings.push(`未知元素类型 ${type}，已跳过`);
            return null;
    }
}

/** 元素级整圈边框（text 类可用；图形元素自己有 border 规则） */
function elementBorder(options) {
    const width = Number(options.borderWidth);
    if (!width) return '';
    return `border:${width}pt ${options.borderStyle || 'solid'} ${options.borderColor || '#333'};`;
}

function renderElements(elements, scope, warnings) {
    return (elements || [])
        .map((element) => renderElement(element, scope, warnings))
        .filter(Boolean)
        .join('');
}

/** 水印层：瓦片用内联 SVG，避免宿主额外请求图片 */
function renderWatermark(watermark, geo) {
    const wm = watermark || {};
    const content = String(wm.content || '').trim();
    if (!content) return '';
    const color = wm.color || '#cccccc';
    const opacity = Number.isFinite(Number(wm.opacity)) ? Number(wm.opacity) : 0.15;
    const rotate = Number.isFinite(Number(wm.rotate)) ? Number(wm.rotate) : -30;
    const tileW = Number(wm.tileWidth) || 260;
    const tileH = Number(wm.tileHeight) || 180;
    const fontSize = Number(wm.fontSize) || 16;
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${tileW}" height="${tileH}"><text x="50%" y="50%" fill="${color}" font-size="${fontSize}" font-family="sans-serif" text-anchor="middle" dominant-baseline="middle" transform="rotate(${rotate} ${tileW / 2} ${tileH / 2})">${esc(
        content,
    )}</text></svg>`;
    const url = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
    return `<div class="pdt-wm" style="opacity:${opacity};background-image:url(&quot;${url}&quot;);background-repeat:repeat;width:${mm(
        geo.paper.width,
    )};height:${mm(geo.paper.height)}"></div>`;
}

/**
 * 只渲染一个带状区（页眉 / 页脚 / 封面正文区）。
 * 本地排印引擎把页眉页脚贴到每一页时用的就是它——只要这一段 HTML，
 * 不要整页盒子。
 *
 * @returns {{html: string, warnings: string[]}}
 */
export function renderBand(template, band, options = {}) {
    const tpl = template || {};
    const geo = geometry(tpl);
    const warnings = [];
    const scope = {...(options.data || {}), pageIndex: options.pageIndex ?? 1, totalPages: options.totalPages ?? 1};
    const area = band === 'header' ? tpl.header : band === 'footer' ? tpl.footer : {elements: tpl.elements || []};
    const height = band === 'header' ? geo.headerH : band === 'footer' ? geo.footerH : geo.contentHeight;
    const html = `<div class="pdt-band" style="position:relative;width:${mm(geo.contentWidth)};height:${mm(
        height,
    )}">${renderElements(area?.elements, scope, warnings)}</div>`;
    return {html, warnings};
}

/**
 * 整页渲染：纸面 + 页眉 + 正文区 + 页脚 + 水印。
 * `bodyHtml` 可由调用方传入（例如本地引擎排好的一页题目），
 * 不传则用模板自身的 elements。
 *
 * @returns {{html: string, css: string, warnings: string[], geometry: object}}
 */
export function renderTemplate(template, options = {}) {
    const tpl = template || {};
    const geo = geometry(tpl);
    const warnings = [];
    const pageIndex = options.pageIndex ?? 1;
    const totalPages = options.totalPages ?? 1;
    const scope = {...(options.data || {}), pageIndex, totalPages};

    const headerHtml = renderElements(tpl.header?.elements, scope, warnings);
    const footerHtml = renderElements(tpl.footer?.elements, scope, warnings);
    const bodyHtml =
        options.bodyHtml === undefined ? renderElements(tpl.elements, scope, warnings) : String(options.bodyHtml);

    const pageStyle = [
        `width:${mm(geo.paper.width)}`,
        `height:${mm(geo.paper.height)}`,
        `padding:${mm(geo.margins.top)} ${mm(geo.margins.right)} ${mm(geo.margins.bottom)} ${mm(geo.margins.left)}`,
    ].join(';');

    const headerBlock = geo.headerH
        ? `<div class="pdt-band" data-band="header" style="position:relative;width:${mm(geo.contentWidth)};height:${mm(geo.headerH)}">${headerHtml}</div>`
        : '';
    const footerBlock = geo.footerH
        ? `<div class="pdt-band" data-band="footer" style="position:absolute;top:${mm(geo.footerTop)};left:${mm(
              geo.margins.left,
          )};width:${mm(geo.contentWidth)};height:${mm(geo.footerH)}">${footerHtml}</div>`
        : '';
    const bodyBlock = `<div class="pdt-body" style="width:${mm(geo.contentWidth)}">${bodyHtml}</div>`;

    const html = `<div class="pdt-page" style="${pageStyle}">${renderWatermark(tpl.watermark, geo)}${headerBlock}${bodyBlock}${footerBlock}</div>`;
    return {html, css: RENDER_CSS, warnings, geometry: geo};
}

/** 模板里用到的元素类型统计（宿主接入自检用） */
export function templateSummary(template) {
    const tpl = template || {};
    const counts = {};
    const visit = (list) => {
        for (const element of list || []) {
            const type = String(element?.type || 'text');
            counts[type] = (counts[type] || 0) + 1;
        }
    };
    visit(tpl.header?.elements);
    visit(tpl.footer?.elements);
    visit(tpl.elements);
    const unsupported = Object.keys(counts).filter((type) => !SUPPORTED_TYPES.includes(type));
    return {counts, unsupported};
}
