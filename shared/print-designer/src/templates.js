// 空白模板工厂：新建模板时给一张「已经像样」的画布，而不是四面白墙。
// 结构必须与 @worm-vue3-print/core 的 TemplateData 一致（unit 固定 mm）。

let seq = 0
const nextId = (prefix) => `${prefix}-${Date.now().toString(36)}-${(seq += 1).toString(36)}`

function text(id, options) {
    return {id, type: 'text', options, printElementType: {type: 'text', title: '文本'}}
}

function line(id, options) {
    return {id, type: 'hline', options, printElementType: {type: 'hline', title: '横线'}}
}

const A4 = {width: 210, height: 297}

/** 模板骨架（页眉页脚三区齐全，元素为空） */
export function emptyTemplate(paperSize = 'A4', orientation = 'portrait') {
    return {
        unit: 'mm',
        paperSize,
        orientation,
        margins: {top: 18, right: 16, bottom: 16, left: 16},
        header: {height: 24, elements: []},
        footer: {height: 8, elements: []},
        elements: [],
        watermark: {},
    }
}

/**
 * 按用途给一张起手模板。
 * - COVER：封面整页（学校 / 卷名 / 考试信息 / 注意事项 / 考生信息）
 * - HEADER：每页重复的卷头（标题 + 信息行 + 分隔线）+ 页脚页码
 * - ANSWER_KEY：答案页卷头（卷末新起一页时用）
 * - ANSWER_SHEET：答题卡卷头（涂卡区由渲染器代码生成，模板只管头尾）
 * - GENERIC：空白三区
 */
export function createTemplate(kind = 'GENERIC') {
    const type = String(kind || 'GENERIC').toUpperCase()
    const contentWidth = A4.width - 32
    if (type === 'COVER') {
        const tpl = emptyTemplate('A4', 'portrait')
        tpl.margins = {top: 24, right: 20, bottom: 20, left: 20}
        tpl.header = {height: 0, elements: []}
        tpl.footer = {height: 0, elements: []}
        const width = A4.width - 40
        const center = (top, height, formatter, fontSize, extra) =>
            text(nextId('el'), {
                left: 0,
                top,
                width,
                height,
                formatter,
                fontSize,
                textAlign: 'center',
                verticalAlign: 'middle',
                lineHeight: 0,
                ...extra,
            })
        tpl.elements = [
            center(22, 10, '{paper.school}', 14, {letterSpacing: 2}),
            center(58, 22, '{paper.title}', 30, {fontWeight: 'bold', letterSpacing: 3}),
            center(86, 12, '{paper.subject} · {paper.grade} · 共 {paper.questionCount} 题 · 满分 {paper.totalScore} 分', 12),
            center(104, 10, '考试时间：{paper.duration} 分钟', 12),
            line(nextId('el'), {left: 0, top: 122, width, height: 0.5, borderWidth: 0.5, borderColor: '#333333'}),
            text(nextId('el'), {
                left: 0,
                top: 132,
                width,
                height: 10,
                formatter: '考生须知',
                fontSize: 12,
                fontWeight: 'bold',
                textAlign: 'left',
                verticalAlign: 'middle',
            }),
            text(nextId('el'), {
                left: 0,
                top: 144,
                width,
                height: 40,
                formatter: '{paper.notice}',
                fontSize: 10.5,
                textAlign: 'left',
                verticalAlign: 'top',
                lineHeight: 18,
                wordWrap: true,
                keepLineBreaks: true,
            }),
            text(nextId('el'), {
                left: 20,
                top: 208,
                width,
                height: 12,
                formatter: '姓名：______________    班级：______________    学号：______________',
                fontSize: 11.5,
                textAlign: 'left',
                verticalAlign: 'middle',
            }),
            text(nextId('el'), {
                left: 0,
                top: 224,
                width,
                height: 10,
                formatter: '{paper.date}',
                fontSize: 11,
                textAlign: 'center',
                verticalAlign: 'middle',
            }),
        ]
        return tpl
    }

    if (type === 'HEADER' || type === 'ANSWER_KEY' || type === 'ANSWER_SHEET') {
        const tpl = emptyTemplate('A4', 'portrait')
        tpl.margins = {top: 16, right: 16, bottom: 16, left: 16}
        tpl.header = {height: 26, elements: []}
        tpl.footer = {height: 8, elements: []}
        const width = contentWidth
        const title =
            type === 'ANSWER_KEY' ? '{paper.title} · 参考答案与解析' : type === 'ANSWER_SHEET' ? '{paper.title} · 答题卡' : '{paper.title}'
        tpl.header.elements = [
            text(nextId('el'), {
                left: 0,
                top: 0,
                width,
                height: 11,
                formatter: title,
                fontSize: 16,
                fontWeight: 'bold',
                textAlign: 'center',
                verticalAlign: 'middle',
            }),
            text(nextId('el'), {
                left: 0,
                top: 11,
                width: width * 0.58,
                height: 7,
                formatter: '{paper.subject} · {paper.grade}',
                fontSize: 10.5,
                textAlign: 'left',
                verticalAlign: 'middle',
            }),
            text(nextId('el'), {
                left: width * 0.58,
                top: 11,
                width: width * 0.42,
                height: 7,
                formatter: '满分 {paper.totalScore} 分 · {paper.duration} 分钟',
                fontSize: 10.5,
                textAlign: 'right',
                verticalAlign: 'middle',
            }),
            text(nextId('el'), {
                left: 0,
                top: 18,
                width,
                height: 6,
                formatter: '姓名：____________  班级：____________  学号：____________  得分：________',
                fontSize: 10,
                textAlign: 'left',
                verticalAlign: 'middle',
            }),
            line(nextId('el'), {
                left: 0,
                top: 25,
                width,
                height: 0.6,
                borderWidth: 0.6,
                borderColor: '#333333',
            }),
        ]
        tpl.footer.elements = [
            text(nextId('el'), {
                left: 0,
                top: 0,
                width,
                height: 7,
                formatter: '第 {pageIndex} 页 / 共 {totalPages} 页',
                fontSize: 9.5,
                textAlign: 'center',
                verticalAlign: 'middle',
            }),
        ]
        return tpl
    }

    return emptyTemplate()
}
