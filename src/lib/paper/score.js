/**
 * 分值体系：总分是合同值，`sum(题分) == total` 才能定稿。
 * 这里只做展示与本地预判，真正的硬校验在 Rust 侧 `paper::finalize`。
 */

export function scoreText(value) {
    const number = Number(value || 0);
    const rounded = Math.round(number * 100) / 100;
    return Number.isInteger(rounded) ? String(rounded) : String(rounded);
}

export function roundScore(value) {
    return Math.round(Number(value || 0) * 1000) / 1000;
}

export function sectionScore(section) {
    return (section?.questions || []).reduce((sum, item) => sum + Number(item.score || 0), 0);
}

/** 全卷汇总：已分配、差额（正数=还差）、是否配平。 */
export function scoreSummary(doc) {
    const sections = doc?.sections || [];
    const assigned = roundScore(
        sections.reduce((sum, section) => sum + sectionScore(section), 0),
    );
    const total = roundScore(doc?.totalScore || 0);
    const delta = roundScore(total - assigned);
    return {
        total,
        assigned,
        delta,
        balanced: Math.abs(delta) < 0.001,
        count: sections.reduce((sum, section) => sum + (section.questions?.length || 0), 0),
    };
}

/** 差额条文案：`已分配 148 / 总分 150 · 还差 2 分`。 */
export function deltaText(summary) {
    if (summary.balanced) return `已分配 ${scoreText(summary.assigned)} / 总分 ${scoreText(summary.total)} · 已配平`;
    const direction = summary.delta > 0 ? '还差' : '超出';
    return `已分配 ${scoreText(summary.assigned)} / 总分 ${scoreText(summary.total)} · ${direction} ${scoreText(Math.abs(summary.delta))} 分`;
}

/** 改大题统一分值时，把本大题所有题一起重算（前端立即反馈，后端再落库）。 */
export function applyUnitScore(doc, sectionId, unitScore) {
    return {
        ...doc,
        sections: (doc.sections || []).map((section) => {
            if (section.id !== sectionId) return section;
            const unit = Number(unitScore) || 0;
            return {
                ...section,
                scoreMode: 'UNIFORM',
                unitScore: unit,
                questions: (section.questions || []).map((item) => ({...item, score: unit})),
            };
        }),
    };
}

export function applyItemScore(doc, sectionId, ord, score) {
    return {
        ...doc,
        sections: (doc.sections || []).map((section) => {
            if (section.id !== sectionId) return section;
            return {
                ...section,
                scoreMode: 'PER_ITEM',
                questions: (section.questions || []).map((item) =>
                    item.ord === ord ? {...item, score: Number(score) || 0} : item,
                ),
            };
        }),
    };
}
