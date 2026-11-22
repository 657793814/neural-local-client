// PrintDesk：把 @worm-vue3-print/common 的 <print-designer> 包成「三行接入」的门面。
//
//   PrintDesk.mount(el, {template, fields, data, onSave})
//   PrintDesk.openPanel({template, fields, onSave})        // 自带全屏弹窗
//   PrintDesk.renderTemplate(tpl, {data, pageIndex})       // 模板 → 打印 HTML（不需要设计器）
//
// 设计原则：
// 1) 零框架依赖——React / Vue / jQuery / 原生页面都用同一套 API；
// 2) 设计器体积（约 590 kB）懒加载，不进宿主首屏；
// 3) 出纸链路不依赖设计器，服务端或离屏页面也能把模板渲成 HTML。
import {PAPER_FIELDS, paperSampleData} from './fields.js';
import {RENDER_CSS, SUPPORTED_TYPES, renderBand, renderTemplate, templateSummary} from './render.js';
import {createTemplate, emptyTemplate} from './templates.js';
import {PAPER_SIZES, geometry, paperMm} from './util.js';

export const VERSION = '1.0.0'
export const CONTRACT = 'print-designer/v1'

const state = {
    vendorUrl: '',
    vendorPromise: null,
    theme: {},
}

/** 推断 vendor 目录：优先显式配置，其次当前脚本地址，最后 `/print-designer/` */
function resolveVendorUrl() {
    if (state.vendorUrl) return state.vendorUrl
    if (typeof document !== 'undefined') {
        const current = document.currentScript && document.currentScript.src
        const scripts = current ? [current] : [...document.scripts].map((s) => s.src).filter(Boolean)
        const hit = scripts.reverse().find((src) => /print-designer(\.esm)?\.js(\?|$)/.test(src))
        if (hit) return new URL('vendor/common.js', hit).href
    }
    return '/print-designer/vendor/common.js'
}

function injectScript(src) {
    return new Promise((resolve, reject) => {
        const existing = document.querySelector(`script[data-printdesk-vendor="${src}"]`)
        if (existing) {
            if (existing.dataset.loaded === '1') resolve()
            else {
                existing.addEventListener('load', () => resolve(), {once: true})
                existing.addEventListener('error', () => reject(new Error(`加载失败：${src}`)), {once: true})
            }
            return
        }
        const script = document.createElement('script')
        script.src = src
        script.async = true
        script.dataset.printdeskVendor = src
        script.addEventListener('load', () => {
            script.dataset.loaded = '1'
            resolve()
        })
        script.addEventListener('error', () => reject(new Error(`加载失败：${src}`)), {once: true})
        document.head.appendChild(script)
    })
}

/** 注入宿主侧必要样式：自定义元素默认 display:inline，不给块级尺寸就会塌陷 */
function ensureHostStyle() {
    if (typeof document === 'undefined') return
    if (document.getElementById('printdesk-host-style')) return
    const style = document.createElement('style')
    style.id = 'printdesk-host-style'
    style.textContent = `
print-designer { display: block; width: 100%; height: 100%; min-height: 420px; }
.pd-modal { position: fixed; inset: 0; z-index: 9000; display: flex; align-items: stretch; justify-content: center;
  background: rgba(20, 24, 28, .55); backdrop-filter: blur(2px); }
.pd-modal-panel { display: flex; flex-direction: column; width: min(1520px, 96vw); margin: 2vh 0; background: #fff;
  border-radius: 14px; overflow: hidden; box-shadow: 0 24px 64px rgba(0, 0, 0, .32); }
.pd-modal-head { display: flex; align-items: center; gap: 10px; padding: 12px 16px; border-bottom: 1px solid #e5e7eb; }
.pd-modal-title { font-size: 14px; font-weight: 600; color: #111827; }
.pd-modal-sub { font-size: 12px; color: #6b7280; }
.pd-modal-spacer { flex: 1; }
.pd-btn { border: 1px solid #d1d5db; background: #fff; color: #111827; border-radius: 8px; padding: 6px 12px;
  font-size: 12.5px; cursor: pointer; }
.pd-btn:hover { border-color: #9ca3af; }
.pd-btn.primary { background: #0f766e; border-color: #0f766e; color: #fff; }
.pd-btn.primary:hover { background: #115e59; }
.pd-modal-body { flex: 1; min-height: 0; }
`
    document.head.appendChild(style)
}

/**
 * 预加载设计器运行时（可提前调用，省掉用户点开时的空白等待）。
 * @returns {Promise<any>} 全局 WormPrintCommon
 */
export function ready() {
    if (typeof window === 'undefined') return Promise.reject(new Error('设计器只能在浏览器环境使用'))
    if (window.WormPrintCommon && window.WormPrintCommon.registerAll) {
        ensureHostStyle()
        return Promise.resolve(window.WormPrintCommon)
    }
    if (state.vendorPromise) return state.vendorPromise
    ensureHostStyle()
    state.vendorPromise = injectScript(resolveVendorUrl())
        .then(() => {
            const api = window.WormPrintCommon
            if (!api || typeof api.registerAll !== 'function') {
                throw new Error('设计器运行时没有挂出 WormPrintCommon')
            }
            api.registerAll()
            return api
        })
        .catch((error) => {
            state.vendorPromise = null
            throw error
        })
    return state.vendorPromise
}

/** 模板里声明的字体（多页模板按 family 去重） */
export function fontsOf(template) {
    const tpl = template || {}
    const pages = Array.isArray(tpl.pages) ? tpl.pages : null
    const list = pages ? pages.flatMap((page) => page?.fonts || []) : tpl.fonts || []
    const seen = new Set()
    return list.filter((font) => font && font.family && !seen.has(font.family) && seen.add(font.family))
}

/**
 * 挂载设计器。
 * @param {HTMLElement} container 容器（会被清空并塞入 <print-designer>）
 * @param {object} options
 * @param {object} [options.template] 模板 JSON
 * @param {Array}  [options.fields]   字段契约，默认用试卷字段
 * @param {object} [options.data]     预览数据
 * @param {boolean}[options.isEdit]   默认 true
 * @param {object} [options.theme]    --pd-* 主题令牌
 * @param {(json:string)=>void} [options.onSave]
 * @param {(json:string)=>void} [options.onPreview]
 * @param {()=>void} [options.onHelp]
 * @param {(id:string)=>void} [options.onDblclickElement]
 * @param {(file:File)=>Promise<string>} [options.uploadImage]
 * @returns {Promise<PrintDesignerHandle>}
 */
export async function mount(container, options = {}) {
    if (!container) throw new Error('mount 需要一个容器元素')
    await ready()
    const element = document.createElement('print-designer')
    element.setAttribute('is-edit', String(options.isEdit !== false))
    element.setAttribute('show-help', String(options.showHelp !== false))
    applyTheme(element, options.theme)
    container.replaceChildren(element)

    const listeners = []
    const on = (name, handler) => {
        if (typeof handler !== 'function') return
        const wrapped = (event) => handler(event.detail, event)
        element.addEventListener(name, wrapped)
        listeners.push([name, wrapped])
    }
    on('save', options.onSave)
    on('preview', options.onPreview)
    on('help', options.onHelp)
    on('dblclick-element', options.onDblclickElement)
    on('dblclick-cell', options.onDblclickCell)

    if (options.uploadImage) element.uploadImage = options.uploadImage
    if (options.uploadDesignBackground) element.uploadDesignBackground = options.uploadDesignBackground

    const template = options.template || createTemplate(options.kind || 'GENERIC')
    const fields = options.fields || PAPER_FIELDS
    const data = options.data || paperSampleData()

    setTemplate(template)
    setFields(fields)
    if (typeof element.setDesignData === 'function') element.setDesignData(data)

    // 设计器首帧要等自定义元素 upgrade 完，否则 property 会被吃掉
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))

    return {
        element,
        getTemplateJson: () => element.getTemplateJson(),
        getTemplate: () => safeParse(element.getTemplateJson()),
        validate: () => (typeof element.validateTemplate === 'function' ? element.validateTemplate() : []),
        save: () => element.save && element.save(),
        getGeometryWarnings: () =>
            typeof element.getGeometryWarnings === 'function' ? element.getGeometryWarnings() : [],
        setTemplate,
        setFields,
        setData: (next) => {
            if (typeof element.setDesignData === 'function') element.setDesignData(next)
        },
        applyTheme: (theme) => applyTheme(element, theme),
        destroy() {
            for (const [name, handler] of listeners) element.removeEventListener(name, handler)
            listeners.length = 0
            container.replaceChildren()
        },
    }

    function setTemplate(next) {
        element.template = next
        element.fonts = fontsOf(next)
    }
    function setFields(next) {
        element.fields = next
    }
}

function applyTheme(element, theme) {
    if (!theme) return
    for (const [key, value] of Object.entries(theme)) {
        element.style.setProperty(key.startsWith('--') ? key : `--pd-${key}`, String(value))
    }
}

function safeParse(text) {
    if (typeof text !== 'string') return text || null
    try {
        return JSON.parse(text)
    } catch {
        return null
    }
}

/**
 * 自带外壳的全屏设计弹窗：不想自己搭顶栏/保存逻辑的服务直接用这个。
 * @returns {Promise<{closed: Promise<void>, close: () => void, handle: object}>}
 */
export async function openPanel(options = {}) {
    ensureHostStyle()
    const mask = document.createElement('div')
    mask.className = 'pd-modal'
    const panel = document.createElement('div')
    panel.className = 'pd-modal-panel'
    panel.innerHTML = `
<div class="pd-modal-head">
  <span class="pd-modal-title"></span>
  <span class="pd-modal-sub"></span>
  <span class="pd-modal-spacer"></span>
  <button type="button" class="pd-btn" data-act="preview">预览</button>
  <button type="button" class="pd-btn primary" data-act="save">保存模板</button>
  <button type="button" class="pd-btn" data-act="close">关闭</button>
</div>
<div class="pd-modal-body"></div>`
    panel.querySelector('.pd-modal-title').textContent = options.title || '设计打印模板'
    panel.querySelector('.pd-modal-sub').textContent = options.subtitle || ''
    mask.appendChild(panel)
    document.body.appendChild(mask)

    let resolveClosed
    let handle = null
    const closed = new Promise((resolve) => (resolveClosed = resolve))
    const close = () => {
        handle?.destroy?.()
        mask.remove()
        resolveClosed()
    }
    mask.querySelector('[data-act="close"]').addEventListener('click', close)
    mask.querySelector('[data-act="preview"]').addEventListener('click', () => {
        const json = handle?.getTemplateJson?.()
        const parsed = safeParse(json)
        if (!parsed) return
        options.onPreview ? options.onPreview(json) : previewTemplate(parsed, {title: options.title})
    })
    mask.querySelector('[data-act="save"]').addEventListener('click', () => {
        const issues = handle.validate()
        if (issues.length) {
            alert(issues.map((item) => item.message || item).join('\n'))
            return
        }
        const json = handle.getTemplateJson()
        if (!options.onSave) return
        Promise.resolve(options.onSave(json)).then((ok) => {
            if (ok !== false) close()
        })
    })

    handle = await mount(panel.querySelector('.pd-modal-body'), {
        ...options,
        onSave: undefined,
        onPreview: undefined,
    })
    return {closed, close, handle}
}

/**
 * 浏览器预览：把模板与数据渲成一叠纸，放进新窗口让用户直接 ⌘P。
 * 不传 template 也可以只传 HTML。
 */
export function previewTemplate(template, options = {}) {
    const rendered = renderTemplate(template, options)
    const win = window.open('', '_blank', 'width=1000,height=800')
    if (!win) {
        alert('浏览器拦截了新窗口，请允许弹窗后重试')
        return null
    }
    const pages = options.pages || 1
    const body = Array.from({length: pages}, (_, index) =>
        renderTemplate(template, {...options, pageIndex: index + 1, totalPages: pages}).html,
    ).join('')
    win.document.write(`<!DOCTYPE html><html lang="zh-CN"><head><meta charset="utf-8" />
<title>${options.title || '打印预览'}</title>
<style>
@page { size: ${template?.paperSize || 'A4'} ${template?.orientation === 'landscape' ? 'landscape' : 'portrait'}; margin: 0; }
body { margin: 0; background: #eef1f4; }
.pdt-page { margin: 10mm auto; box-shadow: 0 2px 12px rgba(0,0,0,.16); }
@media print { body { background: #fff; } .pdt-page { margin: 0; box-shadow: none; page-break-after: always; } }
${rendered.css}
</style></head><body>${body}</body></html>`)
    win.document.close()
    return {warnings: rendered.warnings, window: win}
}

/** 设计器运行时是否已就绪（不触发加载） */
export function isReady() {
    return typeof window !== 'undefined' && !!(window.WormPrintCommon && window.WormPrintCommon.registerAll)
}

/** 覆盖运行时地址（离线内网部署 / 自定义 CDN 时用） */
export function configure(options = {}) {
    if (options.vendorUrl) {
        state.vendorUrl = options.vendorUrl
        state.vendorPromise = null
    }
    if (options.theme) state.theme = {...state.theme, ...options.theme}
}

const PrintDesk = {
    VERSION,
    CONTRACT,
    // 设计器
    ready,
    isReady,
    mount,
    openPanel,
    configure,
    fontsOf,
    // 出纸
    renderTemplate,
    renderBand,
    previewTemplate,
    templateSummary,
    RENDER_CSS,
    SUPPORTED_TYPES,
    // 模板与字段
    createTemplate,
    emptyTemplate,
    PAPER_FIELDS,
    PAPER_SIZES,
    paperSampleData,
    paperMm,
    geometry,
}

export {
    createTemplate,
    emptyTemplate,
    geometry,
    PAPER_FIELDS,
    paperMm,
    paperSampleData,
    PAPER_SIZES,
    renderBand,
    templateSummary,
}

export default PrintDesk
