// 业务字段契约：字段列表喂给设计器的「字段面板」，示例数据喂给预览。
// 其它服务可以直接用自己的字段清单，不必用这里的试卷字段——设计器只认
// {fieldKey, fieldLabel, fieldType, sortOrder} 这个形状。

/** 试卷类模板的字段清单（与 docs/product/local-paper-management-plan.md §7.2 一致） */
export const PAPER_FIELDS = [
    {fieldKey: 'paper.title', fieldLabel: '试卷名称', fieldType: 'string', sortOrder: 1},
    {fieldKey: 'paper.subject', fieldLabel: '学科', fieldType: 'string', sortOrder: 2},
    {fieldKey: 'paper.grade', fieldLabel: '年级', fieldType: 'string', sortOrder: 3},
    {fieldKey: 'paper.duration', fieldLabel: '考试时长（分钟）', fieldType: 'number', sortOrder: 4},
    {fieldKey: 'paper.totalScore', fieldLabel: '总分', fieldType: 'number', sortOrder: 5},
    {fieldKey: 'paper.questionCount', fieldLabel: '题目数', fieldType: 'number', sortOrder: 6},
    {fieldKey: 'paper.date', fieldLabel: '日期', fieldType: 'string', sortOrder: 7},
    {fieldKey: 'paper.school', fieldLabel: '学校', fieldType: 'string', sortOrder: 8},
    {fieldKey: 'paper.examNo', fieldLabel: '试卷编号', fieldType: 'string', sortOrder: 9},
    {fieldKey: 'paper.notice', fieldLabel: '注意事项', fieldType: 'string', sortOrder: 10},
    {fieldKey: 'paper.sections', fieldLabel: '大题列表', fieldType: 'list', sortOrder: 11},
]

/**
 * 由试卷摘要生成设计器预览数据。
 * 字段名与 `PAPER_FIELDS` 对齐；缺项给占位文案，避免预览里出现空白页让人以为模板坏了。
 */
export function paperSampleData(paper) {
    const source = paper || {}
    const today = new Date()
    const pad = (n) => String(n).padStart(2, '0')
    return {
        paper: {
            title: source.title || '八年级数学期中试卷',
            subject: source.subjectName || source.subject || '数学',
            grade: source.grade || '八年级',
            duration: Number(source.durationMin ?? source.duration ?? 90),
            totalScore: Number(source.totalScore ?? 120),
            questionCount: Number(source.questionCount ?? 24),
            date: source.date || `${today.getFullYear()} 年 ${today.getMonth() + 1} 月 ${today.getDate()} 日`,
            school: source.school || '实验中学',
            examNo: source.examNo || `${today.getFullYear()}${pad(today.getMonth() + 1)}-01`,
            notice:
                source.notice ||
                '1. 答题前请在密封线内填写姓名、班级、学号。\n2. 选择题用 2B 铅笔填涂，非选择题用黑色签字笔作答。\n3. 考试结束后将试卷与答题卡一并交回。',
            sections: Array.isArray(source.sections)
                ? source.sections
                : [
                      {title: '一、选择题', count: 10, score: 30},
                      {title: '二、填空题', count: 6, score: 24},
                      {title: '三、解答题', count: 6, score: 66},
                  ],
        },
        pageIndex: 1,
        totalPages: Number(source.pageCount ?? 4),
        printDate: `${today.getFullYear()}-${pad(today.getMonth() + 1)}-${pad(today.getDate())}`,
    }
}
