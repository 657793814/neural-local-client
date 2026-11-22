// 拼音显示样式：后端只存带声调的拼音，数字调 / 无声调在这里转换。

const TONED = {
    'ā': ['a', 1], 'á': ['a', 2], 'ǎ': ['a', 3], 'à': ['a', 4],
    'ō': ['o', 1], 'ó': ['o', 2], 'ǒ': ['o', 3], 'ò': ['o', 4],
    'ē': ['e', 1], 'é': ['e', 2], 'ě': ['e', 3], 'è': ['e', 4],
    'ī': ['i', 1], 'í': ['i', 2], 'ǐ': ['i', 3], 'ì': ['i', 4],
    'ū': ['u', 1], 'ú': ['u', 2], 'ǔ': ['u', 3], 'ù': ['u', 4],
    'ǖ': ['ü', 1], 'ǘ': ['ü', 2], 'ǚ': ['ü', 3], 'ǜ': ['ü', 4],
};

export function formatPinyin(value, style = 'tone') {
    const text = String(value || '').trim();
    if (!text || style === 'tone') return text;
    return Array.from(text)
        .map((ch) => {
            const pair = TONED[ch];
            if (!pair) return ch;
            return style === 'num' ? `${pair[0]}${pair[1]}` : pair[0];
        })
        .join('');
}

/** 校对页选中某个读音：只替换对应字的音节，其余保持自动结果。 */
export function applyReading(item, candidateIndex, optionIndex) {
    const candidate = (item.pinyinCandidates || [])[candidateIndex];
    const option = candidate?.options?.[optionIndex];
    if (!candidate || !option) return item;
    const syllables = String(item.pinyin || '').trim().split(/\s+/).filter(Boolean);
    const at = Number(candidate.index || 0);
    if (at >= syllables.length) return item;
    syllables[at] = option.pinyin;
    const candidates = (item.pinyinCandidates || []).map((each, index) =>
        index === candidateIndex ? {...each, selected: optionIndex} : each,
    );
    return {
        ...item,
        pinyinCandidates: candidates,
        selectedPinyin: syllables.join(' '),
        speakAs: option.homophone || item.text,
        needsReview: candidates.some((each) => each.selected === undefined || each.selected === null),
    };
}
