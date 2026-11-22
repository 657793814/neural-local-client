import {useEffect, useState} from 'react';
import {cancelJob, commitJob, deleteQuestions, listQuestions, startJob, updateJobItem} from '../lib/api.js';
import {DIFFICULTIES, TYPES, cx, isBlocked, isJunkStem} from '../lib/format.js';
import {Rich} from '../lib/Math.jsx';
import {JobItemCard, JobProgress, PickBar, usePollingJob} from '../components/job.jsx';
import {Card, Empty, PageHead, Tabs, confirmDialog, notify} from '../components/ui.jsx';
import {Pager} from '../components/Pager.jsx';
import DiagramTab from './DiagramTab.jsx';

const DIFFICULTY_WORDS = {1: '很容易', 2: '容易', 3: '中档', 4: '较难', 5: '很难'};

export default function Generate({tree, onChanged, onGo, initialSubject = '', initialDifficulty = '', initialTypes = '', initialFocus = ''}) {
    const [tab, setTab] = useState('generate');

    return (
        <div className="page">
            <PageHead
                title="生成"
                subtitle="AI 出题与批量补充都在本机发起；没配模型时先用「录入」"
                help="models"
            />
            <Tabs
                value={tab}
                onChange={setTab}
                items={[
                    {value: 'generate', label: 'AI 出题'},
                    {value: 'diagram', label: 'AI 制图'},
                    {value: 'classify', label: '批量补充'},
                ]}
            />
            {tab === 'generate' ? (
                <GenerateTab
                    tree={tree}
                    initialSubject={initialSubject}
                    initialDifficulty={initialDifficulty}
                    initialTypes={initialTypes}
                    initialFocus={initialFocus}
                    onChanged={onChanged}
                    onGo={onGo}
                />
            ) : tab === 'diagram' ? (
                <DiagramTab tree={tree}/>
            ) : (
                <ClassifyTab tree={tree} onChanged={onChanged}/>
            )}
        </div>
    );
}

function GenerateTab({tree, initialSubject = '', initialDifficulty = '', initialTypes = '', initialFocus = '', onChanged, onGo}) {
    // 从统计页「去出题」进来时带着学科，省得再挑一次；难度 / 题型 / 重点能带也带上。
    const [subjectId, setSubjectId] = useState(initialSubject || tree[0]?.id || '');
    const [types, setTypes] = useState(() => {
        const picked = String(initialTypes || '')
            .split(',')
            .map((code) => code.trim())
            .filter((code) => TYPES.some((type) => type.value === code));
        return new Set(picked.length > 0 ? picked : ['SINGLE', 'FILL']);
    });
    const [difficulty, setDifficulty] = useState(() => {
        const value = Number(initialDifficulty);
        return value >= 1 && value <= 5 ? value : 3;
    });
    const [count, setCount] = useState(5);
    const [withFigures, setWithFigures] = useState(false);
    const [focus, setFocus] = useState(String(initialFocus || ''));
    const [jobId, setJobId] = useState('');
    const [items, setItems] = useState([]);
    const [selection, setSelection] = useState(() => new Set());
    const [committing, setCommitting] = useState(false);

    useEffect(() => {
        if (!subjectId && tree.length) setSubjectId(tree[0].id);
    }, [tree, subjectId]);

    const job = usePollingJob(jobId, (finished) => {
        if (finished.status === 'DONE') {
            const list = finished.result?.items || [];
            setItems(list);
            setSelection(
                new Set(
                    list
                        .map((item, index) => (isBlocked(item) ? null : index))
                        .filter((value) => value !== null),
                ),
            );
            notify.success(`生成 ${list.length} 道，核对后存草稿`);
        } else if (finished.status === 'FAILED') {
            notify.error(finished.error || '生成失败');
        }
    });

    useEffect(() => {
        if (job?.status === 'DONE') setItems(job.result?.items || []);
    }, [job]);

    const start = async () => {
        if (types.size === 0) {
            notify.warning('至少选一种题型');
            return;
        }
        try {
            setSelection(new Set());
            setItems([]);
            setJobId(
                await startJob('generate', {
                    subjectId,
                    types: [...types],
                    difficulty: DIFFICULTY_WORDS[difficulty],
                    count,
                    withFigures,
                    focus: focus || null,
                }),
            );
        } catch (error) {
            notify.error(error.message);
        }
    };

    /** 预览里改一道题：补齐缺项后自动勾上，不用重生成。 */
    const updateItem = async (index, patch) => {
        if (!job) return false;
        try {
            const updated = await updateJobItem(job.id, index, patch);
            setItems((current) => current.map((item, position) => (position === index ? updated : item)));
            const left = updated.issues || [];
            if (left.length > 0) {
                notify.warning(`还差 ${left.length} 项：${left.join('、')}`);
            } else {
                setSelection((current) => {
                    const next = new Set(current);
                    next.add(index);
                    return next;
                });
                notify.success('改好了，已经勾上，可以存草稿');
            }
            return true;
        } catch (error) {
            notify.error(error.message);
            return false;
        }
    };

    const commit = async () => {
        if (!job || selection.size === 0) return;
        setCommitting(true);
        try {
            const result = await commitJob(job.id, [...selection]);
            notify.success(
                `已存入草稿箱 ${result.imported} 道${result.needsWork ? `（其中 ${result.needsWork} 道缺答案或解析，复核时补上）` : ''}${result.skipped ? `，跳过 ${result.skipped} 道` : ''}；复核通过后才是正式入库`,
            );
            (result.problems || []).slice(0, 3).forEach((problem) => notify.warning(problem));
            setJobId('');
            setItems([]);
            setSelection(new Set());
            onChanged?.();
        } catch (error) {
            notify.error(error.message);
        } finally {
            setCommitting(false);
        }
    };

    const ready = items.filter((item) => !isBlocked(item)).length;

    return (
        <div className="generate-grid">
            <Card title="出题条件" subtitle="生成的是草稿；缺答案、解析只算待补，照样能存，复核时再补">
                <div className="form-grid">
                    <label className="field">
                        <span className="field-label">学科</span>
                        <select value={subjectId} onChange={(event) => setSubjectId(event.target.value)}>
                            {tree.map((subject) => (
                                <option key={subject.id} value={subject.id}>
                                    {subject.name}
                                </option>
                            ))}
                        </select>
                    </label>
                    <label className="field">
                        <span className="field-label">数量</span>
                        <input
                            type="number"
                            min={1}
                            max={20}
                            value={count}
                            onChange={(event) => setCount(Math.max(1, Math.min(20, Number(event.target.value) || 1)))}
                        />
                        <span className="field-hint">超过 5 道自动分批，每批独立生成，更稳更快</span>
                    </label>
                    <label className="field">
                        <span className="field-label">难度</span>
                        <select value={difficulty} onChange={(event) => setDifficulty(Number(event.target.value))}>
                            {DIFFICULTIES.map((item) => (
                                <option key={item.value} value={item.value}>
                                    {item.label}
                                </option>
                            ))}
                        </select>
                    </label>
                    <label className="field wide">
                        <span className="field-label">本次重点（可留空）</span>
                        <input
                            value={focus}
                            onChange={(event) => setFocus(event.target.value)}
                            placeholder="例如：二次函数的最值，结合生活情境"
                        />
                    </label>
                    <div className="field wide">
                        <span className="field-label">题型（只出勾选的这几种）</span>
                        <div className="chip-row">
                            {TYPES.map((type) => (
                                <button
                                    key={type.value}
                                    className={cx('chip', types.has(type.value) && 'active')}
                                    onClick={() => {
                                        setTypes((current) => {
                                            const next = new Set(current);
                                            if (next.has(type.value)) next.delete(type.value);
                                            else next.add(type.value);
                                            return next;
                                        });
                                    }}
                                >
                                    {type.label}
                                </button>
                            ))}
                        </div>
                    </div>
                    <label className="field wide check">
                        <input
                            type="checkbox"
                            checked={withFigures}
                            onChange={(event) => setWithFigures(event.target.checked)}
                        />
                        <span>
                            给合适的题配图（几何、函数、示意图等会自动生成图形）
                            <span className="field-hint">
                                勾上后每道配图题会多一次作图调用，整体会慢一些；画不出的题自动降级成纯文字
                            </span>
                        </span>
                    </label>
                </div>
                <div className="row-end">
                    <button className="btn primary" onClick={start} disabled={job?.status === 'RUNNING'}>
                        {job?.status === 'RUNNING' ? '生成中…' : `生成 ${count} 道`}
                    </button>
                </div>
                <JobProgress job={job} onCancel={() => cancelJob(job.id)}/>
            </Card>

            <Card
                title="预览"
                subtitle={items.length > 0 ? `共 ${items.length} 道，可存 ${ready} 道；缺答案的也能先存，复核时补` : undefined}
                actions={
                    items.length > 0 && (
                        <button className="btn primary small" onClick={commit} disabled={committing || selection.size === 0}>
                            {committing ? '保存中…' : `存为草稿 ${selection.size} 道`}
                        </button>
                    )
                }
            >
                {!job && items.length === 0 && (
                    <Empty
                        title="还没有生成"
                        hint={
                            <span>
                                先在右边设置条件；还没有模型的话，
                                <a href="#/settings">去设置里加一个</a>
                            </span>
                        }
                    />
                )}
                {job?.status === 'RUNNING' && <div className="skeleton-list">模型正在出题…</div>}
                <PickBar items={items} selection={selection} onChange={setSelection}/>
                {items.length > 0 && (
                    <div className="job-list">
                        {items.map((item, index) => (
                            <JobItemCard
                                key={index}
                                index={index}
                                item={item}
                                tree={tree}
                                selected={selection.has(index)}
                                kind="generate"
                                onUpdate={updateItem}
                                onToggle={(position, checked) =>
                                    setSelection((current) => {
                                        const next = new Set(current);
                                        if (checked) next.add(position);
                                        else next.delete(position);
                                        return next;
                                    })
                                }
                            />
                        ))}
                    </div>
                )}
            </Card>
        </div>
    );
}

function ClassifyTab({onChanged}) {
    const [rows, setRows] = useState([]);
    const [selected, setSelected] = useState(() => new Set());
    const [jobId, setJobId] = useState('');
    const [committing, setCommitting] = useState(false);
    const [loading, setLoading] = useState(true);
    const [apply, setApply] = useState(() => new Set());
    const [expanded, setExpanded] = useState(() => new Set());
    const [keyword, setKeyword] = useState('');
    const [page, setPage] = useState(1);
    const [total, setTotal] = useState(0);
    const [pageSize, setPageSize] = useState(20);
    const [showJunk, setShowJunk] = useState(false);

    const load = async () => {
        setLoading(true);
        try {
            // 只处理内容还缺的草稿：已经补齐的题不再出现在这里。
            const query = {needsReview: true, uncategorized: true, page, size: pageSize};
            if (keyword.trim()) query.keyword = keyword.trim();
            const result = await listQuestions(query);
            setRows(result.items || []);
            setTotal(result.total || 0);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setLoading(false);
        }
    };

    useEffect(() => {
        load();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [page, keyword]);

    const toggleExpand = (id) =>
        setExpanded((current) => {
            const next = new Set(current);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            return next;
        });

    const toggleSelect = (id) =>
        setSelected((current) => {
            const next = new Set(current);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            return next;
        });

    const totalPages = Math.max(1, Math.ceil(total / pageSize));

    const job = usePollingJob(jobId, (finished) => {
        if (finished.status === 'DONE') {
            const items = finished.result?.items || [];
            setApply(new Set(items.map((item, index) => (item.error ? null : index)).filter((value) => value !== null)));
            notify.success('补充完成，确认后写入');
        } else if (finished.status === 'FAILED') {
            notify.error(finished.error || '补充失败');
        }
    });

    const start = async () => {
        if (cleanSelected.length === 0) return;
        try {
            setApply(new Set());
            setJobId(await startJob('classify', {questionIds: cleanSelected}));
        } catch (error) {
            notify.error(error.message);
        }
    };

    const commit = async () => {
        if (!job || apply.size === 0) return;
        setCommitting(true);
        try {
            const result = await commitJob(job.id, [...apply]);
            notify.success(`已更新 ${result.imported} 道题`);
            setJobId('');
            setSelected(new Set());
            setApply(new Set());
            await load();
            onChanged?.();
        } catch (error) {
            notify.error(error.message);
        } finally {
            setCommitting(false);
        }
    };

    const junkRows = rows.filter((row) => isJunkStem(row.stem));
    const junkIds = junkRows.map((row) => row.id);
    const cleanSelected = [...selected].filter((id) => !junkIds.includes(id));

    const removeJunk = async (ids) => {
        if (!ids.length) return;
        const ok = await confirmDialog({
            title: '移到回收站',
            message: `确认把这 ${ids.length} 条误录的网页文字移到回收站吗？这些是截图录题时混进来的网页按钮，不是真实题目。可在回收站恢复。`,
            danger: true,
            confirmText: '移到回收站',
        });
        if (!ok) return;
        try {
            await deleteQuestions(ids);
            notify.success(`已移走 ${ids.length} 条，可在回收站恢复`);
            setSelected((current) => {
                const next = new Set(current);
                ids.forEach((id) => next.delete(id));
                return next;
            });
            setPage(1);
            await load();
            onChanged?.();
        } catch (error) {
            notify.error(error.message);
        }
    };

    const items = job?.result?.items || [];

    return (
        <div className="generate-grid">
            <Card
                title="待补充的题"
                subtitle="每题标出了缺什么；勾选后交给 AI 补齐知识点、答案、解析、难度。内容齐全的题不再出现"
                actions={
                    <button className="btn primary small" onClick={start} disabled={cleanSelected.length === 0 || job?.status === 'RUNNING'}>
                        补充 {cleanSelected.length} 道
                    </button>
                }
            >
                <div className="classify-toolbar">
                    <input
                        className="classify-search"
                        value={keyword}
                        onChange={(event) => {
                            setKeyword(event.target.value);
                            setPage(1);
                        }}
                        placeholder="搜题干关键词…"
                    />
                    <div className="chip-row">
                        <button
                            className="btn ghost small"
                            onClick={() => setSelected(new Set(rows.filter((r) => !isJunkStem(r.stem)).map((row) => row.id)))}
                            disabled={rows.length === 0}
                        >
                            本页全选
                        </button>
                        <button className="btn ghost small" onClick={() => setSelected(new Set())} disabled={selected.size === 0}>
                            清空
                        </button>
                        <span className="muted small">已选 {cleanSelected.length} 道</span>
                    </div>
                </div>

                {loading ? (
                    <div className="loading">加载中…</div>
                ) : rows.length === 0 ? (
                    <Empty title="没有待补充的题" hint="草稿箱里内容已经补全的题不会再出现在这里"/>
                ) : (
                    <>
                        <div className="pick-list classify-pick-list">
                            {rows.filter((row) => !isJunkStem(row.stem)).map((row) => {
                                const isOpen = expanded.has(row.id);
                                const issues = row.issues || [];
                                return (
                                    <div className={cx('pick-wrap', selected.has(row.id) && 'active')} key={row.id}>
                                        <div className="pick" onClick={() => toggleExpand(row.id)}>
                                            <input
                                                type="checkbox"
                                                checked={selected.has(row.id)}
                                                onClick={(event) => event.stopPropagation()}
                                                onChange={() => toggleSelect(row.id)}
                                            />
                                            <span className="pick-stem">
                                                <span className="pick-issues">
                                                    {issues.length > 0 ? (
                                                        issues.map((issue) => (
                                                            <span className="issue-chip" key={issue}>缺{issue.replace(/^缺/, '')}</span>
                                                        ))
                                                    ) : (
                                                        <span className="issue-chip ok">内容齐全</span>
                                                    )}
                                                </span>
                                                <Rich text={row.stem}/>
                                            </span>
                                            <span className="pick-more">{isOpen ? '收起' : '详情'}</span>
                                        </div>
                                        {isOpen && (
                                            <div className="pick-detail">
                                                {row.options?.length > 0 && (
                                                    <div className="pick-detail-row">
                                                        <span className="pick-detail-label">选项</span>
                                                        <ul className="option-view compact">
                                                            {row.options.map((option) => (
                                                                <li key={option.key}>
                                                                    <span className="option-key">{option.key}</span>
                                                                    <Rich text={option.text}/>
                                                                </li>
                                                            ))}
                                                        </ul>
                                                    </div>
                                                )}
                                                <div className="pick-detail-row">
                                                    <span className="pick-detail-label">答案</span>
                                                    <div>{row.answer ? <Rich text={row.answer}/> : <span className="missing">缺失</span>}</div>
                                                </div>
                                                <div className="pick-detail-row">
                                                    <span className="pick-detail-label">解析</span>
                                                    <div>{row.analysis ? <Rich text={row.analysis}/> : <span className="missing">缺失</span>}</div>
                                                </div>
                                                <div className="pick-detail-row">
                                                    <span className="pick-detail-label">知识点</span>
                                                    <div>{row.knowledge?.length > 0 ? row.knowledge.map((k) => k.name).join('、') : <span className="missing">缺失</span>}</div>
                                                </div>
                                                <div className="pick-detail-row">
                                                    <span className="pick-detail-label">题型/难度</span>
                                                    <div className="muted small">{row.typeLabel}{row.difficulty ? ` · 难度 ${row.difficulty}` : ' · 难度未定'}</div>
                                                </div>
                                            </div>
                                        )}
                                    </div>
                                );
                            })}
                        </div>

                        {junkRows.length > 0 && (
                            <div className="junk-fold">
                                <button className="junk-fold-head" onClick={() => setShowJunk((value) => !value)}>
                                    <span className="junk-fold-arrow">{showJunk ? '▾' : '▸'}</span>
                                    另发现 {junkRows.length} 条疑似误录的网页文字（不是题目）
                                    <span className="junk-fold-action">核对后移走</span>
                                </button>
                                {showJunk && (
                                    <div className="junk-fold-body">
                                        <p className="muted small">这些是截图录题时混进来的网页按钮文字。确认后移到回收站，可恢复。</p>
                                        <button className="btn danger small" onClick={() => removeJunk(junkIds)}>
                                            全部移走 {junkRows.length} 条
                                        </button>
                                        <div className="junk-list">
                                            {junkRows.map((row) => (
                                                <div className="junk-entry" key={row.id}>
                                                    <code>{row.stem.replace(/\s+/g, ' ').trim().slice(0, 100)}</code>
                                                    <button className="link-btn" onClick={() => removeJunk([row.id])}>移走</button>
                                                </div>
                                            ))}
                                        </div>
                                    </div>
                                )}
                            </div>
                        )}
                    </>
                )}
                <Pager
                    page={page}
                    pageCount={totalPages}
                    total={total}
                    size={pageSize}
                    onPage={(value) => { setPage(value); setSelected(new Set()); }}
                    onSize={(value) => { setPageSize(value); setPage(1); setSelected(new Set()); }}
                />
                <JobProgress job={job} onCancel={() => cancelJob(job.id)}/>
            </Card>

            <Card
                title="补充结果"
                subtitle={items.length > 0 ? `${items.length} 道，确认后写入` : undefined}
                actions={
                    items.length > 0 && (
                        <button className="btn primary small" onClick={commit} disabled={committing || apply.size === 0}>
                            {committing ? '写入中…' : `写入 ${apply.size} 道`}
                        </button>
                    )
                }
            >
                {items.length === 0 ? (
                    <Empty title="还没有结果" hint="勾左边的题，点「补充」"/>
                ) : (
                    <div className="job-list">
                        {items.map((item, index) => (
                            <JobItemCard
                                key={index}
                                index={index}
                                item={item}
                                kind="classify"
                                selected={apply.has(index)}
                                onToggle={(position, checked) =>
                                    setApply((current) => {
                                        const next = new Set(current);
                                        if (checked) next.add(position);
                                        else next.delete(position);
                                        return next;
                                    })
                                }
                            />
                        ))}
                    </div>
                )}
            </Card>
        </div>
    );
}
