import {useRef} from 'react';
import {dictationPinyin} from '../lib/api.js';
import {applyReading, formatPinyin} from '../lib/pinyin.js';
import {notify} from './ui.jsx';

/** 校对页：拼音常驻，多音字必须点选，试听念的是 speakAs。 */
export default function DictationReview({items, onChange, onBack, onSave, pinyinStyle}) {
    const reqSeq = useRef(0);
    const update = (index, patch) => onChange((current) => {
        const next = current.slice();
        next[index] = {
            ...next[index],
            ...(typeof patch === 'function' ? patch(next[index]) : patch),
        };
        return next;
    });

    const onText = (index, text) => {
        const lang = /[\u3400-\u9fff]/.test(text) ? 'ZH' : 'EN';
        update(index, (current) => ({
            text,
            lang,
            selectedPinyin: '',
            speakAs: text,
            needsReview: current.needsReview,
        }));
        if (lang !== 'ZH' || !text.trim()) {
            update(index, {pinyin: '', pinyinCandidates: []});
            return;
        }
        const seq = ++reqSeq.current;
        dictationPinyin(text)
            .then((result) => {
                if (seq !== reqSeq.current) return;
                update(index, (current) => ({
                    pinyin: result.pinyin || '',
                    pinyinCandidates: result.candidates || [],
                    needsReview: !!(result.candidates || []).length || current.needsReview,
                }));
            })
            .catch((error) => notify.error(`拼音带出失败：${error.message}`));
    };

    const choose = (index, candidateIndex, optionIndex) => onChange((current) => {
        const next = current.slice();
        next[index] = applyReading(next[index], candidateIndex, optionIndex);
        return next;
    });

    const move = (index, delta) => {
        const to = index + delta;
        if (to < 0 || to >= items.length) return;
        onChange((current) => {
            const next = current.slice();
            [next[index], next[to]] = [next[to], next[index]];
            return next.map((item, at) => ({...item, ord: at + 1}));
        });
    };

    const remove = (index) => onChange((current) => current
        .filter((_, at) => at !== index)
        .map((item, at) => ({...item, ord: at + 1})));

    const add = () => onChange((current) => [
        ...current,
        {
                id: `manual-${Date.now()}`,
                ord: current.length + 1,
                text: '',
                lang: 'ZH',
                pinyin: '',
                pinyinCandidates: [],
                selectedPinyin: '',
                speakAs: '',
                meaning: '',
                needsReview: false,
            },
        ]);

    const trial = (item) => {
        const text = item.speakAs || item.text;
        if (!text.trim()) return;
        if (!window.speechSynthesis) {
            notify.error('当前浏览器没有系统语音，不能试听');
            return;
        }
        window.speechSynthesis.cancel();
        const utterance = new SpeechSynthesisUtterance(text);
        utterance.lang = item.lang === 'EN' ? 'en-US' : 'zh-CN';
        utterance.rate = 0.9;
        window.speechSynthesis.speak(utterance);
    };

    const pending = items.filter(
        (item) => item.lang === 'ZH'
            && (item.pinyinCandidates || []).some((each) => each.selected === undefined || each.selected === null),
    ).length;

    return (
        <section className="card dict-review">
            <div className="card-head">
                <div>
                    <h2>第二步 · 校对词条</h2>
                    <p>中文拼音常驻显示。带多音字的词条必须点选一个读音，才能开始听写。</p>
                </div>
                <div className="card-actions">
                    <button className="btn ghost" onClick={onBack}>返回选图</button>
                    <button className="btn primary" disabled={pending > 0} onClick={onSave}>
                        确认开始{pending > 0 ? `（还有 ${pending} 条未定音）` : ''}
                    </button>
                </div>
            </div>

            <div className="dict-review-list">
                {items.map((item, index) => {
                    const candidates = item.pinyinCandidates || [];
                    const syllables = String(item.pinyin || '').trim().split(/\s+/);
                    const shown = item.selectedPinyin || item.pinyin || '';
                    return (
                        <div key={item.id || index} className={`dict-row ${candidates.some((each) => each.selected === undefined || each.selected === null) ? 'needs' : ''}`}>
                            <div className="dict-row-no">
                                <span>{index + 1}</span>
                                <div className="dict-row-move">
                                    <button className="icon-btn tiny" title="上移" onClick={() => move(index, -1)}>↑</button>
                                    <button className="icon-btn tiny" title="下移" onClick={() => move(index, 1)}>↓</button>
                                </div>
                            </div>

                            <div className="dict-row-main">
                                <div className="dict-row-line">
                                    <input
                                        className="dict-text"
                                        value={item.text}
                                        placeholder="词条内容"
                                        onChange={(event) => onText(index, event.target.value)}
                                    />
                                    <select
                                        className="dict-lang"
                                        value={item.lang}
                                        onChange={(event) => update(index, {
                                            lang: event.target.value,
                                            pinyin: event.target.value === 'EN' ? '' : item.pinyin,
                                            pinyinCandidates: event.target.value === 'EN' ? [] : candidates,
                                            selectedPinyin: event.target.value === 'EN' ? '' : item.selectedPinyin,
                                        })}
                                    >
                                        <option value="ZH">语文</option>
                                        <option value="EN">英语</option>
                                    </select>
                                    <button className="btn ghost small" onClick={() => trial(item)}>试听</button>
                                    <button className="icon-btn tiny" title="删掉这条" onClick={() => remove(index)}>✕</button>
                                </div>

                                <div className="dict-pinyin-line">
                                    <span className="dict-label">拼音</span>
                                    <strong className="dict-pinyin">{formatPinyin(shown, pinyinStyle) || '—'}</strong>
                                    {item.needsReview && <span className="pill warn">OCR 需核对</span>}
                                </div>

                                {candidates.length > 0 && (
                                    <div className="dict-candidates">
                                        <span className="dict-label">多音字定音</span>
                                        {candidates.map((candidate, ci) => (
                                            <span key={ci} className="dict-candidate">
                                                <b>{candidate.char}</b>
                                                {candidate.options.map((option, oi) => {
                                                    const active = syllables[Number(candidate.index || 0)] === option.pinyin;
                                                    return (
                                                        <button
                                                            key={oi}
                                                            className={`dict-choice ${active ? 'on' : ''}`}
                                                            title={option.homophone ? `发音提示字：${option.homophone}` : ''}
                                                            onClick={() => choose(index, ci, oi)}
                                                        >
                                                            {formatPinyin(option.pinyin, pinyinStyle)}
                                                        </button>
                                                    );
                                                })}
                                            </span>
                                        ))}
                                    </div>
                                )}

                                <div className="dict-row-extra">
                                    <label>
                                        <span>按这个读</span>
                                        <input
                                            value={item.speakAs || ''}
                                            placeholder="读音不准时填同音字，如「航」"
                                            onChange={(event) => update(index, {speakAs: event.target.value})}
                                        />
                                    </label>
                                    <label>
                                        <span>释义（可选）</span>
                                        <input
                                            value={item.meaning || ''}
                                            placeholder="核对时给自己看的备注"
                                            onChange={(event) => update(index, {meaning: event.target.value})}
                                        />
                                    </label>
                                </div>
                            </div>
                        </div>
                    );
                })}
                <button className="btn ghost dict-add" onClick={add}>＋ 手工添加一条</button>
            </div>
        </section>
    );
}
