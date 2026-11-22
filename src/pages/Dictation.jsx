import {useCallback, useEffect, useRef, useState} from 'react';
import {
    dictationOcrFromPath,
    dictationSetDelete,
    dictationSetGet,
    dictationSetList,
    dictationSetSave,
    saveTempImage,
    settingGet,
    settingSet,
} from '../lib/api.js';
import {Card, Empty, PageHead, Pill, confirmDialog, notify} from '../components/ui.jsx';
import DictationReview from '../components/DictationReview.jsx';
import DictationPlayer from '../components/DictationPlayer.jsx';

const TODAY = () => new Date().toISOString().slice(0, 10);

function readAsDataUrl(file) {
    return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(new Error('读图片失败'));
        reader.readAsDataURL(file);
    });
}

function parseSetting(raw, fallback) {
    if (raw === null || raw === undefined || raw === '') return fallback;
    try {
        return JSON.parse(raw);
    } catch {
        const number = Number(raw);
        return Number.isFinite(number) ? number : raw;
    }
}

export default function Dictation({nextTick = 0, repeatTick = 0}) {
    const [step, setStep] = useState('upload');
    const [draft, setDraft] = useState(null);
    const [activeSet, setActiveSet] = useState(null);
    const [sets, setSets] = useState([]);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState('');
    const [dragging, setDragging] = useState(false);
    const fileRef = useRef(null);
    const [intervalMs, setIntervalMs] = useState(5000);
    const [pinyinStyle, setPinyinStyle] = useState('tone');
    const [rate, setRate] = useState(0.9);

    const loadSets = useCallback(() => {
        dictationSetList()
            .then(setSets)
            .catch((err) => notify.error(`读取听写批次失败：${err.message}`));
    }, []);

    useEffect(() => {
        loadSets();
        Promise.all([
            settingGet('dictation.intervalMs'),
            settingGet('dictation.pinyinStyle'),
            settingGet('dictation.rate'),
        ]).then(([gap, style, speed]) => {
            setIntervalMs(Number(parseSetting(gap, 5000)) || 5000);
            setPinyinStyle(parseSetting(style, 'tone') || 'tone');
            setRate(Number(parseSetting(speed, 0.9)) || 0.9);
        }).catch(() => {});
    }, [loadSets]);

    const ingest = useCallback(async (file) => {
        if (!file || !file.type.startsWith('image/')) return;
        setBusy(true);
        setError('');
        try {
            const dataUrl = await readAsDataUrl(file);
            const path = await saveTempImage(dataUrl);
            const result = await dictationOcrFromPath(path);
            setDraft({
                title: file.name.replace(/\.[^.]+$/, ''),
                dictationDate: TODAY(),
                sourcePath: path,
                langMode: 'MIXED',
                items: result.items || [],
            });
            setStep('review');
        } catch (err) {
            setError(err.message);
        } finally {
            setBusy(false);
        }
    }, []);

    useEffect(() => {
        const onPaste = (event) => {
            if (step !== 'upload') return;
            const image = Array.from(event.clipboardData?.items || []).find((item) => item.type.startsWith('image/'));
            const file = image?.getAsFile();
            if (file) {
                event.preventDefault();
                ingest(file);
            }
        };
        window.addEventListener('paste', onPaste);
        return () => window.removeEventListener('paste', onPaste);
    }, [ingest, step]);

    const manual = () => {
        setDraft({
            title: `今日听写 ${TODAY()}`,
            dictationDate: TODAY(),
            sourcePath: '',
            langMode: 'MIXED',
            items: [{
                id: `manual-${Date.now()}`,
                ord: 1,
                text: '',
                lang: 'ZH',
                pinyin: '',
                pinyinCandidates: [],
                selectedPinyin: '',
                speakAs: '',
                meaning: '',
                needsReview: false,
            }],
        });
        setStep('review');
    };

    const save = async () => {
        setBusy(true);
        setError('');
        try {
            const id = await dictationSetSave(draft);
            const saved = await dictationSetGet(id);
            setActiveSet(saved);
            setStep('player');
            loadSets();
        } catch (err) {
            setError(err.message);
        } finally {
            setBusy(false);
        }
    };

    const open = async (id) => {
        setBusy(true);
        setError('');
        try {
            const saved = await dictationSetGet(id);
            setActiveSet(saved);
            setStep('player');
        } catch (err) {
            setError(err.message);
        } finally {
            setBusy(false);
        }
    };

    const remove = async (id) => {
        const ok = await confirmDialog({title: '删掉这份听写？', message: '只从列表移除，词条和原图会一并删除，不能撤销。', danger: true, confirmText: '删掉'});
        if (!ok) return;
        try {
            await dictationSetDelete(id);
            notify.success('已删除');
            loadSets();
        } catch (err) {
            notify.error(err.message);
        }
    };

    const changeSetting = (key, value, setter) => {
        setter(value);
        settingSet(key, JSON.stringify(value)).catch(() => {});
    };

    return (
        <div className="page dictation-page">
            <PageHead
                title="听写"
                subtitle="拍照识别听写清单，拼音常驻、多音字人工定音，系统语音逐条播报，播完留清单人工核对"
                help="dictation"
                actions={step !== 'upload' ? (
                    <button className="btn ghost" onClick={() => { setStep('upload'); setActiveSet(null); }}>返回列表</button>
                ) : null}
            />

            {error && <div className="dict-error">{error}</div>}

            {step === 'upload' && (
                <>
                    <Card title="第一步 · 放进今天的听写内容" subtitle="支持选图、拖入、直接粘贴截图；本机 OCR 离线识别，不需要模型 Key">
                        <div
                            className={`dict-drop ${dragging ? 'dragging' : ''}`}
                            onDragOver={(event) => { event.preventDefault(); setDragging(true); }}
                            onDragLeave={() => setDragging(false)}
                            onDrop={(event) => {
                                event.preventDefault();
                                setDragging(false);
                                ingest(event.dataTransfer.files?.[0]);
                            }}
                        >
                            <div className="dict-drop-mark">◇</div>
                            <p>把听写清单图片拖到这里，或</p>
                            <button className="btn primary" disabled={busy} onClick={() => fileRef.current?.click()}>
                                {busy ? '正在识别…' : '选择图片'}
                            </button>
                            <button className="btn ghost" onClick={manual}>手工新建</button>
                            <p className="muted">也可以直接 ⌘V 粘贴截图</p>
                            <input
                                ref={fileRef}
                                type="file"
                                accept="image/*"
                                hidden
                                onChange={(event) => {
                                    ingest(event.target.files?.[0]);
                                    event.target.value = '';
                                }}
                            />
                        </div>
                    </Card>

                    <Card title="设置" subtitle="默认间隔 5 秒，念完才开始计时">
                        <div className="dict-settings">
                            <label>
                                <span>词条间隔</span>
                                <select value={intervalMs} onChange={(event) => changeSetting('dictation.intervalMs', Number(event.target.value), setIntervalMs)}>
                                    <option value={3000}>3 秒</option>
                                    <option value={5000}>5 秒（默认）</option>
                                    <option value={8000}>8 秒</option>
                                    <option value={10000}>10 秒</option>
                                </select>
                            </label>
                            <label>
                                <span>拼音显示</span>
                                <select value={pinyinStyle} onChange={(event) => changeSetting('dictation.pinyinStyle', event.target.value, setPinyinStyle)}>
                                    <option value="tone">带声调（yín háng）</option>
                                    <option value="num">数字调（yin2 hang2）</option>
                                    <option value="plain">无声调（yin hang）</option>
                                </select>
                            </label>
                            <label>
                                <span>语速</span>
                                <select value={rate} onChange={(event) => changeSetting('dictation.rate', Number(event.target.value), setRate)}>
                                    <option value={0.7}>慢</option>
                                    <option value={0.9}>正常</option>
                                    <option value={1.1}>快</option>
                                </select>
                            </label>
                        </div>
                    </Card>

                    <Card title="已有批次" subtitle="只保留听写内容，随时重新播报；不做判分与报告">
                        {sets.length === 0 ? (
                            <Empty title="还没有听写批次" hint="传一张今天的听写清单开始吧"/>
                        ) : (
                            <div className="dict-sets">
                                {sets.map((item) => (
                                    <div key={item.id} className="dict-set">
                                        <div>
                                            <strong>{item.title}</strong>
                                            <small>{item.date || '未标日期'} · {item.itemCount} 条</small>
                                        </div>
                                        <div className="dict-set-actions">
                                            <Pill>{item.langMode || 'MIXED'}</Pill>
                                            <button className="btn ghost small" onClick={() => open(item.id)}>打开并重听</button>
                                            <button className="icon-btn tiny" title="删除" onClick={() => remove(item.id)}>✕</button>
                                        </div>
                                    </div>
                                ))}
                            </div>
                        )}
                    </Card>
                </>
            )}

            {step === 'review' && draft && (
                <DictationReview
                    items={draft.items}
                    pinyinStyle={pinyinStyle}
                    onChange={(next) => setDraft((value) => ({
                        ...value,
                        items: typeof next === 'function' ? next(value.items) : next,
                    }))}
                    onBack={() => setStep('upload')}
                    onSave={save}
                />
            )}

            {step === 'player' && activeSet && (
                <DictationPlayer
                    key={activeSet.id}
                    set={activeSet}
                    intervalMs={intervalMs}
                    rate={rate}
                    pinyinStyle={pinyinStyle}
                    nextTick={nextTick}
                    repeatTick={repeatTick}
                    onExit={() => { setStep('upload'); setActiveSet(null); }}
                />
            )}
        </div>
    );
}
