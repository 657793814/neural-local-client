/**
 * 成绩批量登分：把从 Excel / 成绩册复制来的文本解析成一行行「学号 → 分数」。
 *
 * 名单（ExamSheet.rows，含 studentNo / name）已经在前端，匹配是纯前端的事：
 * 不新增后端命令，也不碰数据库——解析结果先填进登分草稿，用户确认后再走原来的保存校验。
 */

// 表头别名：中英文常见写法都认，避免老师为了导入先改表头。
const HEADERS = {
    no: ['学号', '考号', '编号', '学生编号', 'studentno', 'studentid', 'no', 'id'],
    name: ['姓名', '名字', '学生姓名', 'name'],
    score: ['分数', '成绩', '得分', '分值', 'score', 'points'],
    note: ['备注', '说明', 'note', 'remark'],
};

// 空分数：按「缺考」处理（写进草稿是空串，与 0 分区分）。
const BLANK_SCORES = new Set([
    '', '-', '—', '–', '缺考', '缺', '未考', '没考', '免考', '无', 'absent', 'na', 'n/a',
]);

/** 全角数字 / 字母 / 标点 → 半角。学号从 Excel 复制过来经常带全角。 */
function toHalfWidth(value) {
    return String(value ?? '')
        .replace(/^\ufeff/, '')
        .replace(/[！-～]/g, (ch) => String.fromCharCode(ch.charCodeAt(0) - 0xfee0));
}

/** 学号归一化：去空白、去 Excel 文本标记 '、把 2026001.0 还原成 2026001。 */
export function normalizeNo(value) {
    let text = toHalfWidth(value).trim();
    text = text.replace(/^'+/, '').replace(/\s+/g, '');
    if (/^\d+\.0+$/.test(text)) text = text.replace(/\.0+$/, '');
    return text;
}

function round1(value) {
    return Math.round(value * 10) / 10;
}

/** 优先 Tab（Excel 复制），其次逗号 / 分号 / 顿号，最后按空白切。 */
function splitCells(line) {
    if (line.includes('\t')) return line.split('\t').map((cell) => cell.trim());
    if (line.includes(',')) return line.split(',').map((cell) => cell.trim());
    if (/[;；、]/.test(line)) return line.split(/[;；、]/).map((cell) => cell.trim());
    return line.trim().split(/\s+/).filter(Boolean);
}

function detectHeader(cells) {
    const found = {no: -1, name: -1, score: -1, note: -1};
    cells.forEach((cell, at) => {
        const text = toHalfWidth(cell).trim().toLowerCase().replace(/\s+/g, '');
        Object.entries(HEADERS).forEach(([key, aliases]) => {
            if (found[key] < 0 && aliases.includes(text)) found[key] = at;
        });
    });
    if (found.no < 0 && found.name < 0 && found.score < 0) return null;
    return found;
}

function parseScoreCell(value, fullScore) {
    let text = toHalfWidth(value).trim().replace(/\s+/g, '').replace(/分$/, '');
    if (BLANK_SCORES.has(text.toLowerCase())) return {blank: true, value: null};
    if (!/^\d+(?:\.\d+)?$/.test(text)) return {error: '分数不是数字'};
    const number = Number(text);
    if (!(number >= 0 && number <= fullScore)) {
        return {error: `分数超出 0–${fullScore}`};
    }
    return {blank: false, value: round1(number)};
}

/** 数字单元格（用来在没有表头时猜分数列；名次之类也可能撞上，故只在学号列之后找）。 */
function isNumberCell(value) {
    return /^\d+(?:\.\d+)?$/.test(toHalfWidth(value).trim().replace(/分$/, ''));
}

function resolveRow(cells, header, byNo, byName, fullScore) {
    let noIndex = header ? header.no : -1;
    let nameIndex = header ? header.name : -1;
    let scoreIndex = header ? header.score : -1;

    if (!header) {
        // 无表头：先找能直接对上名单的学号；对不上再用姓名兜底（重名不算）。
        noIndex = cells.findIndex((cell) => byNo.has(normalizeNo(cell)));
        if (noIndex < 0) nameIndex = cells.findIndex((cell) => byName.has(String(cell).trim()));
        const afterId = Math.max(noIndex, nameIndex) + 1;
        for (let at = afterId; at < cells.length; at += 1) {
            if (isNumberCell(cells[at])) {
                scoreIndex = at;
                break;
            }
        }
        // 兼容「分数 学号」这种倒序，再全行找一次。
        if (scoreIndex < 0) {
            for (let at = 0; at < cells.length; at += 1) {
                if (at !== noIndex && isNumberCell(cells[at])) {
                    scoreIndex = at;
                    break;
                }
            }
        }
    }

    const rawNo = noIndex >= 0 ? cells[noIndex] || '' : '';
    const rawName = nameIndex >= 0 ? cells[nameIndex] || '' : '';
    const rawScore = scoreIndex >= 0 && scoreIndex < cells.length ? cells[scoreIndex] : '';

    let student = byNo.get(normalizeNo(rawNo)) || null;
    let matchBy = student ? 'no' : null;
    if (!student && rawName) {
        student = byName.get(rawName.trim()) || null;
        if (student) matchBy = 'name';
    }

    const parsed = scoreIndex < 0 ? {error: '没读到分数'} : parseScoreCell(rawScore, fullScore);

    return {
        rawNo,
        rawName,
        rawScore,
        raw: cells.join(' '),
        studentId: student?.studentId || '',
        name: student?.name || rawName.trim(),
        score: parsed.error ? null : parsed.value,
        matchBy,
        status: !student ? 'missing' : parsed.error ? 'invalid' : 'ok',
        reason: !student ? '名单里没有这个学号' : parsed.error || '',
        blank: !parsed.error && parsed.blank,
    };
}

/**
 * 解析粘贴文本。
 * @param {string} text 从 Excel / 成绩册复制的原始文本
 * @param {Array<{studentId:string, studentNo:string, name:string}>} roster 本场考试的名单
 * @param {number} fullScore 满分，用来卡分数范围
 * @returns {{rows: Array, total: number, matched: number, missing: number, invalid: number, duplicate: number, blank: number}}
 */
export function parseScoreImport(text, roster = [], fullScore = 100) {
    const byNo = new Map();
    const byName = new Map();
    for (const row of roster) {
        const no = normalizeNo(row.studentNo);
        if (no && !byNo.has(no)) byNo.set(no, row);
        const name = String(row.name ?? '').trim();
        if (name) {
            // 重名不敢当匹配依据，宁可让用户手动填。
            byName.set(name, byName.has(name) ? null : row);
        }
    }

    const rows = [];
    const seen = new Set();
    let header = null;
    let firstLine = true;
    const lines = String(text ?? '').replace(/\r\n?/g, '\n').split('\n');

    lines.forEach((rawLine, index) => {
        if (!rawLine.trim()) return;
        const cells = splitCells(rawLine.trim());
        if (!cells.length) return;
        if (firstLine) {
            firstLine = false;
            header = detectHeader(cells);
            if (header) return;
        }
        const row = {...resolveRow(cells, header, byNo, byName, fullScore), line: index + 1};
        if (row.status === 'ok') {
            if (seen.has(row.studentId)) {
                row.status = 'duplicate';
                row.reason = '重复，跳过';
            } else {
                seen.add(row.studentId);
            }
        }
        rows.push(row);
    });

    const count = (status) => rows.filter((row) => row.status === status).length;
    return {
        rows,
        total: rows.length,
        matched: count('ok'),
        missing: count('missing'),
        invalid: count('invalid'),
        duplicate: count('duplicate'),
        blank: rows.filter((row) => row.status === 'ok' && row.blank).length,
    };
}
