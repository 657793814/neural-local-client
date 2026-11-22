import {useCallback, useEffect, useRef, useState} from 'react';
import {formatPinyin} from '../lib/pinyin.js';
import {notify} from './ui.jsx';

/**
 * 听写播放台：Web Speech API 调用系统离线音色，onend 之后才开始 5s 计时。
 * 自动计时 / 按钮 / 快捷键 / 全局热键都只调 next()，状态机只有一份。
 */
export default function DictationPlayer({
    set, intervalMs = 5000, rate = 0.9, pinyinStyle = 'tone',
    nextTick = 0, repeatTick = 0, onExit,
}) {
    const items = set.items || [];
    const [index, setIndex] = useState(0);
    const [status, setStatus] = useState('idle');
    const [showAnswer, setShowAnswer] = useState(false);
    const [voices, setVoices] = useState([]);
    const current = useRef(0);
    const statusRef = useRef('idle');
    const timer = useRef(null);
    const cancelled = useRef(false);
    const started = useRef(false);

    useEffect(() => {
        statusRef.current = status;
    }, [status]);

    useEffect(() => {
        const synth = window.speechSynthesis;
        if (!synth) {
            notify.error('当前环境没有系统语音，无法播报');
            return undefined;
        }
        const load = () => setVoices(synth.getVoices() || []);
        load();
        synth.addEventListener?.('voiceschanged', load);
        return () => synth.removeEventListener?.('voiceschanged', load);
    }, []);

    const silence = useCallback(() => {
        cancelled.current = true;
        window.clearTimeout(timer.current);
        window.speechSynthesis?.cancel();
    }, []);

    const speakAt = useCallback((at) => {
        const item = items[at];
        if (!item) {
            setStatus('finished');
            return;
        }
        silence();
        cancelled.current = false;
        current.current = at;
        setIndex(at);
        setStatus('speaking');
        const text = String(item.speakAs || item.text || '').trim();
        if (!text) {
            setStatus('finished');
            return;
        }
        const utterance = new SpeechSynthesisUtterance(text);
        utterance.lang = item.lang === 'EN' ? 'en-US' : 'zh-CN';
        const prefix = item.lang === 'EN' ? 'en' : 'zh';
        const voice = voices.find((each) => each.lang?.toLowerCase().startsWith(prefix));
        if (voice) utterance.voice = voice;
        utterance.rate = Number(rate) || 0.9;
        utterance.onend = () => {
            if (cancelled.current) return;
            setStatus('gap');
            timer.current = window.setTimeout(() => {
                if (at + 1 >= items.length) {
                    setStatus('finished');
                    return;
                }
                speakAt(at + 1);
            }, Number(intervalMs) || 5000);
        };
        utterance.onerror = (event) => {
            if (cancelled.current) return;
            if (event.error === 'interrupted' || event.error === 'canceled') return;
            notify.error(`语音播报失败：${event.error}`);
            setStatus('paused');
        };
        window.speechSynthesis.speak(utterance);
    }, [items, voices, rate, intervalMs, silence]);

    const next = useCallback(() => {
        if (statusRef.current === 'finished' || statusRef.current === 'idle') {
            speakAt(0);
            return;
        }
        const at = current.current + 1;
        if (at >= items.length) {
            silence();
            setStatus('finished');
            return;
        }
        speakAt(at);
    }, [items.length, silence, speakAt]);

    const repeat = useCallback(() => speakAt(current.current), [speakAt]);
    const pause = () => {
        silence();
        setStatus('paused');
    };
    const resume = () => speakAt(current.current);
    const stop = () => {
        silence();
        setStatus('idle');
        setIndex(0);
        current.current = 0;
    };

    // 只启动一次；音色异步到达后 speakAt 会带上合适音色，不需要重播。
    useEffect(() => {
        if (!started.current && items.length) {
            started.current = true;
            speakAt(0);
        }
        return () => silence();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [set.id]);

    useEffect(() => {
        if (nextTick > 0) next();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [nextTick]);
    useEffect(() => {
        if (repeatTick > 0) repeat();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [repeatTick]);

    useEffect(() => {
        const onKey = (event) => {
            const tag = event.target?.tagName;
            if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
            if (event.code === 'Space') {
                event.preventDefault();
                repeat();
            } else if (event.key === 'ArrowRight') {
                event.preventDefault();
                next();
            } else if (event.key === 'Escape') {
                stop();
            }
        };
        window.addEventListener('keydown', onKey);
        return () => window.removeEventListener('keydown', onKey);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [repeat, next]);

    const item = items[index];
    const finite = status === 'finished';

    return (
        <section className="card dict-player">
            <div className="card-head">
                <div>
                    <h2>第三步 · 听写与核对</h2>
                    <p>念完才开始倒计时。暂停 / 下一个 / 重播只推进一条；播完保留整批清单供人工核对。</p>
                </div>
                <div className="card-actions">
                    <button className="btn ghost" onClick={onExit}>返回</button>
                </div>
            </div>

            <div className="dict-stage">
                <div className="dict-progress">
                    <strong>{finite ? items.length : Math.min(index + 1, items.length)}</strong>
                    <span>/ {items.length}</span>
                    <span className={`dict-state ${status}`}>
                        {status === 'speaking' ? '正在播报' : status === 'gap' ? `间隔 ${Math.round(intervalMs / 1000)}s`
                            : status === 'paused' ? '已暂停' : finite ? '已播完' : '准备中'}
                    </span>
                </div>

                {!finite && item && (
                    <div className="dict-current">
                        <div className="dict-current-word">
                            {showAnswer ? item.text : '••••'}
                        </div>
                        {showAnswer && item.lang === 'ZH' && (
                            <div className="dict-current-pinyin">
                                {formatPinyin(item.selectedPinyin || item.pinyin, pinyinStyle)}
                            </div>
                        )}
                        <button className="linklike" onClick={() => setShowAnswer((value) => !value)}>
                            {showAnswer ? '隐藏答案' : '看一眼答案'}
                        </button>
                    </div>
                )}

                <div className="dict-controls">
                    {status === 'paused' ? (
                        <button className="btn primary" onClick={resume}>继续</button>
                    ) : finite ? (
                        <button className="btn primary" onClick={() => speakAt(0)}>重听这一批</button>
                    ) : (
                        <button className="btn ghost" onClick={pause}>暂停</button>
                    )}
                    <button className="btn ghost" onClick={repeat} disabled={finite}>重播这条 <kbd>⌘⇧R</kbd></button>
                    <button className="btn primary" onClick={next} disabled={finite}>播报下一个 <kbd>⌘⇧N</kbd></button>
                    <button className="btn ghost" onClick={stop}>结束</button>
                </div>
                <p className="dict-hotkey-note">播放台内：<kbd>空格</kbd> 重播 · <kbd>→</kbd> 下一个 · <kbd>Esc</kbd> 结束</p>
            </div>

            {finite && (
                <div className="dict-checklist">
                    <h3>本批清单（人工核对用，不判分）</h3>
                    <ol>
                        {items.map((each, at) => (
                            <li key={each.id || at}>
                                <b>{each.text}</b>
                                {each.lang === 'ZH' && (
                                    <span>{formatPinyin(each.selectedPinyin || each.pinyin, pinyinStyle)}</span>
                                )}
                                {each.meaning && <small>{each.meaning}</small>}
                            </li>
                        ))}
                    </ol>
                </div>
            )}
        </section>
    );
}
