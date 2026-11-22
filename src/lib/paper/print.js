/**
 * 打印通道：P0 走「自包含 HTML + 系统浏览器打印 / 另存 PDF」。
 * 应用内预览与打印是同一份 HTML，避免两端排版漂移。
 */

import {appInfo, openPath, writeTextFile} from '../api.js';
import {saveTextFile} from '../format.js';

function safeName(name) {
    const cleaned = String(name || '试卷')
        .replace(/[\\/:*?"<>|\n\r]/g, '_')
        .trim()
        .slice(0, 60);
    return cleaned || '试卷';
}

export async function writePrintFile(html, name) {
    const info = await appInfo();
    const path = `${info.dataDir}/prints/${safeName(name)}.html`;
    await writeTextFile(path, html);
    return path;
}

/** 写到数据目录再交给系统默认浏览器：⌘P 打印，也可以「另存为 PDF」。 */
export async function openInBrowser(html, name) {
    const path = await writePrintFile(html, name);
    await openPath(path);
    return path;
}

/** 导出 HTML 到用户选的位置。 */
export async function exportHtml(html, name) {
    const filename = `${safeName(name)}.html`;
    return saveTextFile(filename, html, [{name: '网页', extensions: ['html']}]);
}

/** 应用内直接调打印（WKWebView 不一定支持，失败时用系统浏览器兜底）。 */
export function printFrame(frame) {
    const win = frame?.contentWindow;
    if (!win) return false;
    try {
        win.focus();
        win.print();
        return true;
    } catch {
        return false;
    }
}
