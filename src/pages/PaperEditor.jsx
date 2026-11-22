import {useCallback, useEffect, useMemo, useRef, useState} from 'react';
import {
    paperAddQuestions,
    paperFinalize,
    paperGet,
    paperPrintLog,
    paperRefreshSnapshot,
    paperRemoveQuestion,
    paperReopen,
    paperReorderQuestions,
    paperReplaceQuestion,
    paperSectionDelete,
    paperSectionReorder,
    paperSectionSave,
    paperSetScores,
    paperSetTotal,
    paperUndoReplace,
    paperUpdate,
} from '../lib/api.js';
import {RichInline} from '../lib/Math.jsx';
import {SheetPreview} from '../components/paper/AnswerSheetDesigner.jsx';
import {renderPaperHtml} from '../lib/paper/render.js';
import {exportHtml, openInBrowser} from '../lib/paper/print.js';
import {
    EXAM_TYPES,
    PAPER_STATUS,
    contentBox,
    fullSectionTitle,
    mergeLayout,
    paperSize,
} from '../lib/paper/model.js';
import {deltaText, scoreText, scoreSummary} from '../lib/paper/score.js';
import {
    BUILTIN_ANSWER_SHEETS,
    answerSheetOf,
    loadAnswerSheets,
} from '../lib/paper/answerSheet.js';
import {subjectList} from '../lib/format.js';
import {
    Empty,
    Field,
    Loading,
    Modal,
    Pill,
    confirmDialog,
    notify,
} from '../components/ui.jsx';
import {QuestionPicker} from '../components/paper/QuestionPicker.jsx';
import {ReplaceDrawer} from '../components/paper/ReplaceDrawer.jsx';
import {AnswerSheetDesigner} from '../components/paper/AnswerSheetDesigner.jsx';
import {BlueprintModal} from '../components/paper/BlueprintModal.jsx';
import PaperPreview, {
    MM_TO_PX,
    PrintPreviewFrame,
    previewPaperHeight,
} from '../components/paper/PaperPreview.jsx';

const EMPTY_SECTION = {
    title: '',
    kind: 'SINGLE',
    scoreMode: 'UNIFORM',
    unitScore: 5,
};

function excerptOf(snapshot) {
    const stem = String(snapshot?.stem || '')
        .replace(/!\[[^\]]*\]\([^)]*\)/g, '')
        .replace(/\s+/g, ' ')
        .trim();
    return stem.length > 46 ? `${stem.slice(0, 46)}…` : stem || '（无题干）';
}

export default function PaperEditor({paperId, initialPreview = false, tree = [], onGo, onChanged}) {
    const [doc, setDoc] = useState(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState('');
    const [variant, setVariant] = useState('STUDENT');
    const [html, setHtml] = useState('');
    const [printHtml, setPrintHtml] = useState('');
    const [pageCount, setPageCount] = useState(0);
    const [warnings, setWarnings] = useState([]);
    const [rendering, setRendering] = useState(false);
    const [previewOpen, setPreviewOpen] = useState(initialPreview);
    const [picker, setPicker] = useState(null); // 目标大题 id
    const [replaceTarget, setReplaceTarget] = useState(null); // {sectionId, ord, sectionLabel}
    const [blueprintOpen, setBlueprintOpen] = useState(false);
    const [sectionForm, setSectionForm] = useState(null);
    const [settingsOpen, setSettingsOpen] = useState(false);
    const [totalOpen, setTotalOpen] = useState(false);
    const [answerSheets, setAnswerSheets] = useState(BUILTIN_ANSWER_SHEETS);
    const [sheetDesignerOpen, setSheetDesignerOpen] = useState(false);
    const [finalizeError, setFinalizeError] = useState('');
    const [busy, setBusy] = useState(false);
    const [scores, setScores] = useState({});
    const [centerScale, setCenterScale] = useState(0.6);
    const centerRef = useRef(null);
    const layoutTimer = useRef(null);

    const subjects = useMemo(() => subjectList(tree), [tree]);
    const summary = useMemo(() => scoreSummary(doc), [doc]);
    const layout = useMemo(() => mergeLayout(doc?.layout), [doc?.layout]);
    const editable = doc?.status === 'DRAFT';
    // 横向卷面比纵向宽很多：默认收起右侧版面栏，把宽度让给画布。
    const landscape = useMemo(() => {
        const size = paperSize(layout);
        return size.width > size.height;
    }, [layout]);
    const [leftOpen, setLeftOpen] = useState(true);
    const [rightOpen, setRightOpen] = useState(() => !landscape);
    const landscapeRef = useRef(landscape);

    useEffect(() => {
        if (landscapeRef.current === landscape) return;
        landscapeRef.current = landscape;
        setRightOpen(!landscape);
    }, [landscape]);

    // 切换方向时联动分栏：横向卷面用双栏铺满，纵向回到单栏；
    // 用户之后仍可在「版面 → 分栏」里手动改。
    const setOrientation = (orientation) => {
        const toLandscape = orientation === 'landscape';
        patchLayout({orientation, columns: toLandscape ? 2 : 1});
    };

    const answerSheetTemplate = useMemo(
        () => answerSheetOf(answerSheets, layout.answerSheet?.templateId),
        [answerSheets, layout.answerSheet?.templateId],
    );

    // 答题卡预览用的真实题号/分值/题型（与设计器共用一份）。
    const sheetEntries = useMemo(() => {
        const list = (doc?.sections || []).flatMap((section) =>
            (section.questions || []).map((item) => {
                const raw = String(item.snapshot?.type || '').toUpperCase();
                return {
                    score: Number(item.score) || 0,
                    type: raw === 'CALC' ? 'SOLVED' : raw,
                };
            }),
        );
        return list.map((entry, index) => ({...entry, number: index + 1}));
    }, [doc]);

    useEffect(() => {
        loadAnswerSheets().then(setAnswerSheets).catch(() => {});
    }, []);

    const load = useCallback(async () => {
        try {
            const data = await paperGet(paperId);
            setDoc(data);
            setError('');
        } catch (err) {
            setError(err.message);
        } finally {
            setLoading(false);
        }
    }, [paperId]);

    useEffect(() => {
        load();
    }, [load]);

    // 预览：写一次 doc 就要重排一次，防抖 350ms，避免调分时每敲一个字都测量。
    useEffect(() => {
        if (!doc) return undefined;
        let alive = true;
        setRendering(true);
        const timer = setTimeout(async () => {
            try {
                const result = await renderPaperHtml(doc, variant, {
                    previewMode: true,
                    answerSheet: answerSheetTemplate?.config,
                });
                if (!alive) return;
                setHtml(result.html);
                setPageCount(result.pages);
                setWarnings(result.warnings);
                setPrintHtml(result.printHtml);
            } catch (err) {
                if (alive) setWarnings([`排版失败：${err.message}`]);
            } finally {
                if (alive) setRendering(false);
            }
        }, 350);
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [doc, variant, answerSheetTemplate]);

    // 中央预览按容器宽度适配，A4 在窄栏里也能看到整页
    useEffect(() => {
        const element = centerRef.current;
        if (!element) return undefined;
        const compute = () => {
            const box = contentBox(layout);
            const width = (box.pageWidth || 210) * MM_TO_PX;
            setCenterScale(Math.min(1, Math.max(0.3, (element.clientWidth - 48) / width)));
        };
        compute();
        const observer = new ResizeObserver(compute);
        observer.observe(element);
        return () => observer.disconnect();
    }, [layout]);

    const apply = (next) => {
        setDoc(next);
        onChanged?.();
    };

    const run = async (work, success) => {
        try {
            setBusy(true);
            const next = await work();
            if (next) apply(next);
            if (success) notify.success(success);
        } catch (err) {
            notify.error(err.message);
        } finally {
            setBusy(false);
        }
    };

    const patchLayout = (patch) => {
        const nextLayout = {...layout, ...patch, margins: {...layout.margins, ...(patch.margins || {})}, answerSheet: {...layout.answerSheet, ...(patch.answerSheet || {})}};
        setDoc((current) => (current ? {...current, layout: nextLayout} : current));
        clearTimeout(layoutTimer.current);
        layoutTimer.current = setTimeout(() => {
            paperUpdate(paperId, {layout: nextLayout})
                .then((next) => {
                    setDoc(next);
                    onChanged?.();
                })
                .catch((err) => notify.error(err.message));
        }, 600);
    };

    const commitScore = async (section, ord, raw) => {
        const value = Number(raw);
        if (!Number.isFinite(value) || value < 0) {
            setScores((current) => {
                const next = {...current};
                delete next[`${section.id}:${ord}`];
                return next;
            });
            return;
        }
        setScores((current) => {
            const next = {...current};
            delete next[`${section.id}:${ord}`];
            return next;
        });
        await run(() => paperSetScores(paperId, [{sectionId: section.id, ord, score: value}]));
    };

    const changeSectionScore = (section, unitScore) => {
        const next = {
            ...doc,
            sections: doc.sections.map((item) =>
                item.id === section.id
                    ? {
                          ...item,
                          scoreMode: 'UNIFORM',
                          unitScore: Number(unitScore),
                          questions: item.questions.map((question) => ({
                              ...question,
                              score: Number(unitScore),
                          })),
                      }
                    : item,
            ),
        };
        setDoc(next);
    };

    const saveSection = async (section, patch) => {
        await run(() =>
            paperSectionSave(paperId, {
                id: section.id,
                title: section.title,
                note: '',
                kind: section.kind,
                scoreMode: section.scoreMode,
                unitScore: section.unitScore,
                ...patch,
            }),
        );
    };

    const moveSection = (index, direction) => {
        const sections = [...doc.sections];
        const target = index + direction;
        if (target < 0 || target >= sections.length) return;
        [sections[index], sections[target]] = [sections[target], sections[index]];
        run(() => paperSectionReorder(paperId, sections.map((item) => item.id)), '大题顺序已调整');
    };

    const moveQuestion = (section, ord, direction) => {
        const ords = section.questions.map((item) => item.ord);
        const index = ords.indexOf(ord);
        const target = index + direction;
        if (index < 0 || target < 0 || target >= ords.length) return;
        [ords[index], ords[target]] = [ords[target], ords[index]];
        run(() => paperReorderQuestions(paperId, section.id, ords));
    };

    const removeQuestion = (section, ord) =>
        run(() => paperRemoveQuestion(paperId, section.id, ord), '已移出本卷');

    const applyReplace = async (questionId) => {
        if (!replaceTarget) return;
        const target = replaceTarget;
        setReplaceTarget(null);
        await run(
            () => paperReplaceQuestion(paperId, target.sectionId, target.ord, questionId),
            '已换题，行尾的 ↩ 可以撤销',
        );
    };

    const undoReplace = (section, ord) =>
        run(() => paperUndoReplace(paperId, section.id, ord), '已换回上一题');

    const addSection = async () => {
        if (!sectionForm.title.trim()) {
            notify.warning('给大题起个名字');
            return;
        }
        await run(
            () =>
                paperSectionSave(paperId, {
                    title: sectionForm.title.trim(),
                    note: '',
                    kind: sectionForm.kind,
                    scoreMode: sectionForm.scoreMode,
                    unitScore: Number(sectionForm.unitScore) || 0,
                }),
            '已加一个大题',
        );
        setSectionForm(null);
    };

    const finalize = async () => {
        try {
            const next = await paperFinalize(paperId);
            apply(next);
            notify.success('已定稿，可以打印了');
        } catch (err) {
            setFinalizeError(err.message);
        }
    };

    const print = async () => {
        try {
            const name = `${doc.title}-${variant === 'TEACHER' ? '教师版' : '学生版'}`;
            await openInBrowser(printHtml, name);
            paperPrintLog(paperId, variant, 1, pageCount, 'BROWSER').catch(() => {});
            notify.success('已交给系统浏览器，按 ⌘P 打印或另存为 PDF');
        } catch (err) {
            notify.error(err.message);
        }
    };

    const exportFile = async () => {
        try {
            const path = await exportHtml(printHtml, `${doc.title}-${variant === 'TEACHER' ? '教师版' : '学生版'}`);
            if (path) notify.success('已导出 HTML');
        } catch (err) {
            notify.error(err.message);
        }
    };

    if (loading) return <Loading text="正在打开试卷…"/>;
    if (error) {
        return (
            <div className="page">
                <Empty title="打不开这份试卷" hint={error} action={<button className="btn" onClick={() => onGo?.('papers')}>回试卷列表</button>}/>
            </div>
        );
    }
    if (!doc) return null;

    const usedIds = new Set(
        doc.sections.flatMap((section) => section.questions.map((item) => item.questionId).filter(Boolean)),
    );
    let numberCursor = 0;
    const status = PAPER_STATUS[doc.status] || PAPER_STATUS.DRAFT;

    return (
        <div className="paper-editor">
            <header className="pe-head">
                <div className="pe-head-left">
                    <button className="btn ghost small" onClick={() => onGo?.('papers')}>
                        ← 试卷
                    </button>
                    <button className="pe-title" onClick={() => setSettingsOpen(true)} title="改卷名 / 学科 / 时长">
                        {doc.title}
                    </button>
                    <Pill tone={status.tone}>{status.label}</Pill>
                    <span className="pe-meta">
                        {doc.subjectName || '未设学科'}
                        {doc.grade ? ` · ${doc.grade}` : ''}
                        {doc.durationMin ? ` · ${doc.durationMin} 分钟` : ''}
                    </span>
                    <div className="seg pe-orient" role="group" aria-label="纸张方向">
                        <button
                            className={layout.orientation !== 'landscape' ? 'active' : ''}
                            aria-pressed={layout.orientation !== 'landscape'}
                            disabled={!editable}
                            title="纵向卷面"
                            onClick={() => setOrientation('portrait')}
                        >
                            ▯ 纵向
                        </button>
                        <button
                            className={layout.orientation === 'landscape' ? 'active' : ''}
                            aria-pressed={layout.orientation === 'landscape'}
                            disabled={!editable}
                            title="横向卷面（排版更宽，侧栏自动让位）"
                            onClick={() => setOrientation('landscape')}
                        >
                            ▭ 横向
                        </button>
                    </div>
                    <div className="seg pe-panes" role="group" aria-label="显示或隐藏侧栏">
                        <button
                            className={leftOpen ? 'active' : ''}
                            aria-pressed={leftOpen}
                            title="显示 / 隐藏题目栏"
                            onClick={() => setLeftOpen((open) => !open)}
                        >
                            题目
                        </button>
                        <button
                            className={rightOpen ? 'active' : ''}
                            aria-pressed={rightOpen}
                            title="显示 / 隐藏版面栏"
                            onClick={() => setRightOpen((open) => !open)}
                        >
                            版面
                        </button>
                    </div>
                </div>
                <div className="pe-head-right">
                    <span className={`pe-delta ${summary.balanced ? 'ok' : 'warn'}`}>{deltaText(summary)}</span>
                    <div className="seg">
                        <button className={variant === 'STUDENT' ? 'active' : ''} onClick={() => setVariant('STUDENT')}>
                            学生版
                        </button>
                        <button className={variant === 'TEACHER' ? 'active' : ''} onClick={() => setVariant('TEACHER')}>
                            教师版
                        </button>
                    </div>
                    <button className="btn ghost" onClick={() => setPreviewOpen(true)} disabled={!html}>
                        预览 / 打印
                    </button>
                    {editable && (
                        <button className="btn ghost" onClick={() => setBlueprintOpen(true)} title="按题数 / 分值 / 易中难自动抽题">
                            按蓝图组卷
                        </button>
                    )}
                    {doc.status === 'DRAFT' ? (
                        <button className="btn primary" onClick={finalize} disabled={busy}>
                            定稿
                        </button>
                    ) : (
                        <button
                            className="btn"
                            onClick={() => run(() => paperReopen(paperId), '已重新打开，可以修改了')}
                        >
                            重新打开
                        </button>
                    )}
                </div>
            </header>

            <div
                className={[
                    'pe-body',
                    leftOpen ? '' : 'pe-no-left',
                    rightOpen ? '' : 'pe-no-right',
                ]
                    .filter(Boolean)
                    .join(' ')}
            >
                <aside className="pe-left">
                    {doc.sections.length === 0 && (
                        <Empty title="还没有大题" hint="先加一个大题，再从题库选题"/>
                    )}
                    {doc.sections.map((section, index) => {
                        const start = numberCursor;
                        numberCursor += section.questions.length;
                        return (
                            <section className="pe-section" key={section.id}>
                                <div className="pe-section-head">
                                    <strong>{fullSectionTitle(section, index)}</strong>
                                    <span className="muted">
                                        {section.questions.length} 题 · {scoreText(section.scoreTotal)} 分
                                    </span>
                                </div>
                                <div className="pe-section-note">
                                    {section.note || '还没有题'}
                                </div>
                                <div className="pe-section-controls">
                                    <label className="pe-inline">
                                        每题
                                        <input
                                            type="number"
                                            min="0"
                                            step="0.5"
                                            value={section.unitScore}
                                            disabled={!editable}
                                            onChange={(event) => changeSectionScore(section, event.target.value)}
                                            onBlur={(event) =>
                                                editable &&
                                                saveSection(section, {
                                                    scoreMode: 'UNIFORM',
                                                    unitScore: Number(event.target.value) || 0,
                                                })
                                            }
                                        />
                                        分
                                    </label>
                                    <span className={`pill ${section.scoreMode === 'UNIFORM' ? 'line' : 'muted'}`}>
                                        {section.scoreMode === 'UNIFORM' ? '统一分值' : '逐题自定'}
                                    </span>
                                    {editable && (
                                        <div className="pe-icon-actions">
                                            <button onClick={() => moveSection(index, -1)} title="上移">↑</button>
                                            <button onClick={() => moveSection(index, 1)} title="下移">↓</button>
                                            <button
                                                onClick={async () => {
                                                    const ok = await confirmDialog({
                                                        title: '删除大题',
                                                        message: `「${fullSectionTitle(section, index)}」里的 ${section.questions.length} 道题也会一起移出本卷。`,
                                                        confirmText: '删除',
                                                        danger: true,
                                                    });
                                                    if (ok) run(() => paperSectionDelete(paperId, section.id), '大题已删除');
                                                }}
                                                title="删除大题"
                                            >
                                                ✕
                                            </button>
                                        </div>
                                    )}
                                </div>
                                <div className="pe-qlist">
                                    {section.questions.map((item, qIndex) => {
                                        const key = `${section.id}:${item.ord}`;
                                        const value = scores[key] ?? item.score;
                                        return (
                                            <div className="pe-q" key={key}>
                                                <span className="pe-q-no">{start + qIndex + 1}</span>
                                                <span className="pe-q-stem" title={excerptOf(item.snapshot)}>
                                                    <RichInline text={excerptOf(item.snapshot)}/>
                                                </span>
                                                <input
                                                    className="pe-q-score"
                                                    type="number"
                                                    min="0"
                                                    step="0.5"
                                                    value={value}
                                                    disabled={!editable}
                                                    onChange={(event) =>
                                                        setScores((current) => ({...current, [key]: event.target.value}))
                                                    }
                                                    onBlur={(event) => commitScore(section, item.ord, event.target.value)}
                                                    onKeyDown={(event) => {
                                                        if (event.key === 'Enter') event.target.blur();
                                                    }}
                                                    title="本题分值"
                                                />
                                                {editable && (
                                                    <span className="pe-q-actions">
                                                        <button
                                                            onClick={() =>
                                                                setReplaceTarget({
                                                                    sectionId: section.id,
                                                                    ord: item.ord,
                                                                    number: start + qIndex + 1,
                                                                    sectionLabel: fullSectionTitle(section, index),
                                                                })
                                                            }
                                                            title="换一道题（题号与分值不变）"
                                                        >
                                                            ⇄
                                                        </button>
                                                        {item.flags?.prev && (
                                                            <button
                                                                onClick={() => undoReplace(section, item.ord)}
                                                                title="撤销上一次换题"
                                                            >
                                                                ↩
                                                            </button>
                                                        )}
                                                        <button onClick={() => moveQuestion(section, item.ord, -1)} title="上移">↑</button>
                                                        <button onClick={() => moveQuestion(section, item.ord, 1)} title="下移">↓</button>
                                                        <button onClick={() => removeQuestion(section, item.ord)} title="移出本卷">✕</button>
                                                    </span>
                                                )}
                                            </div>
                                        );
                                    })}
                                </div>
                                {editable && (
                                    <button className="pe-add" onClick={() => setPicker(section.id)}>
                                        + 从题库选题
                                    </button>
                                )}
                            </section>
                        );
                    })}
                    {editable && (
                        <div className="pe-section-add">
                            <button className="btn ghost" onClick={() => setSectionForm({...EMPTY_SECTION})}>
                                + 新增大题
                            </button>
                            <button
                                className="btn ghost"
                                onClick={() => run(() => paperRefreshSnapshot(paperId), '已同步题库最新版')}
                                title="把卷内快照刷新成题库当前内容（分值、顺序不变）"
                            >
                                同步题库最新
                            </button>
                        </div>
                    )}
                </aside>

                <section className="pe-center" ref={centerRef}>
                    {rendering && !html ? (
                        <Loading text="正在排版…"/>
                    ) : html ? (
                        <div
                            className="pe-paper"
                            style={{
                                width: paperSize(layout).width * MM_TO_PX * centerScale,
                                height: previewPaperHeight(layout, pageCount, centerScale),
                            }}
                        >

                            <PrintPreviewFrame
                                html={html}
                                layout={layout}
                                pageCount={pageCount}
                                scale={centerScale}
                            />
                            </div>
                    ) : (
                        <Empty title="卷面还是空的" hint="点上方「题目」加题，「版面」调纸张"/>
                    )}
                    {warnings.length > 0 && (
                        <div className="pe-warnings">
                            {warnings.map((warning) => (
                                <span key={warning}>{warning}</span>
                            ))}
                        </div>
                    )}
                </section>

                <aside className="pe-right">
                    <h3 className="pe-panel-title">版面</h3>
                    <Field label="纸型" hint="默认 A4，可改 A3 / B5 / 8K / 16K">
                        <select
                            value={layout.paperSize}
                            disabled={!editable}
                            onChange={(event) => patchLayout({paperSize: event.target.value})}
                        >
                            <option value="A4">A4（210×297）</option>
                            <option value="A3">A3（297×420）</option>
                            <option value="B5">B5（176×250）</option>
                            <option value="8K">8K（270×390）</option>
                            <option value="16K">16K（195×270）</option>
                        </select>
                    </Field>
                    <Field label="方向">
                        <select
                            value={layout.orientation}
                            disabled={!editable}
                            onChange={(event) => setOrientation(event.target.value)}
                        >
                            <option value="portrait">纵向</option>
                            <option value="landscape">横向</option>
                        </select>
                    </Field>
                    <div className="pe-margins">
                        {['top', 'right', 'bottom', 'left'].map((side) => (
                            <Field key={side} label={{top: '上', right: '右', bottom: '下', left: '左'}[side]}>
                                <input
                                    type="number"
                                    min="0"
                                    value={layout.margins[side]}
                                    disabled={!editable}
                                    onChange={(event) =>
                                        patchLayout({margins: {[side]: Number(event.target.value) || 0}})
                                    }
                                />
                            </Field>
                        ))}
                    </div>
                    <Field label="分栏">
                        <select
                            value={layout.columns}
                            disabled={!editable}
                            onChange={(event) => patchLayout({columns: Number(event.target.value)})}
                        >
                            <option value={1}>单栏</option>
                            <option value={2}>双栏</option>
                        </select>
                    </Field>
                    <div className="pe-margins">
                        <Field label="字号 pt">
                            <input
                                type="number"
                                step="0.5"
                                min="8"
                                value={layout.fontSize}
                                disabled={!editable}
                                onChange={(event) => patchLayout({fontSize: Number(event.target.value) || 10.5})}
                            />
                        </Field>
                        <Field label="行距">
                            <input
                                type="number"
                                step="0.1"
                                min="1"
                                value={layout.lineHeight}
                                disabled={!editable}
                                onChange={(event) => patchLayout({lineHeight: Number(event.target.value) || 1.6})}
                            />
                        </Field>
                    </div>
                    <div className="pe-margins">
                        <Field label="题距 mm">
                            <input
                                type="number"
                                min="0"
                                value={layout.questionGap}
                                disabled={!editable}
                                onChange={(event) => patchLayout({questionGap: Number(event.target.value) || 0})}
                            />
                        </Field>
                        <Field label="选项列">
                            <select
                                value={layout.optionColumns}
                                disabled={!editable}
                                onChange={(event) => patchLayout({optionColumns: Number(event.target.value)})}
                            >
                                <option value={2}>2 列</option>
                                <option value={4}>4 列</option>
                                <option value={1}>1 列</option>
                            </select>
                        </Field>
                    </div>
                    <Field label="解答题留白（行）" hint="只作用于解答题；客观题不留白，填空固定 3 行。留空按自动（解答 8 行）">
                        <input
                            type="number"
                            min="0"
                            value={layout.answerSpace === 'auto' ? '' : layout.answerSpace}
                            placeholder="自动"
                            disabled={!editable}
                            onChange={(event) =>
                                patchLayout({
                                    answerSpace: event.target.value === '' ? 'auto' : Number(event.target.value),
                                })
                            }
                        />
                    </Field>
                    <label className="pe-check">
                        <input
                            type="checkbox"
                            checked={!!layout.bindingLine}
                            disabled={!editable}
                            onChange={(event) => patchLayout({bindingLine: event.target.checked})}
                        />
                        装订线
                    </label>
                    <Field label="水印文字">
                        <input
                            value={layout.watermark || ''}
                            placeholder="留空不加水印"
                            disabled={!editable}
                            onChange={(event) => patchLayout({watermark: event.target.value || null})}
                        />
                    </Field>
                    <label className="pe-check">
                        <input
                            type="checkbox"
                            checked={!!layout.answerKeyPage}
                            disabled={!editable}
                            onChange={(event) => patchLayout({answerKeyPage: event.target.checked})}
                        />
                        教师版卷末附答案（另起一页）
                    </label>
                    <label className="pe-check">
                        <input
                            type="checkbox"
                            checked={!!layout.answerSheet?.enabled}
                            disabled={!editable}
                            onChange={(event) =>
                                patchLayout({answerSheet: {...layout.answerSheet, enabled: event.target.checked}})
                            }
                        />
                        附答题卡
                    </label>
                    {layout.answerSheet?.enabled && (
                        <>
                            <Field label="答题卡模板" hint="答题卡单独设计，试卷只绑定模板">
                                <select
                                    value={answerSheetTemplate?.id || 'builtin-generic'}
                                    disabled={!editable}
                                    onChange={(event) =>
                                        patchLayout({
                                            answerSheet: {
                                                ...layout.answerSheet,
                                                templateId: event.target.value,
                                            },
                                        })
                                    }
                                >
                                    {answerSheets.map((item) => (
                                        <option key={item.id} value={item.id}>
                                            {item.name}
                                        </option>
                                    ))}
                                </select>
                            </Field>
                            <button
                                className="btn ghost small"
                                onClick={() => setSheetDesignerOpen(true)}
                            >
                                设计 / 管理答题卡模板
                            </button>
                            {answerSheetTemplate?.description && (
                                <p className="field-hint">{answerSheetTemplate.description}</p>
                            )}
                            <details className="pe-sheet-peek" open>
                                <summary>答题卡搭配预览（题号 + 作答区）</summary>
                                <div className="pe-sheet-peek-body">
                                    <SheetPreview
                                        draft={answerSheetTemplate}
                                        entries={sheetEntries}
                                    />
                                </div>
                            </details>
                        </>
                    )}

                    <h3 className="pe-panel-title">分值</h3>
                    <div className="pe-total-row">
                        <span>
                            总分 <strong>{scoreText(doc.totalScore)}</strong>
                        </span>
                        {editable && (
                            <button className="btn small ghost" onClick={() => setTotalOpen(true)}>
                                重新设置
                            </button>
                        )}
                    </div>
                    <p className={`pe-delta-line ${summary.balanced ? 'ok' : 'warn'}`}>{deltaText(summary)}</p>
                    {!summary.balanced && (
                        <p className="field-hint">
                            差值不为 0 时不能定稿：改「每题 X 分」或逐题微调，直到与总分相等。
                        </p>
                    )}
                </aside>
            </div>

            {picker && (
                <QuestionPicker
                    subjectId={doc.subjectId}
                    subjectTree={tree}
                    defaultKind={doc.sections.find((section) => section.id === picker)?.kind || ''}
                    usedIds={usedIds}
                    sectionLabel={
                        fullSectionTitle(
                            doc.sections.find((section) => section.id === picker),
                            doc.sections.findIndex((section) => section.id === picker),
                        ) || '本大题'
                    }
                    onClose={() => setPicker(null)}
                    onPick={async (ids) => {
                        try {
                            const next = await paperAddQuestions(paperId, picker, ids);
                            apply(next);
                            setPicker(null);
                            notify.success(`已加入 ${ids.length} 道题`);
                        } catch (err) {
                            notify.error(err.message);
                        }
                    }}
                />
            )}

            {replaceTarget && (
                <ReplaceDrawer
                    paperId={paperId}
                    sectionId={replaceTarget.sectionId}
                    ord={replaceTarget.ord}
                    number={replaceTarget.number}
                    sectionLabel={replaceTarget.sectionLabel}
                    onClose={() => setReplaceTarget(null)}
                    onPick={applyReplace}
                />
            )}

            {blueprintOpen && (
                <BlueprintModal
                    paperId={paperId}
                    paper={doc}
                    tree={tree}
                    onClose={() => setBlueprintOpen(false)}
                    onFilled={(next) => {
                        apply(next);
                        setBlueprintOpen(false);
                    }}
                />
            )}

            {sheetDesignerOpen && (
                <AnswerSheetDesigner
                    templates={answerSheets}
                    initialId={layout.answerSheet?.templateId}
                    entries={sheetEntries}
                    onClose={() => setSheetDesignerOpen(false)}
                    onChanged={setAnswerSheets}
                />
            )}

            {sectionForm && (
                <Modal
                    title="新增大题"
                    onClose={() => setSectionForm(null)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={() => setSectionForm(null)}>
                                取消
                            </button>
                            <button className="btn primary" onClick={addSection} disabled={busy}>
                                加到大题
                            </button>
                        </>
                    }
                >
                    <div className="form-grid">
                        <Field label="大题标题" wide hint="序号自动生成，不用自己写「一、」">
                            <input
                                autoFocus
                                value={sectionForm.title}
                                placeholder="例如：单项选择题"
                                onChange={(event) =>
                                    setSectionForm((current) => ({...current, title: event.target.value}))
                                }
                            />
                        </Field>
                        <Field label="题型约束">
                            <select
                                value={sectionForm.kind}
                                onChange={(event) =>
                                    setSectionForm((current) => ({...current, kind: event.target.value}))
                                }
                            >
                                <option value="">不限</option>
                                <option value="SINGLE">单选</option>
                                <option value="MULTI">多选</option>
                                <option value="FILL">填空</option>
                                <option value="CALC">解答</option>
                                <option value="JUDGE">判断</option>
                            </select>
                        </Field>
                        <Field label="每题分值">
                            <input
                                type="number"
                                min="0"
                                step="0.5"
                                value={sectionForm.unitScore}
                                onChange={(event) =>
                                    setSectionForm((current) => ({
                                        ...current,
                                        unitScore: event.target.value,
                                    }))
                                }
                            />
                        </Field>
                    </div>
                </Modal>
            )}

            {settingsOpen && (
                <Modal
                    title="试卷信息"
                    onClose={() => setSettingsOpen(false)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={() => setSettingsOpen(false)}>
                                取消
                            </button>
                            <button
                                className="btn primary"
                                onClick={async () => {
                                    await run(
                                        () =>
                                            paperUpdate(paperId, {
                                                title: doc.title,
                                                subjectId: doc.subjectId,
                                                grade: doc.grade,
                                                examType: doc.examType,
                                                durationMin: doc.durationMin,
                                            }),
                                        '已保存',
                                    );
                                    setSettingsOpen(false);
                                }}
                            >
                                保存
                            </button>
                        </>
                    }
                >
                    <div className="form-grid">
                        <Field label="卷名" wide>
                            <input
                                value={doc.title}
                                disabled={!editable}
                                onChange={(event) =>
                                    setDoc((current) => ({...current, title: event.target.value}))
                                }
                            />
                        </Field>
                        <Field label="学科">
                            <select
                                value={doc.subjectId || ''}
                                disabled={!editable}
                                onChange={(event) =>
                                    setDoc((current) => ({...current, subjectId: event.target.value || null}))
                                }
                            >
                                <option value="">不指定</option>
                                {subjects.map((subject) => (
                                    <option key={subject.id} value={subject.id}>
                                        {subject.name}
                                    </option>
                                ))}
                            </select>
                        </Field>
                        <Field label="年级">
                            <input
                                value={doc.grade}
                                disabled={!editable}
                                onChange={(event) =>
                                    setDoc((current) => ({...current, grade: event.target.value}))
                                }
                            />
                        </Field>
                        <Field label="类型">
                            <select
                                value={doc.examType}
                                disabled={!editable}
                                onChange={(event) =>
                                    setDoc((current) => ({...current, examType: event.target.value}))
                                }
                            >
                                {EXAM_TYPES.map((type) => (
                                    <option key={type.value} value={type.value}>
                                        {type.label}
                                    </option>
                                ))}
                            </select>
                        </Field>
                        <Field label="时长（分钟）">
                            <input
                                type="number"
                                min="0"
                                value={doc.durationMin}
                                disabled={!editable}
                                onChange={(event) =>
                                    setDoc((current) => ({
                                        ...current,
                                        durationMin: Number(event.target.value) || 0,
                                    }))
                                }
                            />
                        </Field>
                    </div>
                </Modal>
            )}

            {totalOpen && <TotalModal paperId={paperId} current={doc.totalScore} onClose={() => setTotalOpen(false)} onDone={apply}/>}

            {finalizeError && (
                <Modal
                    title="还不能定稿"
                    onClose={() => setFinalizeError('')}
                    footer={
                        <button className="btn primary" onClick={() => setFinalizeError('')}>
                            知道了
                        </button>
                    }
                >
                    <p className="pe-finalize-error">{finalizeError}</p>
                    <p className="field-hint">分值要印在卷面上，必须由人确认；系统不会自动把差额塞给最后一道题。</p>
                </Modal>
            )}

            {previewOpen && (
                <PaperPreview
                    doc={{...doc, layout}}
                    html={html}
                    pageCount={pageCount}
                    warnings={warnings}
                    variant={variant}
                    onVariant={setVariant}
                    onClose={() => setPreviewOpen(false)}
                    onPrint={print}
                    onExport={exportFile}
                    rendering={rendering}
                />
            )}
        </div>
    );
}

function TotalModal({paperId, current, onClose, onDone}) {
    const [value, setValue] = useState(current);
    const [busy, setBusy] = useState(false);
    return (
        <Modal
            title="重新设置总分"
            subtitle="总分是合同值：改完必须重新配平，定稿时要求题分之和与它完全相等"
            onClose={onClose}
            footer={
                <>
                    <button className="btn ghost" onClick={onClose}>
                        取消
                    </button>
                    <button
                        className="btn danger"
                        disabled={busy}
                        onClick={async () => {
                            setBusy(true);
                            try {
                                const next = await paperSetTotal(paperId, Number(value), true);
                                onDone(next);
                                notify.success('总分已更新，请重新配平');
                                onClose();
                            } catch (error) {
                                notify.error(error.message);
                            } finally {
                                setBusy(false);
                            }
                        }}
                    >
                        确认改总分
                    </button>
                </>
            }
        >
            <Field label="新的卷面总分">
                <input type="number" min="1" value={value} onChange={(event) => setValue(event.target.value)}/>
            </Field>
        </Modal>
    );
}
