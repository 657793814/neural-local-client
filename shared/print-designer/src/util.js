// 通用小工具：模板渲染层只用这些，不引任何框架运行时。
export const MM_PER_INCH = 25.4
export const PT_PER_INCH = 72

/** HTML 转义（文本内容） */
export function esc(value) {
    return String(value == null ? '' : value)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;');
}

/** mm 数值 → CSS 长度，去掉浮点噪声 */
export function mm(value) {
    const n = Number(value)
    if (!Number.isFinite(n)) return '0mm'
    return `${Math.round(n * 1000) / 1000}mm`
}

/** pt 数值 → CSS 长度 */
export function pt(value) {
    const n = Number(value)
    if (!Number.isFinite(n)) return '0pt'
    return `${Math.round(n * 100) / 100}pt`
}

/** 点分路径取值：'paper.title' → data.paper.title */
export function getByPath(source, path) {
    if (!source || !path) return undefined
    let cursor = source
    for (const key of String(path).split('.')) {
        if (cursor == null) return undefined
        cursor = cursor[key]
    }
    return cursor
}

/** 标量转显示文本；列表/对象返回空串（由调用方决定怎么处理） */
export function scalarText(value) {
    if (value == null) return ''
    if (typeof value === 'string') return value
    if (typeof value === 'number' || typeof value === 'boolean') return String(value)
    return ''
}

/**
 * 模板表达式取值：`{paper.title}` / `{pageIndex}` / `{totalPages}`。
 * 只做占位符替换——设计器里写的是「显示什么」，不需要完整表达式引擎；
 * 未知字段保留原样（`{foo}`）便于老师发现自己写错了字段名。
 */
export function interpolate(text, scope) {
    return String(text == null ? '' : text).replace(/\{([^{}]+)\}/g, (all, key) => {
        const path = String(key).trim()
        const value = getByPath(scope, path)
        if (value == null) return all
        const plain = scalarText(value)
        return plain === '' ? all : plain
    })
}

/** 纸张尺寸（mm），与 @worm-vue3-print/core 的 getPaperDimensions 同口径 */
const PAPER_MM = {
    A0: [841, 1189],
    A1: [594, 841],
    A2: [420, 594],
    A3: [297, 420],
    A4: [210, 297],
    A5: [148, 210],
    A6: [105, 148],
    B4: [250, 353],
    B5: [176, 250],
    B6: [125, 176],
    '8K': [270, 390],
    '16K': [195, 270],
    Letter: [215.9, 279.4],
    Legal: [215.9, 355.6],
}

export const PAPER_SIZES = Object.keys(PAPER_MM)

export function paperMm(paperSize, orientation) {
    const pair = PAPER_MM[paperSize] || PAPER_MM.A4
    const landscape = String(orientation || '').toLowerCase().startsWith('land')
    return landscape ? {width: pair[1], height: pair[0]} : {width: pair[0], height: pair[1]}
}

/** 模板几何：纸面、内容区、页眉页脚三区，与设计器画布完全一致 */
export function geometry(template) {
    const tpl = template || {}
    const paper = paperMm(tpl.paperSize, tpl.orientation)
    const margins = {
        top: Number(tpl.margins?.top) || 0,
        right: Number(tpl.margins?.right) || 0,
        bottom: Number(tpl.margins?.bottom) || 0,
        left: Number(tpl.margins?.left) || 0,
    }
    const headerH = Number(tpl.header?.height) || 0
    const footerH = Number(tpl.footer?.height) || 0
    const contentWidth = Math.max(0, paper.width - margins.left - margins.right)
    const contentHeight = Math.max(0, paper.height - margins.top - margins.bottom)
    return {
        paper,
        margins,
        headerH,
        footerH,
        contentWidth,
        contentHeight,
        // 页脚贴纸底：top = 纸高 - 下边距 - 页脚高
        footerTop: paper.height - margins.bottom - footerH,
    }
}
