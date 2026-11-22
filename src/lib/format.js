import {save} from '@tauri-apps/plugin-dialog';
import {writeTextFile} from './api.js';

export const TYPES = [
    {value: 'SINGLE', label: '单选题'},
    {value: 'MULTI', label: '多选题'},
    {value: 'FILL', label: '填空题'},
    {value: 'SOLVED', label: '解答题'},
];

export const DIFFICULTIES = [
    {value: 1, label: '1 · 很容易'},
    {value: 2, label: '2 · 容易'},
    {value: 3, label: '3 · 中等'},
    {value: 4, label: '4 · 较难'},
    {value: 5, label: '5 · 很难'},
];

export function typeLabel(kind) {
    return TYPES.find((item) => item.value === kind)?.label || '解答题';
}

/**
 * 任务预览里的一道题能不能勾上存草稿。
 * 只有「连题干都没认出来」或识别直接失败才拦；缺答案、缺解析、缺知识点
 * 属于待补事项——草稿箱本来就是放这类半成品的地方。
 */
export function isBlocked(item) {
    if (!item) return true;
    if (item.error) return true;
    return (item.blocking || []).length > 0;
}

export function statusLabel(question) {
    if (question.deletedAt) return {text: '回收站', tone: 'muted'};
    if (question.needsReview) return {text: '草稿', tone: 'warn'};
    if (question.status === 'PUBLISHED') return {text: '已入库', tone: 'ok'};
    // 复核后被打回、还留着原因：这不是普通草稿，列表里要一眼看得出来。
    if (question.reviewNote) return {text: '已驳回', tone: 'danger'};
    return {text: '草稿', tone: 'muted'};
}

export function syncLabel(state) {
    switch (state) {
        case 'PENDING':
            return {text: '待同步', tone: 'warn'};
        case 'SYNCED':
            return {text: '已同步', tone: 'ok'};
        case 'CONFLICT':
            return {text: '冲突', tone: 'danger'};
        case 'FAILED':
            return {text: '同步失败', tone: 'danger'};
        default:
            return null;
    }
}

export function formatTime(value) {
    if (!value) return '';
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return String(value);
    const now = new Date();
    const sameDay = date.toDateString() === now.toDateString();
    const pad = (num) => String(num).padStart(2, '0');
    if (sameDay) return `今天 ${pad(date.getHours())}:${pad(date.getMinutes())}`;
    const sameYear = date.getFullYear() === now.getFullYear();
    return sameYear
        ? `${date.getMonth() + 1}月${date.getDate()}日`
        : `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

export function splitList(text) {
    return String(text || '')
        .split(/[;；,，\n]/)
        .map((item) => item.trim())
        .filter(Boolean);
}

/** 选项删除或去掉空项后重新编号，答案字母也跟着原选项走。 */
export function reindexChoice(options, answer, dropMissing = true) {
    const keyMap = new Map();
    const nextOptions = options.map((option, index) => {
        const oldKey = String(option.key || String.fromCharCode(65 + index)).toUpperCase();
        const key = String.fromCharCode(65 + index);
        keyMap.set(oldKey, key);
        return {...option, key};
    });
    const nextAnswer = (String(answer || '').toUpperCase().match(/[A-Z]/g) || [])
        .map((key) => keyMap.get(key) || (dropMissing ? '' : key))
        .filter(Boolean)
        .join('');
    return {options: nextOptions, answer: nextAnswer};
}

/** 非空答案才检查；空答案由草稿 / 正式入库各自的规则处理。 */
export function choiceAnswerError(kind, options, answer) {
    const raw = String(answer || '').trim();
    if (!raw || (kind !== 'SINGLE' && kind !== 'MULTI')) return '';
    const letters = raw.toUpperCase().match(/[A-Z]/g) || [];
    const keys = new Set(options.map((option) => String(option.key).toUpperCase()));
    const validSyntax = /^[A-Z\s,，、;；/|.．。()（）\[\]【】]+$/i.test(raw);
    if (!validSyntax || letters.length === 0 || letters.some((key) => !keys.has(key))) {
        return '选择题答案只能填实际选项的字母';
    }
    if (kind === 'SINGLE' && letters.length !== 1) {
        return '单选题答案只能是一个选项字母';
    }
    return '';
}

/**
 * 网页截图录题时，组卷网站底部的「今日 / 次组卷 / 相似题 / 纠错 / 详情 /
 * 收藏 / 加入试题篮」按钮会被一起 OCR 进来，拆题时又被误拆成独立题目。
 * 这类条目没有题干内容，识别出来让用户确认后移到回收站。
 * 用「加入试题篮 + 组卷/相似题」组合判断，避免误伤正常题目里的「收藏」等词。
 */
export function isJunkStem(stem) {
    const text = String(stem || '');
    if (!text) return false;
    const hasBasket = /加入试题篮/.test(text);
    const hasWebChrome = /(次组卷|相似题|纠错|卷引用)/.test(text);
    return hasBasket && hasWebChrome;
}

export function cx(...values) {
    return values.filter(Boolean).join(' ');
}

/** 把一段文本写成文件并弹出保存框。 */
/// 只让用户选一个保存位置，返回路径；取消返回 null。写文件由 Rust 那边做。
export async function pickSavePath(defaultName, filters) {
    const path = await save({defaultPath: defaultName, filters});
    return path || null;
}

export async function saveTextFile(defaultName, content, filters) {
    const path = await save({defaultPath: defaultName, filters});
    if (!path) return null;
    await writeTextFile(path, content);
    return path;
}

export function readFileAsDataUrl(file) {
    return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(reader.result);
        reader.onerror = () => reject(new Error('读不了这个文件'));
        reader.readAsDataURL(file);
    });
}

/** 教材树拍平成选择器要用的列表。 */
export function flattenPoints(tree = []) {
    const out = [];
    for (const subject of tree) {
        for (const textbook of subject.textbooks || []) {
            for (const chapter of textbook.chapters || []) {
                for (const point of chapter.points || []) {
                    out.push({
                        id: point.id,
                        name: point.name,
                        subjectId: subject.id,
                        chapter: chapter.name,
                        path: `${subject.name} / ${textbook.name} / ${chapter.name}`,
                    });
                }
            }
        }
    }
    return out;
}

export function subjectList(tree = []) {
    return tree.map((subject) => ({
        id: subject.id,
        name: subject.name,
        points: (subject.textbooks || []).flatMap((textbook) =>
            (textbook.chapters || []).flatMap((chapter) =>
                (chapter.points || []).map((point) => ({
                    id: point.id,
                    name: point.name,
                    chapter: chapter.name,
                })),
            ),
        ),
    }));
}

/** 字节数转人读大小；存储面板和清理提示共用。 */
export function formatBytes(value) {
    const bytes = Number(value) || 0;
    if (bytes < 1024) return `${bytes} B`;
    const units = ['KB', 'MB', 'GB', 'TB'];
    let size = bytes / 1024;
    let index = 0;
    while (size >= 1024 && index < units.length - 1) {
        size /= 1024;
        index += 1;
    }
    return `${size >= 100 ? size.toFixed(0) : size.toFixed(1)} ${units[index]}`;
}
