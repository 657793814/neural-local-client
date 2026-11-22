import {useCallback, useEffect, useMemo, useState} from 'react';
import {
    composeDraft,
    composePool,
    composeSwap,
    listQuestions,
    settingGet,
    settingSet,
} from '../lib/api.js';
import {Rich} from '../lib/Math.jsx';
import {TYPES, typeLabel} from '../lib/format.js';
import {Field, Modal, Pill, notify} from './ui.jsx';

const BANDS = [
    {key: 'easy', label: '易', hint: '难度 1–2'},
    {key: 'middle', label: '中', hint: '难度 3'},
    {key: 'hard', label: '难', hint: '难度 4–5'},
];

const DEFAULT_COUNTS = {easy: 3, middle: 4, hard: 2};
const DEFAULT_SCORES = {SINGLE: 3, MULTI: 4, FILL: 4, SOLVED: 10};
const PROFILE_KEY = 'compose.scoreProfile';

function scoreOf(scores, kind) {
    const value = Number(scores[kind]);
    return Number.isFinite(value) && value > 0 ? value : 1;
}

/** 编辑已有作业时没有 band 字段，按难度倒推，换题才能落在同一档。 */
function bandOf(difficulty, band) {
    if (band) return band;
    const level = Number(difficulty) || 3;
    if (level <= 2) return 'easy';
    if (level >= 4) return 'hard';
    return 'middle';
}

function totalOf(questions) {
    return questions.reduce((sum, item) => sum + (Number(item.score) || 0), 0);
}

function scoreText(value) {
    const number = Number(value || 0);
    return Number.isInteger(number) ? String(number) : number.toFixed(1);
}

/**
 * 布置作业：先选「考哪些知识点 + 每档出几题」，生成预览后逐题改。
 *
 * 为什么不是「先搜题再一道道勾」：离线题库动辄几千题，老师要的是
 * 「这几节课讲的知识点，各来几道」，而不是自己在列表里翻。手选题仍然保留，
 * 作为生成结果的补充（`QuestionPicker`）。
 *
 * 分值按题型给默认值并记住（`compose.scoreProfile`）：题库没有「题分」概念，
 * 这份画像就是作业分值的来源，比每题默认 1 分合理得多。
 */
export default function AssignmentComposer({classId, subjectId, initial, onClose, onSave}) {
    const editing = Boolean(initial?.id);
    const [stage, setStage] = useState(editing ? 'preview' : 'compose');
    const [meta, setMeta] = useState({
        title: initial?.title || '',
        type: initial?.type || 'PRACTICE',
        description: initial?.description || '',
        dueAt: initial?.dueAt || '',
        lateHours: initial?.lateHours || 0,
    });
    const [questions, setQuestions] = useState(
        (initial?.questions || []).map((item) => ({
            questionId: item.questionId,
            score: item.score,
            stem: item.stem,
            type: item.type,
            typeLabel: typeLabel(item.type),
            difficulty: item.difficulty,
            knowledge: item.knowledge || [],
            band: bandOf(item.difficulty),
        })),
    );
    const [pool, setPool] = useState(null);
    const [picked, setPicked] = useState([]);
    const [keyword, setKeyword] = useState('');
    const [counts, setCounts] = useState(DEFAULT_COUNTS);
    const [types, setTypes] = useState([]);
    const [scores, setScores] = useState(DEFAULT_SCORES);
    const [gaps, setGaps] = useState([]);
    const [busy, setBusy] = useState(false);
    const [picker, setPicker] = useState(false);

    useEffect(() => {
        if (!subjectId) return;
        composePool(subjectId)
            .then(setPool)
            .catch(() => setPool(null));
    }, [subjectId]);

    useEffect(() => {
        settingGet(PROFILE_KEY)
            .then((raw) => {
                if (!raw) return;
                const parsed = JSON.parse(raw);
                setScores({...DEFAULT_SCORES, ...parsed});
            })
            .catch(() => {});
    }, []);

    const rememberScores = useCallback((next) => {
        setScores(next);
        settingSet(PROFILE_KEY, JSON.stringify(next)).catch(() => {});
    }, []);

    const knowledge = pool?.knowledge || [];
    const filtered = useMemo(() => {
        const word = keyword.trim();
        if (!word) return knowledge;
        return knowledge.filter((item) => `${item.name}${item.path}`.includes(word));
    }, [knowledge, keyword]);

    const pickedNames = useMemo(
        () => knowledge.filter((item) => picked.includes(item.id)).map((item) => item.name),
        [knowledge, picked],
    );

    const available = useMemo(() => {
        if (!pool) return null;
        if (!picked.length) return {easy: pool.easy, middle: pool.middle, hard: pool.hard};
        return picked.reduce(
            (acc, id) => {
                const item = knowledge.find((each) => each.id === id);
                if (!item) return acc;
                return {
                    easy: acc.easy + item.easy,
                    middle: acc.middle + item.middle,
                    hard: acc.hard + item.hard,
                };
            },
            {easy: 0, middle: 0, hard: 0},
        );
    }, [pool, picked, knowledge]);

    const planned = counts.easy + counts.middle + counts.hard;

    const toggleKnowledge = (id) => {
        setPicked((current) =>
            current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
        );
    };

    const toggleType = (value) => {
        setTypes((current) =>
            current.includes(value) ? current.filter((item) => item !== value) : [...current, value],
        );
    };

    const generate = async () => {
        if (planned === 0) {
            // 一题都不出也算合法：老师可能只想手选。直接进预览，那里有「手动加题」。
            setQuestions([]);
            setGaps([]);
            setStage('preview');
            notify.info('没设题数，直接手选题吧');
            return;
        }
        setBusy(true);
        try {
            const out = await composeDraft({
                subjectId: subjectId || null,
                knowledgeIds: picked,
                counts,
                types,
                exclude: [],
            });
            setQuestions(
                out.questions.map((item) => ({
                    questionId: item.id,
                    score: scoreOf(scores, item.type),
                    stem: item.stem,
                    type: item.type,
                    typeLabel: item.typeLabel,
                    difficulty: item.difficulty,
                    knowledge: item.knowledge,
                    band: item.band,
                })),
            );
            setGaps(out.gaps || []);
            if (!meta.title.trim()) {
                const head = pickedNames.slice(0, 2).join('、') || pool?.knowledge?.[0]?.name || '练习';
                setMeta((current) => ({...current, title: `${head} · 练习`}));
            }
            setStage('preview');
            if (!out.questions.length) {
                notify.warning('这个条件下一道题都没抽到，可以回去放宽条件，或者手动加题');
            } else if (out.gaps?.length) {
                const text = out.gaps
                    .map((gap) => `${gap.label} 想要 ${gap.need} 道，只有 ${gap.got} 道`)
                    .join('；');
                notify.info(`题没凑齐：${text}。可以换条件，或先用现有的题。`);
            }
        } catch (error) {
            notify.error(error.message || String(error));
        } finally {
            setBusy(false);
        }
    };

    const update = (index, patch) => {
        setQuestions((current) =>
            current.map((item, at) => (at === index ? {...item, ...patch} : item)),
        );
    };

    const move = (index, delta) => {
        setQuestions((current) => {
            const next = [...current];
            const target = index + delta;
            if (target < 0 || target >= next.length) return current;
            [next[index], next[target]] = [next[target], next[index]];
            return next;
        });
    };

    const swap = async (index) => {
        const item = questions[index];
        setBusy(true);
        try {
            const out = await composeSwap({
                subjectId: subjectId || null,
                knowledgeIds: picked,
                band: bandOf(item.difficulty, item.band),
                types,
                exclude: questions.map((each) => each.questionId),
            });
            if (!out) {
                notify.info('这个条件没有别的题了；放宽知识点或手选一道');
                return;
            }
            update(index, {
                questionId: out.id,
                stem: out.stem,
                type: out.type,
                typeLabel: out.typeLabel,
                difficulty: out.difficulty,
                knowledge: out.knowledge,
                band: out.band,
                score: scoreOf(scores, out.type),
            });
        } catch (error) {
            notify.error(error.message || String(error));
        } finally {
            setBusy(false);
        }
    };

    const submit = async () => {
        if (!meta.title.trim()) {
            notify.warning('先给作业起个名字');
            return;
        }
        if (!questions.length) {
            notify.warning('至少留一道题');
            return;
        }
        setBusy(true);
        try {
            await onSave({
                id: initial?.id || null,
                classId,
                title: meta.title.trim(),
                type: meta.type,
                description: meta.description,
                dueAt: meta.dueAt || null,
                lateHours: Number(meta.lateHours) || 0,
                questions: questions.map((item) => ({
                    questionId: item.questionId,
                    score: Number(item.score) || 0,
                })),
            });
        } catch (error) {
            notify.error(error.message || String(error));
        } finally {
            setBusy(false);
        }
    };

    const title = editing ? '编辑作业' : stage === 'compose' ? '布置作业' : '作业预览';
    const subtitle =
        stage === 'compose'
            ? '先选考哪些知识点、每档出几题；生成后逐题还能换、能改分'
            : '检查一遍再保存；发布后题目与分值不再改，避免成绩对不上';

    return (
        <Modal
            title={title}
            subtitle={subtitle}
            wide
            onClose={onClose}
            footer={
                stage === 'compose' ? (
                    <>
                        <button className="btn ghost" onClick={onClose}>
                            取消
                        </button>
                        <button className="btn primary" onClick={generate} disabled={busy}>
                            {busy ? '正在抽题…' : '生成预览'}
                        </button>
                    </>
                ) : (
                    <>
                        <button className="btn ghost" onClick={onClose}>
                            取消
                        </button>
                        <button
                            className="btn ghost"
                            onClick={() => setStage('compose')}
                            disabled={busy}
                        >
                            {editing ? '重新组卷' : '上一步：改条件'}
                        </button>
                        <button className="btn primary" onClick={submit} disabled={busy}>
                            {busy ? '正在保存…' : editing ? '保存' : '保存草稿'}
                        </button>
                    </>
                )
            }
        >
            <div className="cp-shell">
            {stage === 'compose' ? (
                <>
                    <div className="form-grid">
                        <Field label="标题" wide hint="留空会按选中的知识点自动起一个">
                            <input
                                value={meta.title}
                                placeholder="第二章 一元二次方程 · 课后练习"
                                onChange={(event) => setMeta({...meta, title: event.target.value})}
                            />
                        </Field>
                        <Field label="类型">
                            <select
                                value={meta.type}
                                onChange={(event) => setMeta({...meta, type: event.target.value})}
                            >
                                <option value="PRACTICE">练习</option>
                                <option value="EXAM">测验</option>
                            </select>
                        </Field>
                        <Field label="截止时间" hint="留空表示不限时">
                            <input
                                type="datetime-local"
                                value={meta.dueAt}
                                onChange={(event) => setMeta({...meta, dueAt: event.target.value})}
                            />
                        </Field>
                    </div>

                    <section className="cp-block">
                        <header>
                            <strong>考哪些知识点</strong>
                            <span className="muted">
                                {picked.length ? `已选 ${picked.length} 个` : '不选表示整个学科随机'}
                            </span>
                            <div className="cp-block-actions">
                                <button
                                    className="btn ghost small"
                                    onClick={() => setPicked(knowledge.map((item) => item.id))}
                                    disabled={!knowledge.length}
                                >
                                    全选
                                </button>
                                <button
                                    className="btn ghost small"
                                    onClick={() => setPicked([])}
                                    disabled={!picked.length}
                                >
                                    清空
                                </button>
                            </div>
                        </header>
                        {!subjectId ? (
                            <p className="muted cp-hint">这个班还没设学科，先去班级里补上学科，才能按知识点出题。</p>
                        ) : (
                            <>
                                <input
                                    className="teach-search"
                                    placeholder="按知识点名字筛（如「二次函数」）"
                                    value={keyword}
                                    onChange={(event) => setKeyword(event.target.value)}
                                />
                                <div className="cp-kp-list">
                                    {filtered.map((item) => {
                                        const on = picked.includes(item.id);
                                        const count = item.easy + item.middle + item.hard;
                                        return (
                                            <label key={item.id} className={`cp-kp ${on ? 'on' : ''}`}>
                                                <input
                                                    type="checkbox"
                                                    checked={on}
                                                    onChange={() => toggleKnowledge(item.id)}
                                                />
                                                <span className="cp-kp-name">{item.name}</span>
                                                <span className={`cp-kp-count ${count ? '' : 'zero'}`}>
                                                    {count}
                                                </span>
                                            </label>
                                        );
                                    })}
                                    {!filtered.length && (
                                        <p className="muted cp-hint">
                                            {knowledge.length
                                                ? '没搜到，换个词试试'
                                                : '这个学科还没有知识点，先到题库里归类'}
                                        </p>
                                    )}
                                </div>
                            </>
                        )}
                    </section>

                    <section className="cp-block">
                        <header>
                            <strong>每个难度出几题</strong>
                            <span className="muted">共 {planned} 题</span>
                        </header>
                        <div className="cp-bands">
                            {BANDS.map((band) => (
                                <label key={band.key} className="cp-band">
                                    <span className="cp-band-label">
                                        {band.label}
                                        <em>{band.hint}</em>
                                    </span>
                                    <input
                                        type="number"
                                        min="0"
                                        value={counts[band.key]}
                                        onChange={(event) =>
                                            setCounts({...counts, [band.key]: Math.max(0, Number(event.target.value) || 0)})
                                        }
                                    />
                                    <span className="cp-band-avail">
                                        {available ? `可用 ${available[band.key]}` : '—'}
                                    </span>
                                </label>
                            ))}
                        </div>
                        {available && planned > 0 && (
                            <p className="cp-hint muted">
                                {planned > available.easy + available.middle + available.hard
                                    ? '要的比题库里有的多，抽不满会明确告诉你缺几道。'
                                    : '抽题优先用没被别的作业用过的题。'}
                            </p>
                        )}
                    </section>

                    <section className="cp-block">
                        <header>
                            <strong>题型</strong>
                            <span className="muted">不选表示不限</span>
                        </header>
                        <div className="cp-types">
                            {TYPES.map((item) => (
                                <button
                                    key={item.value}
                                    className={`cp-chip ${types.includes(item.value) ? 'on' : ''}`}
                                    onClick={() => toggleType(item.value)}
                                >
                                    {item.label}
                                </button>
                            ))}
                        </div>
                        <div className="cp-scores">
                            <span className="muted">每题分值</span>
                            {TYPES.map((item) => (
                                <label key={item.value} className="cp-score">
                                    <span>{item.label}</span>
                                    <input
                                        type="number"
                                        min="0"
                                        step="0.5"
                                        value={scores[item.value]}
                                        onChange={(event) =>
                                            rememberScores({
                                                ...scores,
                                                [item.value]: Math.max(0, Number(event.target.value) || 0),
                                            })
                                        }
                                    />
                                    <em>分</em>
                                </label>
                            ))}
                        </div>
                    </section>
                </>
            ) : (
                <>
                    <div className="form-grid">
                        <Field label="标题" wide>
                            <input
                                value={meta.title}
                                onChange={(event) => setMeta({...meta, title: event.target.value})}
                            />
                        </Field>
                        <Field label="类型">
                            <select
                                value={meta.type}
                                onChange={(event) => setMeta({...meta, type: event.target.value})}
                            >
                                <option value="PRACTICE">练习</option>
                                <option value="EXAM">测验</option>
                            </select>
                        </Field>
                        <Field label="截止时间" hint="留空表示不限时">
                            <input
                                type="datetime-local"
                                value={meta.dueAt}
                                onChange={(event) => setMeta({...meta, dueAt: event.target.value})}
                            />
                        </Field>
                        <Field label="迟交宽限（小时）">
                            <input
                                type="number"
                                min="0"
                                value={meta.lateHours}
                                onChange={(event) => setMeta({...meta, lateHours: event.target.value})}
                            />
                        </Field>
                        <Field label="说明" wide>
                            <textarea
                                rows={2}
                                value={meta.description}
                                placeholder="做题要求、交作业方式…"
                                onChange={(event) => setMeta({...meta, description: event.target.value})}
                            />
                        </Field>
                    </div>

                    <div className="teach-pick">
                        <div className="teach-pick-head">
                            <strong>
                                {questions.length} 题 · {scoreText(totalOf(questions))} 分
                            </strong>
                            <button className="btn ghost small" onClick={() => setPicker((value) => !value)}>
                                {picker ? '收起手选题' : '手动加题'}
                            </button>
                        </div>
                        {gaps.length > 0 && (
                            <p className="cp-gap">
                                没凑齐：
                                {gaps.map((gap) => `${gap.label} 想要 ${gap.need} 道、只有 ${gap.got} 道`).join('；')}
                                。可以返回改条件，或手选补上。
                            </p>
                        )}
                        <div className="teach-picked">
                            {questions.map((item, index) => (
                                <div key={item.questionId} className="cp-row">
                                    <span className="row-no">{index + 1}</span>
                                    <span className="cp-row-main">
                                        <span className="cp-row-meta">
                                            <Pill tone="line">{item.typeLabel || typeLabel(item.type)}</Pill>
                                            <Pill tone="line">难度 {item.difficulty}</Pill>
                                            {item.knowledge?.length > 0 && (
                                                <span className="muted cp-row-kp">{item.knowledge.join(' · ')}</span>
                                            )}
                                        </span>
                                        <Rich text={item.stem} className="clamp-2"/>
                                    </span>
                                    <input
                                        className="teach-score"
                                        type="number"
                                        min="0"
                                        step="0.5"
                                        value={item.score}
                                        onChange={(event) => update(index, {score: event.target.value})}
                                    />
                                    <div className="cp-row-actions">
                                        <button
                                            className="icon-btn"
                                            title="上移"
                                            disabled={index === 0}
                                            onClick={() => move(index, -1)}
                                        >
                                            ↑
                                        </button>
                                        <button
                                            className="icon-btn"
                                            title="下移"
                                            disabled={index === questions.length - 1}
                                            onClick={() => move(index, 1)}
                                        >
                                            ↓
                                        </button>
                                        <button
                                            className="btn ghost small"
                                            disabled={busy}
                                            onClick={() => swap(index)}
                                        >
                                            换一题
                                        </button>
                                        <button
                                            className="btn ghost small danger-text"
                                            onClick={() =>
                                                setQuestions((current) =>
                                                    current.filter((_, at) => at !== index),
                                                )
                                            }
                                        >
                                            移除
                                        </button>
                                    </div>
                                </div>
                            ))}
                            {!questions.length && <p className="teach-pick-empty">还没有题，返回上一步生成，或手动加。</p>}
                        </div>
                        {picker && (
                            <QuestionPicker
                                selected={questions.map((item) => item.questionId)}
                                onToggle={(question, checked) =>
                                    setQuestions((current) =>
                                        checked
                                            ? [
                                                  ...current,
                                                  {
                                                      questionId: question.id,
                                                      score: scoreOf(scores, question.type),
                                                      stem: question.stem,
                                                      type: question.type,
                                                      typeLabel: typeLabel(question.type),
                                                      difficulty: question.difficulty,
                                                      knowledge: (question.knowledge || []).map((item) => item.name),
                                                      band: bandOf(question.difficulty),
                                                  },
                                              ]
                                            : current.filter((item) => item.questionId !== question.id),
                                    )
                                }
                            />
                        )}
                    </div>
                </>
            )}
            </div>
        </Modal>
    );
}

/** 手选题：按关键词搜题库，勾上就加进预览。 */
function QuestionPicker({selected, onToggle}) {
    const [keyword, setKeyword] = useState('');
    const [items, setItems] = useState([]);
    const [page, setPage] = useState(1);
    const [total, setTotal] = useState(0);
    const [loading, setLoading] = useState(false);

    const search = useCallback(async (word, nextPage) => {
        setLoading(true);
        try {
            const data = await listQuestions({
                keyword: word || null,
                page: nextPage,
                size: 20,
                sort: 'updated_desc',
            });
            setTotal(data.total);
            setItems((current) => (nextPage === 1 ? data.items : [...current, ...data.items]));
            setPage(nextPage);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => {
        const timer = setTimeout(() => search(keyword, 1), 220);
        return () => clearTimeout(timer);
    }, [keyword, search]);

    return (
        <div className="teach-picker">
            <input
                className="teach-search"
                placeholder="搜题干 / 答案 / 标签，补一道题进去"
                value={keyword}
                onChange={(event) => setKeyword(event.target.value)}
            />
            <div className="teach-picker-list">
                {items.map((question) => {
                    const checked = selected.includes(question.id);
                    return (
                        <label key={question.id} className={`teach-picker-row ${checked ? 'on' : ''}`}>
                            <input
                                type="checkbox"
                                checked={checked}
                                onChange={(event) => onToggle(question, event.target.checked)}
                            />
                            <span className="teach-picker-main">
                                <span className="teach-picker-meta">
                                    <Pill tone="line">{typeLabel(question.type)}</Pill>
                                    <Pill tone="line">难度 {question.difficulty}</Pill>
                                    <span className="muted">{question.subjectName}</span>
                                </span>
                                <Rich text={question.stem} className="clamp-2"/>
                            </span>
                        </label>
                    );
                })}
                {!loading && items.length === 0 && (
                    <p className="muted teach-pick-empty">
                        {keyword ? '没搜到题，换个关键词试试' : '题库还是空的，先去录入或导入题目'}
                    </p>
                )}
            </div>
            <div className="teach-picker-foot">
                <span className="muted">
                    已显示 {items.length} / {total} 道
                </span>
                {items.length < total && (
                    <button className="btn ghost small" disabled={loading} onClick={() => search(keyword, page + 1)}>
                        {loading ? '加载中…' : '加载更多'}
                    </button>
                )}
            </div>
        </div>
    );
}
