import {settingGet, settingSet} from '../api.js';

/** 自定义答题卡模板存在 app_setting 里；内置模板跟着应用走，不进数据库。 */
export const ANSWER_SHEET_STORE_KEY = 'paper.answerSheets';

/**
 * 答题卡模板的通用数据结构：
 * - pageColumns：答题卡正文分几栏（1 / 2）；
 * - fields：卷头要填的字段，渲染成「班级：______」；
 * - notice：作答须知；
 * - rules：按题型分区，每条规则决定这一区画成涂卡格、横线还是答题框。
 *
 * 规则里的 types 用题库题型（SINGLE / MULTI / FILL / SOLVED）；
 * sectionKinds 可选，用大题题型（CALC 等）兜底。这样固定规制与按题形态自适应
 * 用的是同一份数据，只是规则写得粗或细。
 */
export const DEFAULT_ANSWER_SHEET_CONFIG = {
    version: 1,
    mode: 'ADAPTIVE',
    pageColumns: 2,
    fields: ['班级', '姓名', '学号'],
    notice: '选择题用 2B 铅笔填涂；非选择题用 0.5 毫米黑色签字笔在各题答题区内作答。',
    rules: [
        {title: '选择题', types: ['SINGLE', 'MULTI'], style: 'bubble', columns: 2, heightMm: 0},
        {title: '填空题', types: ['FILL'], style: 'lines', columns: 2, heightMm: 22, lines: 3},
        {title: '解答题', types: ['SOLVED'], style: 'box', columns: 1, heightMm: 48},
    ],
};

/** 历届高考答题卡常见的几种规制；老师可以复制成自定义模板再改。 */
export const BUILTIN_ANSWER_SHEETS = [
    {
        id: 'builtin-generic',
        name: '通用 · 按题目形态自适应',
        description: '不预设科目：选择题涂卡、填空题横线、解答题答题框，按卷内实际题型自动分区。',
        builtin: true,
        config: {
            ...DEFAULT_ANSWER_SHEET_CONFIG,
            mode: 'ADAPTIVE',
        },
    },
    {
        id: 'builtin-math',
        name: '高考数学 · 选择 + 填空 + 解答',
        description: '选择题涂卡区、填空题短框、解答题大框；适合新高考数学与大多数理科卷。',
        builtin: true,
        config: {
            version: 1,
            mode: 'FIXED',
            pageColumns: 2,
            fields: ['班级', '姓名', '学号', '座位号'],
            notice:
                '选择题用 2B 铅笔填涂，填空题与解答题用 0.5 毫米黑色签字笔作答。解答题超出答题区的部分无效。',
            rules: [
                {title: '选择题', types: ['SINGLE', 'MULTI'], style: 'bubble', columns: 2, heightMm: 0},
                {title: '填空题', types: ['FILL'], style: 'box', columns: 2, heightMm: 18},
                {title: '解答题', types: ['SOLVED'], style: 'box', columns: 1, heightMm: 58},
            ],
        },
    },
    {
        id: 'builtin-chinese',
        name: '高考语文 · 选择 + 阅读 + 作文',
        description: '选择题涂卡、阅读题大框、作文长框；单栏排版，给长答案留足宽度。',
        builtin: true,
        config: {
            version: 1,
            mode: 'FIXED',
            pageColumns: 1,
            fields: ['班级', '姓名', '学号', '考场号'],
            notice:
                '选择题用 2B 铅笔填涂；非选择题用 0.5 毫米黑色签字笔在对应题号区域内作答，超出答题区无效。',
            rules: [
                {title: '选择题', types: ['SINGLE', 'MULTI'], style: 'bubble', columns: 4, heightMm: 0},
                {title: '非选择题', types: ['FILL'], style: 'lines', columns: 1, heightMm: 34, lines: 5},
                {title: '阅读与作文', types: ['SOLVED'], style: 'box', columns: 1, heightMm: 78},
            ],
        },
    },
    {
        id: 'builtin-english',
        name: '高考英语 · 选择 + 语法填空 + 写作',
        description: '选择题涂卡、语法填空横线、应用文与读后续写答题框；双栏排版。',
        builtin: true,
        config: {
            version: 1,
            mode: 'FIXED',
            pageColumns: 2,
            fields: ['班级', '姓名', '学号'],
            notice:
                '选择题用 2B 铅笔填涂；语法填空与写作部分用 0.5 毫米黑色签字笔作答。',
            rules: [
                {title: '选择题', types: ['SINGLE', 'MULTI'], style: 'bubble', columns: 2, heightMm: 0},
                {title: '语法填空', types: ['FILL'], style: 'lines', columns: 2, heightMm: 26, lines: 4},
                {title: '写作', types: ['SOLVED'], style: 'box', columns: 1, heightMm: 72},
            ],
        },
    },
    {
        id: 'builtin-science',
        name: '高考理综 · 选择 + 非选择',
        description: '选择题集中涂卡，物理 / 化学 / 生物非选择题按题给大框；双栏排版。',
        builtin: true,
        config: {
            version: 1,
            mode: 'FIXED',
            pageColumns: 2,
            fields: ['班级', '姓名', '学号', '座位号'],
            notice:
                '选择题用 2B 铅笔填涂；非选择题用 0.5 毫米黑色签字笔作答，注意题号对应，写错区域无效。',
            rules: [
                {title: '选择题', types: ['SINGLE', 'MULTI'], style: 'bubble', columns: 2, heightMm: 0},
                {title: '非选择题', types: ['FILL'], style: 'box', columns: 2, heightMm: 20},
                {title: '实验与计算题', types: ['SOLVED'], style: 'box', columns: 1, heightMm: 60},
            ],
        },
    },
    {
        id: 'builtin-liberal',
        name: '高考文综 · 选择 + 非选择',
        description: '选择题涂卡、填空短框、材料分析长框；双栏排版，适合政史地。',
        builtin: true,
        config: {
            version: 1,
            mode: 'FIXED',
            pageColumns: 2,
            fields: ['班级', '姓名', '学号', '座位号'],
            notice:
                '选择题用 2B 铅笔填涂；非选择题用 0.5 毫米黑色签字笔在对应题号区域内作答。',
            rules: [
                {title: '选择题', types: ['SINGLE', 'MULTI'], style: 'bubble', columns: 2, heightMm: 0},
                {title: '填空题', types: ['FILL'], style: 'lines', columns: 2, heightMm: 24, lines: 3},
                {title: '材料分析题', types: ['SOLVED'], style: 'box', columns: 1, heightMm: 64},
            ],
        },
    },
];

export const ANSWER_SHEET_STYLES = [
    {value: 'bubble', label: '选择题涂卡'},
    {value: 'lines', label: '横线作答'},
    {value: 'box', label: '答题框'},
];

export function newAnswerSheetTemplate() {
    return {
        id: `custom-${Date.now().toString(36)}`,
        name: '未命名答题卡',
        description: '',
        builtin: false,
        config: {
            ...DEFAULT_ANSWER_SHEET_CONFIG,
            rules: DEFAULT_ANSWER_SHEET_CONFIG.rules.map((rule) => ({
                ...rule,
                types: [...rule.types],
            })),
        },
    };
}

/** 内置模板 + 用户自定义模板；同 id 时自定义优先，方便覆盖改坏的内置副本。 */
export function mergeAnswerSheets(custom = []) {
    const saved = (Array.isArray(custom) ? custom : []).filter((item) => item && item.id);
    const overridden = new Set(saved.map((item) => item.id));
    return [...BUILTIN_ANSWER_SHEETS.filter((item) => !overridden.has(item.id)), ...saved];
}

export async function loadAnswerSheets() {
    try {
        const raw = await settingGet(ANSWER_SHEET_STORE_KEY);
        const parsed = raw ? JSON.parse(raw) : [];
        return mergeAnswerSheets(parsed);
    } catch {
        return mergeAnswerSheets([]);
    }
}

export async function saveCustomAnswerSheets(list) {
    const custom = (list || []).filter((item) => !item.builtin);
    await settingSet(ANSWER_SHEET_STORE_KEY, JSON.stringify(custom));
    return mergeAnswerSheets(custom);
}

export function answerSheetOf(templates, id) {
    const list = templates?.length ? templates : BUILTIN_ANSWER_SHEETS;
    return list.find((item) => item.id === id) || list[0] || BUILTIN_ANSWER_SHEETS[0];
}
