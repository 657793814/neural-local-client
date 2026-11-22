import {useCallback, useEffect, useMemo, useState} from 'react';
import {
    paperArchive,
    paperBlueprintAgain,
    paperCreate,
    paperDelete,
    paperDuplicate,
    paperList,
    paperRestore,
    paperSchemes,
} from '../lib/api.js';
import {examTypeLabel, PAPER_STATUS} from '../lib/paper/model.js';
import {scoreText} from '../lib/paper/score.js';
import {formatTime, subjectList} from '../lib/format.js';
import {BUILTIN_ANSWER_SHEETS, loadAnswerSheets} from '../lib/paper/answerSheet.js';
import {AnswerSheetDesigner} from '../components/paper/AnswerSheetDesigner.jsx';
import {
    Empty,
    Field,
    Loading,
    Modal,
    PageHead,
    Pill,
    confirmDialog,
    notify,
} from '../components/ui.jsx';

const STATUS_TABS = [
    {key: '', label: '全部'},
    {key: 'DRAFT', label: '草稿'},
    {key: 'FINAL', label: '已定稿'},
    {key: 'ARCHIVED', label: '已归档'},
    {key: 'TRASH', label: '回收站'},
];

const EMPTY_FORM = {
    title: '',
    subjectId: '',
    grade: '',
    examType: 'EXAM',
    scoreScheme: 'XGK1_MATH',
    durationMin: 120,
    answerSheetId: 'builtin-generic',
    answerSheetEnabled: true,
};

/** 学科 → 常见答题卡规制，建卷时先给个合适的默认值，随时可改。 */
function suggestAnswerSheet(subjectName = '') {
    if (subjectName.includes('数学')) return 'builtin-math';
    if (subjectName.includes('语文')) return 'builtin-chinese';
    if (subjectName.includes('英语')) return 'builtin-english';
    if (/物理|化学|生物|科学/.test(subjectName)) return 'builtin-science';
    if (/政治|历史|地理|道德/.test(subjectName)) return 'builtin-liberal';
    return 'builtin-generic';
}

export default function Papers({tree = [], onGo, onChanged}) {
    const [query, setQuery] = useState({keyword: '', status: '', trashed: false});
    const [rows, setRows] = useState([]);
    const [loading, setLoading] = useState(true);
    const [schemes, setSchemes] = useState([]);
    const [creating, setCreating] = useState(null);
    const [answerSheets, setAnswerSheets] = useState(BUILTIN_ANSWER_SHEETS);
    const [sheetDesignerOpen, setSheetDesignerOpen] = useState(false);
    const [busy, setBusy] = useState(false);

    const subjects = useMemo(() => subjectList(tree), [tree]);

    const load = useCallback(async () => {
        setLoading(true);
        try {
            const page = await paperList({
                keyword: query.keyword || null,
                status: query.status || null,
                trashed: query.trashed,
                page: 1,
                size: 100,
            });
            setRows(page.items || []);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setLoading(false);
        }
    }, [query]);

    useEffect(() => {
        load();
    }, [load]);

    useEffect(() => {
        paperSchemes()
            .then(setSchemes)
            .catch(() => {});
        loadAnswerSheets().then(setAnswerSheets).catch(() => {});
    }, []);

    const openCreate = () => {
        const defaultScheme = schemes.find((item) => item.id === 'XGK1_MATH') || schemes[0];
        const firstSubject = subjects[0];
        setCreating({
            ...EMPTY_FORM,
            subjectId: firstSubject?.id || '',
            scoreScheme: defaultScheme?.id || 'CUSTOM',
            durationMin: defaultScheme?.durationMin || 90,
            answerSheetId: suggestAnswerSheet(firstSubject?.name || ''),
        });
    };

    const submitCreate = async () => {
        if (!creating.title.trim()) {
            notify.warning('先给试卷起个名字');
            return;
        }
        setBusy(true);
        try {
            const paper = await paperCreate({
                title: creating.title.trim(),
                subjectId: creating.subjectId || null,
                grade: creating.grade.trim(),
                examType: creating.examType,
                scoreScheme: creating.scoreScheme,
                durationMin: Number(creating.durationMin) || 90,
                layout: {
                    answerSheet: {
                        enabled: creating.answerSheetEnabled,
                        templateId: creating.answerSheetId,
                    },
                },
            });
            setCreating(null);
            onChanged?.();
            onGo?.(`papers/${paper.id}`);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const act = async (label, work) => {
        try {
            await work();
            notify.success(label);
            await load();
            onChanged?.();
        } catch (error) {
            notify.error(error.message);
        }
    };

    const remove = async (paper) => {
        const ok = await confirmDialog({
            title: '删除试卷',
            message: `「${paper.title}」会进入回收站，可随时恢复。`,
            confirmText: '删除',
            danger: true,
        });
        if (!ok) return;
        act('已移入回收站', () => paperDelete(paper.id));
    };

    // 再组一套（B 卷 / 补考卷）：复制一份草稿并按蓝图重抽，抽完不撞题。
    const again = async (paper) => {
        if (!paper.questionCount) {
            notify.warning('这份卷还没有题，先打开它加题或按蓝图组卷');
            return;
        }
        const ok = await confirmDialog({
            title: '按这份卷再组一套',
            message: `会新建一份草稿卷，按「${paper.title}」的题数、分值与易中难重新抽题，并避开这份卷已用的题。抽不满时会整次回滚，不会留半份卷子。`,
            confirmText: '再组一套',
        });
        if (!ok) return;
        try {
            const result = await paperBlueprintAgain(paper.id);
            if (!result.report.filled) {
                notify.error(result.report.message);
                return;
            }
            notify.success(result.report.message);
            await load();
            onChanged?.();
            onGo?.(`papers/${result.paper.id}`);
        } catch (error) {
            notify.error(error.message);
        }
    };

    return (
        <div className="page">
            <PageHead
                title="试卷"
                subtitle="组卷 · 排版 · 答题卡 · 打印：从题库挑题，按标准卷分值配平，一键出可打印的卷子"
                help="papers"
                actions={
                    <>
                        <input
                            className="input paper-search"
                            placeholder="搜索卷名 / 年级 / 备注"
                            value={query.keyword}
                            onChange={(event) =>
                                setQuery((current) => ({...current, keyword: event.target.value}))
                            }
                        />
                        <button className="btn ghost" onClick={() => setSheetDesignerOpen(true)}>
                            答题卡模板
                        </button>
                        <button className="btn primary" onClick={openCreate}>
                            新建试卷
                        </button>
                    </>
                }
            />

            <div className="paper-tabs">
                {STATUS_TABS.map((tab) => (
                    <button
                        key={tab.key || 'all'}
                        className={`paper-tab ${
                            (tab.key === 'TRASH' ? query.trashed : !query.trashed && query.status === tab.key)
                                ? 'active'
                                : ''
                        }`}
                        onClick={() =>
                            setQuery((current) => ({
                                ...current,
                                status: tab.key === 'TRASH' ? '' : tab.key,
                                trashed: tab.key === 'TRASH',
                            }))
                        }
                    >
                        {tab.label}
                    </button>
                ))}
            </div>

            {loading ? (
                <Loading text="正在取试卷…"/>
            ) : rows.length === 0 ? (
                <Empty
                    title={query.trashed ? '回收站是空的' : '还没有试卷'}
                    hint={
                        query.trashed
                            ? '删掉的试卷会先放这里，随时能恢复'
                            : '新建一份卷子，从题库挑题后自动按标准卷配分'
                    }
                    action={
                        !query.trashed && (
                            <button className="btn primary" onClick={openCreate}>
                                新建第一份试卷
                            </button>
                        )
                    }
                />
            ) : (
                <div className="paper-grid">
                    {rows.map((paper) => {
                        const status = PAPER_STATUS[paper.status] || PAPER_STATUS.DRAFT;
                        return (
                            <article key={paper.id} className="paper-card">
                                <div className="paper-card-head">
                                    <h3
                                        onClick={() => onGo?.(`papers/${paper.id}`)}
                                        title="打开试卷"
                                    >
                                        {paper.title}
                                    </h3>
                                    <Pill tone={status.tone}>{status.label}</Pill>
                                </div>
                                <div className="paper-card-meta">
                                    <span>{paper.subjectName || '未设学科'}</span>
                                    {paper.grade && <span>{paper.grade}</span>}
                                    <span>{examTypeLabel(paper.examType)}</span>
                                    {paper.durationMin > 0 && <span>{paper.durationMin} 分钟</span>}
                                </div>
                                <div className="paper-card-stats">
                                    <div>
                                        <strong>{paper.questionCount}</strong>
                                        <span>题</span>
                                    </div>
                                    <div>
                                        <strong>{scoreText(paper.assignedScore)}</strong>
                                        <span>/ {scoreText(paper.totalScore)} 分</span>
                                    </div>
                                    <div className={paper.assignedScore === paper.totalScore ? 'ok' : 'warn'}>
                                        <strong>
                                            {paper.assignedScore === paper.totalScore
                                                ? '已配平'
                                                : paper.questionCount
                                                  ? `差 ${scoreText(paper.totalScore - paper.assignedScore)} 分`
                                                  : '未配分'}
                                        </strong>
                                        <span>分值</span>
                                    </div>
                                </div>
                                <div className="paper-card-foot">
                                    <span className="muted">
                                        更新于 {formatTime(paper.updatedAt)}
                                        {paper.printCount > 0
                                            ? ` · 打印 ${paper.printCount} 次（${formatTime(paper.lastPrintedAt)}）`
                                            : ' · 还没打印过'}
                                    </span>
                                    <div className="paper-card-actions">
                                        {!query.trashed && (
                                            <>
                                                <button
                                                    className="btn small ghost"
                                                    onClick={() => onGo?.(`papers/${paper.id}`)}
                                                >
                                                    打开
                                                </button>
                                                <button
                                                    className="btn small ghost"
                                                    onClick={() => onGo?.(`papers/${paper.id}/preview`)}
                                                >
                                                    打印
                                                </button>
                                                <button
                                                    className="btn small ghost"
                                                    onClick={() =>
                                                        act('已复制一份草稿', () => paperDuplicate(paper.id))
                                                    }
                                                >
                                                    复制
                                                </button>
                                                <button
                                                    className="btn small ghost"
                                                    title="按这份卷的蓝图再抽一套不重题的卷（B 卷 / 补考卷）"
                                                    onClick={() => again(paper)}
                                                >
                                                    再组一套
                                                </button>
                                                {paper.status !== 'ARCHIVED' && (
                                                    <button
                                                        className="btn small ghost"
                                                        onClick={() => act('已归档', () => paperArchive(paper.id))}
                                                    >
                                                        归档
                                                    </button>
                                                )}
                                                <button
                                                    className="btn small danger"
                                                    onClick={() => remove(paper)}
                                                >
                                                    删除
                                                </button>
                                            </>
                                        )}
                                        {query.trashed && (
                                            <button
                                                className="btn small"
                                                onClick={() => act('已恢复', () => paperRestore(paper.id))}
                                            >
                                                恢复
                                            </button>
                                        )}
                                    </div>
                                </div>
                            </article>
                        );
                    })}
                </div>
            )}

            {creating && (
                <Modal
                    title="新建试卷"
                    subtitle="先选标准卷方案定总分与大题结构，后面都能改"
                    onClose={() => setCreating(null)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={() => setCreating(null)}>
                                取消
                            </button>
                            <button className="btn primary" onClick={submitCreate} disabled={busy}>
                                {busy ? '正在建…' : '建卷并开始选题'}
                            </button>
                        </>
                    }
                >
                    <div className="form-grid">
                        <Field label="卷名" wide>
                            <input
                                autoFocus
                                value={creating.title}
                                placeholder="例如：2026 学年第一学期期中数学"
                                onChange={(event) =>
                                    setCreating((current) => ({...current, title: event.target.value}))
                                }
                            />
                        </Field>
                        <Field label="学科">
                            <select
                                value={creating.subjectId}
                                onChange={(event) =>
                                    setCreating((current) => ({...current, subjectId: event.target.value}))
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
                                value={creating.grade}
                                placeholder="例如：高二"
                                onChange={(event) =>
                                    setCreating((current) => ({...current, grade: event.target.value}))
                                }
                            />
                        </Field>
                        <Field label="类型">
                            <select
                                value={creating.examType}
                                onChange={(event) =>
                                    setCreating((current) => ({...current, examType: event.target.value}))
                                }
                            >
                                <option value="EXAM">考试卷</option>
                                <option value="PRACTICE">练习卷</option>
                                <option value="QUIZ">小测</option>
                            </select>
                        </Field>
                        <Field label="标准卷方案" hint="只预填总分与大题结构，不锁定分值">
                            <select
                                value={creating.scoreScheme}
                                onChange={(event) => {
                                    const scheme = schemes.find((item) => item.id === event.target.value);
                                    setCreating((current) => ({
                                        ...current,
                                        scoreScheme: event.target.value,
                                        durationMin: scheme?.durationMin || current.durationMin,
                                    }));
                                }}
                            >
                                {schemes.map((scheme) => (
                                    <option key={scheme.id} value={scheme.id}>
                                        {scheme.name} · {scoreText(scheme.totalScore)} 分
                                    </option>
                                ))}
                                {schemes.length === 0 && <option value="CUSTOM">自定义</option>}
                            </select>
                        </Field>
                        <Field label="考试时长（分钟）">
                            <input
                                type="number"
                                min="0"
                                value={creating.durationMin}
                                onChange={(event) =>
                                    setCreating((current) => ({
                                        ...current,
                                        durationMin: Number(event.target.value),
                                    }))
                                }
                            />
                        </Field>
                    </div>
                </Modal>
            )}

            {sheetDesignerOpen && (
                <AnswerSheetDesigner
                    templates={answerSheets}
                    onClose={() => setSheetDesignerOpen(false)}
                    onChanged={setAnswerSheets}
                />
            )}
        </div>
    );
}
