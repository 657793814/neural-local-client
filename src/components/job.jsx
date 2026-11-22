import {useEffect, useRef, useState} from 'react';
import {getJob, imageFromPath} from '../lib/api.js';
import {Rich} from '../lib/Math.jsx';
import {CopyableError, Pill, Progress} from './ui.jsx';
import {cx, isBlocked, typeLabel} from '../lib/format.js';
import {DraftEditor} from './DraftEditor.jsx';
import {DiagramDialog} from './DiagramDialog.jsx';
import {FigureCropDialog} from './FigureCropDialog.jsx';

/** 给题干正文挑一个不重复的「图N」引用名。 */
function nextFigureRef(item) {
    const parts = [item?.stem, item?.analysis, ...(item?.options || []).map((option) => option.text)];
    const used = new Set();
    for (const part of parts) {
        for (const match of String(part || '').matchAll(/!\[[^\]]*\]\((图\d+)\)/g)) used.add(match[1]);
    }
    let n = 1;
    while (used.has(`图${n}`)) n += 1;
    return `图${n}`;
}

// 识别时裁出来的图形：同一张预览图只读一次盘。
const figureCache = new Map();

/** 把任务结果里的 figures（ref + path）读成 dataUrl，供 Rich 在预览里直接画出来。 */
function useFigureUrls(figures) {
    const key = (figures || []).map((figure) => `${figure.ref}:${figure.path}`).join('|');
    const [urls, setUrls] = useState({});
    useEffect(() => {
        if (!key) {
            setUrls({});
            return undefined;
        }
        let alive = true;
        (async () => {
            const next = {};
            for (const figure of figures || []) {
                if (!figure.path) continue;
                let url = figureCache.get(figure.path);
                if (url === undefined) {
                    try {
                        url = (await imageFromPath(figure.path)).dataUrl;
                    } catch {
                        url = '';
                    }
                    figureCache.set(figure.path, url);
                }
                if (url) next[figure.ref] = url;
            }
            if (alive) setUrls(next);
        })();
        return () => {
            alive = false;
        };
    }, [key]);
    return urls;
}

/**
 * 结果列表顶部的批量勾选条。
 * 以前只有一个「全选可存」藏在卡头，缺答案的题又都是灰的，等于没法批量；
 * 现在勾选不看缺项，只把「没题干」的排除在外。
 */
export function PickBar({items, selection, onChange}) {
    if (!items.length) return null;
    const savable = items
        .map((item, index) => (isBlocked(item) ? null : index))
        .filter((value) => value !== null);
    const blockedCount = items.length - savable.length;

    return (
        <div className="pick-bar">
            <span className="muted small">
                已选 {selection.size} / {items.length} 道
            </span>
            <button className="link-btn" onClick={() => onChange(new Set(savable))}>
                全选
            </button>
            <button className="link-btn" onClick={() => onChange(new Set())} disabled={selection.size === 0}>
                清空
            </button>
            {blockedCount > 0 && (
                <span className="muted small">{blockedCount} 道没认出题干，勾不上</span>
            )}
        </div>
    );
}

/** 轮询任务直到结束。任务 30–60 秒起步，用轮询比 SSE 更稳，代理也不会掐断。 */
export function usePollingJob(jobId, onDone) {
    const [job, setJob] = useState(null);
    const doneRef = useRef(onDone);
    doneRef.current = onDone;

    useEffect(() => {
        if (!jobId) {
            setJob(null);
            return undefined;
        }
        let alive = true;
        let timer = null;
        const tick = async () => {
            try {
                const next = await getJob(jobId);
                if (!alive) return;
                setJob(next);
                if (next.status === 'RUNNING') {
                    timer = setTimeout(tick, 1200);
                } else {
                    doneRef.current?.(next);
                }
            } catch (error) {
                if (!alive) return;
                setJob((current) => ({...(current || {}), status: 'FAILED', error: error.message}));
            }
        };
        tick();
        return () => {
            alive = false;
            if (timer) clearTimeout(timer);
        };
    }, [jobId]);

    return job;
}

export function JobProgress({job, onCancel}) {
    if (!job) return null;
    const running = job.status === 'RUNNING';
    return (
        <div className="job-progress">
            <div className="job-progress-head">
                <span>
                    {running
                        ? job.progress < 25
                            ? '正在读题…'
                            : job.progress < 95
                                ? '正在整理题目，通常几秒到一分钟'
                                : '马上就好'
                        : job.status === 'DONE'
                            ? '整理完成，请逐题核对'
                            : job.status === 'FAILED'
                                ? '这次没成功'
                                : '已停止'}
                </span>
                <span className="muted">{job.progress}%</span>
            </div>
            <Progress value={job.progress}/>
            {job.status === 'FAILED' && <CopyableError text={job.error}/>} 
            {running && onCancel && (
                <button className="btn ghost small" onClick={onCancel}>
                    停止
                </button>
            )}
        </div>
    );
}

/**
 * 任务结果里的一道题：先给人看，改好、勾选后才存成草稿。
 * 缺答案、缺解析的题**不再拦人**——草稿箱本来就是放半成品的地方，
 * 只有连题干都没认出来（blocking）才勾不上。
 */
export function JobItemCard({item, index, selected, onToggle, kind, tree, onUpdate}) {
    const issues = item.issues || [];
    const warnings = item.warnings || [];
    const blocking = item.blocking || [];
    const blocked = blocking.length > 0 || !!item.error;
    const [editing, setEditing] = useState(false);
    const [saving, setSaving] = useState(false);
    const [diagramOpen, setDiagramOpen] = useState(false);
    // undefined = 关着；null = 补一张新图；对象 = 改这张图的框
    const [tuning, setTuning] = useState(undefined);
    const figures = useFigureUrls(item.figures);

    const save = async (patch) => {
        if (!onUpdate) return;
        setSaving(true);
        try {
            const ok = await onUpdate(index, patch);
            if (ok) setEditing(false);
        } finally {
            setSaving(false);
        }
    };

    /** 一键制图：生成后把引用插进题干，figure 交给后端合并进 item。 */
    const saveDiagram = async (svg, dataUrl, path, refName) => {
        if (!onUpdate) return;
        setSaving(true);
        try {
            await onUpdate(index, {
                stem: (item.stem || '').trimEnd() + `\n\n![](${refName})`,
                figures: [{ref: refName, path}],
            });
        } finally {
            setSaving(false);
        }
    };

    /**
     * 图框校对确认：重新裁出来的图按同一个引用换掉旧的。
     * 补新图时顺手把 `![](图N)` 追加到题干末尾，不然图没有引用，存草稿时会被丢掉。
     */
    const applyCrop = async (path, box) => {
        if (!onUpdate) return;
        setSaving(true);
        try {
            if (tuning) {
                await onUpdate(index, {figures: [{ref: tuning.ref, path, box}]});
            } else {
                const refName = nextFigureRef(item);
                await onUpdate(index, {
                    stem: `${(item.stem || '').trimEnd()}\n\n![](${refName})`,
                    figures: [{ref: refName, path, box}],
                });
            }
        } finally {
            setSaving(false);
        }
    };

    return (
        <>
        <article className={cx('job-item', selected && 'selected', blocked && 'blocked')}>
            <label className="job-item-check">
                <input
                    type="checkbox"
                    checked={!!selected}
                    disabled={blocked}
                    onChange={(event) => onToggle(index, event.target.checked)}
                />
            </label>
            <div className="job-item-body">
                <div className="job-item-head">
                    <Pill tone="line">{typeLabel(item.type)}</Pill>
                    <Pill tone="line">难度 {item.difficulty ?? 3}</Pill>
                    {(item.knowledgeNames || []).map((name) => (
                        <span className="tag kp" key={name}>
                            {name}
                        </span>
                    ))}
                    {item.confidence != null && (
                        <span className="muted small">置信度 {Math.round(Number(item.confidence) * 100)}%</span>
                    )}
                    {kind !== 'classify' && onUpdate && !editing && (
                        <>
                            <button className="link-btn job-item-edit job-item-draw" onClick={() => setDiagramOpen(true)}>
                                AI 制图
                            </button>
                            <button className="link-btn job-item-edit" onClick={() => setEditing(true)}>
                                编辑
                            </button>
                        </>
                    )}
                </div>

                {editing ? (
                    <DraftEditor
                        item={item}
                        tree={tree}
                        saving={saving}
                        onCancel={() => setEditing(false)}
                        onSave={save}
                    />
                ) : (
                    <>
                        {kind === 'classify' ? (
                            <>
                                <div className="job-item-stem">
                                    <Rich text={item.stem || item.stemExcerpt} className="inline"/>
                                </div>
                                <div className="job-item-side">
                                    <span>学科：{item.subjectName || '未改'}</span>
                                    <span>难度：{item.difficulty ?? '未改'}</span>
                                    <span>知识点：{(item.knowledgeNames || []).join('、') || '未改'}</span>
                                </div>
                            </>
                        ) : (
                            <div className="job-item-stem">
                                <Rich text={item.stem} figures={figures}/>
                            </div>
                        )}
                        {/* 截图识别出来的图：框歪了能自己拖，漏了能补一张。 */}
                        {!editing && onUpdate && item.capturePath && kind !== 'classify' && (
                            <div className="figure-tune">
                                <span className="muted small">图框不对？</span>
                                {(item.figures || []).map((figure) => (
                                    <button
                                        type="button"
                                        key={figure.ref}
                                        className="link-btn"
                                        onClick={() => setTuning(figure)}
                                    >
                                        {figure.ref} 调整框选
                                    </button>
                                ))}
                                <button type="button" className="link-btn" onClick={() => setTuning(null)}>
                                    ＋ 从截图补一张图
                                </button>
                            </div>
                        )}
                        {item.options?.length > 0 && (
                            <ul className="option-view compact">
                                {item.options.map((option) => (
                                    <li key={option.key}>
                                        <span className="option-key">{option.key}</span>
                                        <Rich text={option.text} className="inline" figures={figures}/>
                                    </li>
                                ))}
                            </ul>
                        )}
                        {item.answer && (
                            <div className="job-item-answer">
                                <strong>答案</strong>
                                <Rich text={item.answer} className="inline" figures={figures}/>
                            </div>
                        )}
                        {item.analysis && (
                            <details className="job-item-analysis">
                                <summary>解析</summary>
                                <Rich text={item.analysis} figures={figures}/>
                            </details>
                        )}
                        {item.note && <p className="muted small">{item.note}</p>}
                        {blocking.length > 0 && (
                            <div className="job-item-issues">
                                {blocking.map((issue) => (
                                    <Pill tone="danger" key={issue}>
                                        {issue}
                                    </Pill>
                                ))}
                                <span className="muted small">
                                    这道勾不上：点「编辑」补上题干，或者直接丢掉
                                </span>
                            </div>
                        )}
                        {/* 已经被拦住（没题干）的题不再提「可以存」——同一张卡片上两句话会打架 */}
                        {issues.length > 0 && blocking.length === 0 && (
                            <div className="job-item-todo">
                                {issues.map((issue) => (
                                    <Pill tone="warn" key={issue}>
                                        {issue}
                                    </Pill>
                                ))}
                                <span className="muted small">
                                    可以先存进草稿箱，复核时补上
                                </span>
                            </div>
                        )}
                        {warnings.length > 0 && (
                            <div className="job-item-warnings">
                                {warnings.map((warning) => (
                                    <span key={warning} className="muted small">
                                        · {warning}
                                    </span>
                                ))}
                            </div>
                        )}
                        {item.error && <CopyableError text={item.error}/>} 
                    </>
                )}
            </div>

            {diagramOpen && (
                <DiagramDialog
                    stem={item.stem || ''}
                    subjectId={item.subjectId || ''}
                    refName={nextFigureRef(item)}
                    onInsert={saveDiagram}
                    onClose={() => setDiagramOpen(false)}
                />
            )}

            {tuning !== undefined && (
                <FigureCropDialog
                    sourcePath={item.capturePath}
                    box={tuning?.box}
                    auto={tuning?.box}
                    refName={tuning?.ref || ''}
                    onApply={applyCrop}
                    onClose={() => setTuning(undefined)}
                />
            )}
        </article>
        </>
    );
}
