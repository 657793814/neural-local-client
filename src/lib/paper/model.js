/**
 * 试卷的版面模型：纸型、默认参数、状态文案。
 * 与 Rust 侧 `paper::default_layout()` 保持一致——新建时后端写默认值，
 * 这里只负责读、补缺与前端的物理尺寸换算。
 */

export const PAPER_SIZES = {
    A4: {width: 210, height: 297, label: 'A4（210×297）'},
    A3: {width: 297, height: 420, label: 'A3（297×420）'},
    B5: {width: 176, height: 250, label: 'B5（176×250）'},
    '8K': {width: 270, height: 390, label: '8K（270×390）'},
    '16K': {width: 195, height: 270, label: '16K（195×270）'},
};

export const DEFAULT_LAYOUT = {
    paperSize: 'A4',
    orientation: 'portrait',
    margins: {top: 20, right: 20, bottom: 20, left: 20},
    columns: 1,
    fontFamily: '',
    fontSize: 10.5,
    lineHeight: 1.6,
    questionGap: 6,
    optionColumns: 4,
    answerSpace: 'auto',
    bindingLine: false,
    watermark: null,
    answerKeyPage: true,
    answerSheet: {enabled: true, templateId: 'builtin-generic', paperSize: 'A4', columns: 2},
};

export const EXAM_TYPES = [
    {value: 'EXAM', label: '考试卷'},
    {value: 'PRACTICE', label: '练习卷'},
    {value: 'QUIZ', label: '小测'},
];

export const PAPER_STATUS = {
    DRAFT: {label: '草稿', tone: 'muted'},
    FINAL: {label: '已定稿', tone: 'ok'},
    ARCHIVED: {label: '已归档', tone: 'line'},
};

export function examTypeLabel(value) {
    return EXAM_TYPES.find((item) => item.value === value)?.label || '考试卷';
}

/** 版面参数补缺：老卷子没有新字段时用默认值，不让渲染层到处写 `||`。 */
export function mergeLayout(layout) {
    const source = layout && typeof layout === 'object' ? layout : {};
    return {
        ...DEFAULT_LAYOUT,
        ...source,
        margins: {...DEFAULT_LAYOUT.margins, ...(source.margins || {})},
        answerSheet: {...DEFAULT_LAYOUT.answerSheet, ...(source.answerSheet || {})},
    };
}

/** 纸张物理尺寸（mm），横向时宽高互换。 */
export function paperSize(layout) {
    const merged = mergeLayout(layout);
    const size = PAPER_SIZES[merged.paperSize] || PAPER_SIZES.A4;
    return merged.orientation === 'landscape'
        ? {width: size.height, height: size.width}
        : {width: size.width, height: size.height};
}

/** 版心尺寸（mm）：扣掉页边距；装订线只加宽左侧。 */
export function contentBox(layout) {
    const merged = mergeLayout(layout);
    const size = paperSize(merged);
    const binding = merged.bindingLine ? 10 : 0;
    return {
        width: size.width - merged.margins.left - merged.margins.right - binding,
        height: size.height - merged.margins.top - merged.margins.bottom,
        pageWidth: size.width,
        pageHeight: size.height,
        binding,
    };
}

const CN_NUMBERS = '一二三四五六七八九十';

/** 大题序号：一、二、…… 十之后的用「十一」。 */
export function sectionOrdinal(index) {
    if (index < 10) return CN_NUMBERS[index];
    if (index === 10) return '十';
    if (index < 20) return `十${CN_NUMBERS[index - 10]}`;
    return `${index + 1}`;
}

/** 从「一、单项选择题」里取出纯标题，避免用户自己写了序号又重复一遍。 */
export function stripSectionPrefix(title) {
    return String(title || '').replace(/^\s*[一二三四五六七八九十]+[、.．]\s*/, '').trim();
}

export function fullSectionTitle(section, index) {
    const name = stripSectionPrefix(section?.title);
    return `${sectionOrdinal(index)}、${name || '试题'}`;
}
