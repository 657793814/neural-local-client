import {useCallback, useEffect, useState} from 'react';
import {
    assignmentClose,
    assignmentDelete,
    assignmentGet,
    assignmentList,
    assignmentPublish,
    assignmentReopen,
    assignmentSave,
    attemptBatchGrade,
    attemptClear,
    attemptSheet,
    attemptSubmit,
    centerAssignments,
    centerBindAssignment,
    centerBinding,
    centerClasses,
    centerPullAssets,
    classMembers,
    studentBindCenter,
    classGet,
} from '../lib/api.js';
import {Rich, RichInline} from '../lib/Math.jsx';
import {typeLabel} from '../lib/format.js';
import {
    Card,
    Drawer,
    Empty,
    Field,
    Modal,
    PageHead,
    Pill,
    confirmDialog,
    notify,
} from '../components/ui.jsx';
import AssignmentComposer from '../components/AssignmentComposer.jsx';
import ReviewSheetView from '../components/ReviewSheet.jsx';

const STATUS = {
    DRAFT: {text: '草稿', tone: 'muted'},
    PUBLISHED: {text: '进行中', tone: 'ok'},
    CLOSED: {text: '已关闭', tone: 'line'},
};

function dueText(value) {
    if (!value) return '不限时';
    return value.replace('T', ' ').slice(0, 16);
}

function scoreText(value) {
    const number = Number(value || 0);
    return Number.isInteger(number) ? String(number) : number.toFixed(1);
}

export default function Assignments({classId, onGo, onChanged}) {
    const [klass, setKlass] = useState(null);
    const [rows, setRows] = useState([]);
    const [loading, setLoading] = useState(true);
    const [composer, setComposer] = useState(null);
    const [detail, setDetail] = useState(null);
    const [sheet, setSheet] = useState(null);
    const [entry, setEntry] = useState(null);
    const [drafts, setDrafts] = useState({});
    const [busy, setBusy] = useState(false);
    const [centerSync, setCenterSync] = useState(null);
    const [review, setReview] = useState(null);

    const load = useCallback(async () => {
        if (!classId) return;
        try {
            const [info, list] = await Promise.all([classGet(classId), assignmentList(classId)]);
            setKlass(info);
            setRows(list);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setLoading(false);
        }
    }, [classId]);

    useEffect(() => {
        load();
    }, [load]);

    const openDetail = async (id) => {
        try {
            setDetail(await assignmentGet(id));
        } catch (error) {
            notify.error(error.message);
        }
    };

    const refreshDetail = async (id) => {
        const next = await assignmentGet(id);
        setDetail(next);
        return next;
    };

    const newForm = () => setComposer({initial: null});

    const editForm = async (row) => {
        try {
            const data = await assignmentGet(row.id);
            setComposer({
                initial: {
                    id: data.assignment.id,
                    title: data.assignment.title,
                    type: data.assignment.type,
                    description: data.assignment.description,
                    dueAt: (data.assignment.dueAt || '').slice(0, 16),
                    lateHours: data.assignment.lateHours,
                    questions: data.questions.map((item) => ({
                        questionId: item.question.id,
                        score: item.score,
                        stem: item.question.stem,
                        type: item.question.type,
                        difficulty: item.question.difficulty,
                        knowledge: (item.question.knowledge || []).map((point) => point.name),
                    })),
                },
            });
        } catch (error) {
            notify.error(error.message);
        }
    };

    const saveAssignment = async (payload) => {
        const saved = await assignmentSave(payload);
        setComposer(null);
        await load();
        onChanged?.();
        if (detail?.assignment.id === saved.id) await refreshDetail(saved.id);
        notify.success(payload.id ? '已保存' : '草稿已建好，检查无误后发布');
    };

    const publish = async (row) => {
        try {
            await assignmentPublish(row.id);
            await load();
            if (detail?.assignment.id === row.id) await refreshDetail(row.id);
            notify.success('已发布，学生端可以做了');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const closeOrReopen = async (row) => {
        const closing = row.status !== 'CLOSED';
        if (closing) {
            const ok = await confirmDialog({
                title: '关闭作业',
                message: '关闭后不再接收新的提交（已有作答和成绩都保留），需要时可以重新打开。',
                confirmText: '关闭',
            });
            if (!ok) return;
        }
        try {
            if (closing) await assignmentClose(row.id);
            else await assignmentReopen(row.id);
            await load();
            if (detail?.assignment.id === row.id) await refreshDetail(row.id);
            notify.success(closing ? '已关闭' : '已重新开放');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const remove = async (row) => {
        const ok = await confirmDialog({
            title: '删除作业',
            message: `「${row.title}」连同作答记录会从列表里移除（数据仍在本机，可追溯）。`,
            confirmText: '删除',
            danger: true,
        });
        if (!ok) return;
        try {
            await assignmentDelete(row.id);
            if (detail?.assignment.id === row.id) setDetail(null);
            await load();
            onChanged?.();
            notify.success('已删除');
        } catch (error) {
            notify.error(error.message);
        }
    };

    // 学生交的是纸面作业时，老师照着答案把作答录进来，之后照常判分与批改。
    const startEntry = (row) => {
        setEntry({student: {id: row.studentId, name: row.studentName}, answers: {}});
    };

    const setAnswer = (questionId, answer) => {
        setEntry((current) => ({
            ...current,
            answers: {...current.answers, [questionId]: answer},
        }));
    };

    const saveEntry = async () => {
        if (!entry) return;
        setBusy(true);
        try {
            const result = await attemptSubmit({
                assignmentId: detail.assignment.id,
                studentId: entry.student.id,
                answers: detail.questions.map((item) => ({
                    questionId: item.question.id,
                    answer: entry.answers[item.question.id] || '',
                })),
            });
            setEntry(null);
            await refreshDetail(detail.assignment.id);
            await load();
            notify.success(
                `已记录：自动判 ${result.graded} 题，待批 ${result.pending} 题`,
            );
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const openSheet = async (studentId) => {
        try {
            const data = await attemptSheet(detail.assignment.id, studentId);
            setSheet(data);
            const next = {};
            data.items.forEach((item) => {
                next[item.attemptId] = {
                    score: item.score ?? 0,
                    remark: item.remark || '',
                };
            });
            setDrafts(next);
        } catch (error) {
            notify.error(error.message);
        }
    };

    const saveGrades = async () => {
        if (!sheet) return;
        setBusy(true);
        try {
            const items = sheet.items.map((item) => ({
                attemptId: item.attemptId,
                score: Number(drafts[item.attemptId]?.score ?? 0),
                remark: drafts[item.attemptId]?.remark || '',
            }));
            await attemptBatchGrade(items);
            setSheet(null);
            await refreshDetail(detail.assignment.id);
            await load();
            notify.success('批改已保存');
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const clearSubmission = async () => {
        const ok = await confirmDialog({
            title: '撤销提交',
            message: `撤销 ${sheet.student.name} 这次的作答？成绩一并作废，学生可以重做。`,
            confirmText: '撤销',
            danger: true,
        });
        if (!ok) return;
        try {
            await attemptClear(detail.assignment.id, sheet.student.id);
            setSheet(null);
            await refreshDetail(detail.assignment.id);
            await load();
            notify.success('已撤销，可以重做');
        } catch (error) {
            notify.error(error.message);
        }
    };

    if (!classId) {
        return (
            <div className="page">
                <Empty title="先选一个班级" hint="作业挂在班级下面。">
                    <button className="btn primary" onClick={() => onGo('classes')}>
                        去班级
                    </button>
                </Empty>
            </div>
        );
    }

    const total = rows.reduce((sum, row) => sum + row.questionCount, 0);
    const pending = rows.reduce((sum, row) => sum + row.pendingGrade, 0);

    return (
        <div className="page">
            <PageHead
                title={klass ? `${klass.name} · 作业` : '作业'}
                subtitle={
                    klass
                        ? `${klass.subjectName || '未设学科'} · ${klass.memberCount} 人 · ${rows.length} 份作业${pending ? ` · ${pending} 题待批` : ''}`
                        : '正在读班级…'
                }
                actions={
                    <>
                        <button className="btn ghost" onClick={() => onGo('classes')}>
                            返回班级
                        </button>
                        <button className="btn ghost" onClick={() => onGo('grading')}>
                            拍照批改
                        </button>
                        <button className="btn primary" onClick={newForm}>
                            布置作业
                        </button>
                    </>
                }
            />

            {loading ? (
                <div className="loading">正在读作业…</div>
            ) : rows.length === 0 ? (
                <Card>
                    <Empty
                        title="还没有作业"
                        hint="从本地题库选题，设好分值和截止时间，发布后就能逐个批改。"
                        action={
                            <button className="btn primary" onClick={newForm}>
                                布置第一份作业
                            </button>
                        }
                    />
                </Card>
            ) : (
                <div className="teach-grid">
                    {rows.map((row) => {
                        const status = STATUS[row.status] || STATUS.DRAFT;
                        return (
                            <article key={row.id} className="teach-card">
                                <div className="teach-card-head">
                                    <h3>{row.title}</h3>
                                    <Pill tone={status.tone}>{status.text}</Pill>
                                </div>
                                <p className="teach-card-sub">
                                    {row.type === 'EXAM' ? '测验' : '练习'} · 截止 {dueText(row.dueAt)} ·
                                    满分 {scoreText(row.totalScore)}
                                </p>
                                <div className="teach-card-stats">
                                    <span>
                                        <strong>{row.questionCount}</strong> 题
                                    </span>
                                    <span>
                                        <strong>{row.submittedCount}</strong>/{row.studentCount} 已交
                                    </span>
                                    {row.pendingGrade > 0 ? (
                                        <span className="hot">
                                            <strong>{row.pendingGrade}</strong> 待批
                                        </span>
                                    ) : (
                                        <span className="muted">{row.status === 'DRAFT' ? '未发布' : '无待批'}</span>
                                    )}
                                </div>
                                <div className="teach-card-actions">
                                    <button className="btn primary small" onClick={() => openDetail(row.id)}>
                                        批改 / 成绩
                                    </button>
                                    {row.status !== 'DRAFT' && (
                                        <button className="btn ghost small" onClick={() => setReview(row)}>
                                            讲评清单
                                        </button>
                                    )}
                                    {row.status === 'DRAFT' && (
                                        <>
                                            <button className="btn ghost small" onClick={() => editForm(row)}>
                                                编辑
                                            </button>
                                            <button className="btn ghost small" onClick={() => publish(row)}>
                                                发布
                                            </button>
                                        </>
                                    )}
                                    {row.status !== 'DRAFT' && (
                                        <button className="btn ghost small" onClick={() => closeOrReopen(row)}>
                                            {row.status === 'CLOSED' ? '重开' : '关闭'}
                                        </button>
                                    )}
                                    <button className="btn ghost small" onClick={() => setCenterSync(row)}>
                                        中心照片
                                    </button>
                                    <button className="btn ghost small danger-text" onClick={() => remove(row)}>
                                        删除
                                    </button>
                                </div>
                            </article>
                        );
                    })}
                    <p className="teach-foot-note">
                        共 {rows.length} 份作业、{total} 道题；待批 {pending} 题。
                    </p>
                </div>
            )}

            {centerSync && (
                <CenterSyncDialog
                    assignment={centerSync}
                    localClassId={classId}
                    onClose={() => setCenterSync(null)}
                    onPulled={() => {
                        notify.info('照片已进批改台，去「拍照批改」里切题给分');
                    }}
                />
            )}

            {composer && (
                <AssignmentComposer
                    classId={classId}
                    subjectId={klass?.subjectId || null}
                    initial={composer.initial}
                    onClose={() => setComposer(null)}
                    onSave={saveAssignment}
                />
            )}
            {detail && (
                <Drawer
                    title={detail.assignment.title}
                    subtitle={`${detail.assignment.questionCount} 题 · 满分 ${scoreText(detail.assignment.totalScore)} · 截止 ${dueText(detail.assignment.dueAt)}`}
                    onClose={() => setDetail(null)}
                >
                    <div className="teach-detail-bar">
                        <Pill tone={(STATUS[detail.assignment.status] || STATUS.DRAFT).tone}>
                            {(STATUS[detail.assignment.status] || STATUS.DRAFT).text}
                        </Pill>
                        {detail.assignment.status === 'DRAFT' && (
                            <button className="btn primary small" onClick={() => publish(detail.assignment)}>
                                发布
                            </button>
                        )}
                        <button
                            className="btn ghost small"
                            onClick={() => setReview(detail.assignment)}
                        >
                            讲评清单
                        </button>
                    </div>

                    {detail.submissions.length === 0 ? (
                        <Empty title="班里还没有学生" hint="先把学生加进班级，再来看提交情况。"/>
                    ) : (
                        <div className="teach-table submissions">
                            <div className="teach-tr teach-th">
                                <span>学生</span>
                                <span>状态</span>
                                <span>得分</span>
                                <span>提交时间</span>
                                <span>操作</span>
                            </div>
                            {detail.submissions.map((row) => (
                                <div key={row.studentId} className="teach-tr">
                                    <span className="teach-name">{row.studentName}</span>
                                    <span>
                                        {!row.submissionId ? (
                                            <Pill tone="muted">未交</Pill>
                                        ) : row.pending > 0 ? (
                                            <Pill tone="warn">待批 {row.pending}</Pill>
                                        ) : (
                                            <Pill tone="ok">已批</Pill>
                                        )}
                                        {row.late && <Pill tone="danger">迟交</Pill>}
                                    </span>
                                    <span>{row.submissionId ? `${scoreText(row.score)} / ${scoreText(row.maxScore)}` : '—'}</span>
                                    <span className="muted">
                                        {row.submittedAt ? row.submittedAt.replace('T', ' ').slice(5, 16) : '—'}
                                    </span>
                                    <span>
                                        {row.submissionId ? (
                                            <button className="btn ghost small" onClick={() => openSheet(row.studentId)}>
                                                {row.pending > 0 ? '批改' : '查看'}
                                            </button>
                                        ) : (
                                            <button className="btn ghost small" onClick={() => startEntry(row)}>
                                                录入作答
                                            </button>
                                        )}
                                    </span>
                                </div>
                            ))}
                        </div>
                    )}

                    <div className="teach-detail-block">
                        <strong>题目</strong>
                        <ol className="teach-question-list">
                            {detail.questions.map((item) => (
                                <li key={item.question.id}>
                                    <div className="teach-question-line">
                                        <span className="row-no">{item.ord + 1}</span>
                                        <Rich text={item.question.stem} className="clamp-2"/>
                                        <span className="muted">{scoreText(item.score)} 分</span>
                                    </div>
                                </li>
                            ))}
                        </ol>
                    </div>
                </Drawer>
            )}

            {entry && detail && (
                <Drawer
                    title={`录入作答 · ${entry.student.name}`}
                    subtitle="纸面作业照着答案录进来；客观题选选项，其余直接写答案"
                    onClose={() => setEntry(null)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={() => setEntry(null)}>
                                取消
                            </button>
                            <button className="btn primary" onClick={saveEntry} disabled={busy}>
                                保存并判分
                            </button>
                        </>
                    }
                >
                    {detail.questions.map((item, index) => {
                        const question = item.question;
                        const value = entry.answers[question.id] || '';
                        const options = question.options || [];
                        const multi = question.type === 'MULTI';
                        return (
                            <section key={question.id} className="teach-grade-item">
                                <header>
                                    <span className="row-no">第 {index + 1} 题</span>
                                    <Pill tone="line">{typeLabel(question.type)}</Pill>
                                    <span className="muted">{scoreText(item.score)} 分</span>
                                </header>
                                <Rich text={question.stem} className="teach-grade-stem"/>
                                {options.length > 0 ? (
                                    <div className="teach-options">
                                        {options.map((option) => {
                                            const chosen = multi
                                                ? value.includes(option.key)
                                                : value === option.key;
                                            return (
                                                <button
                                                    key={option.key}
                                                    className={`chip ${chosen ? 'active' : ''}`}
                                                    onClick={() => {
                                                        if (!multi) {
                                                            setAnswer(question.id, chosen ? '' : option.key);
                                                            return;
                                                        }
                                                        const letters = new Set(value.split(''));
                                                        if (chosen) letters.delete(option.key);
                                                        else letters.add(option.key);
                                                        setAnswer(
                                                            question.id,
                                                            [...letters].filter(Boolean).sort().join(''),
                                                        );
                                                    }}
                                                >
                                                    {option.key}. {option.text}
                                                </button>
                                            );
                                        })}
                                    </div>
                                ) : (
                                    <textarea
                                        rows={2}
                                        className="teach-entry-answer"
                                        placeholder="学生写的答案（留空表示没做）"
                                        value={value}
                                        onChange={(event) => setAnswer(question.id, event.target.value)}
                                    />
                                )}
                            </section>
                        );
                    })}
                </Drawer>
            )}

            {sheet && (
                <Drawer
                    title={`批改 · ${sheet.student.name}`}
                    subtitle={
                        sheet.submittedAt
                            ? `提交于 ${sheet.submittedAt.replace('T', ' ').slice(0, 16)}${sheet.late ? '（迟交）' : ''}`
                            : '还没有提交'
                    }
                    onClose={() => setSheet(null)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={clearSubmission}>
                                撤销提交
                            </button>
                            <button className="btn primary" onClick={saveGrades} disabled={busy || sheet.items.length === 0}>
                                保存批改
                            </button>
                        </>
                    }
                >
                    {sheet.items.length === 0 ? (
                        <Empty title="还没有作答" hint="学生提交后这里会出现逐题答案与判分。"/>
                    ) : (
                        <>
                            <div className="teach-grade-total">
                                合计{' '}
                                <strong>
                                    {scoreText(
                                        sheet.items.reduce(
                                            (sum, item) => sum + (Number(drafts[item.attemptId]?.score) || 0),
                                            0,
                                        ),
                                    )}
                                </strong>{' '}
                                / {scoreText(sheet.assignment.totalScore)}
                            </div>
                            {sheet.items.map((item, index) => {
                                const draft = drafts[item.attemptId] || {score: 0, remark: ''};
                                const objective = item.correct !== null;
                                return (
                                    <section key={item.attemptId} className="teach-grade-item">
                                        <header>
                                            <span className="row-no">第 {index + 1} 题</span>
                                            <Pill tone="line">{typeLabel(item.question.type)}</Pill>
                                            <span className="muted">
                                                {scoreText(draft.score)} / {scoreText(item.maxScore)} 分
                                            </span>
                                            {objective && (
                                                <Pill tone={item.correct ? 'ok' : 'danger'}>
                                                    {item.correct ? '自动判对' : '自动判错'}
                                                </Pill>
                                            )}
                                        </header>
                                        <Rich text={item.question.stem} className="teach-grade-stem"/>
                                        <div className="teach-answer">
                                            <span className="teach-answer-label">学生答案</span>
                                            <span className={item.answer ? '' : 'muted'}>
                                                {item.answer ? <RichInline text={item.answer}/> : '（未作答）'}
                                            </span>
                                        </div>
                                        <div className="teach-answer ref">
                                            <span className="teach-answer-label">参考答案</span>
                                            <span>
                                                {item.question.answer
                                                    ? <RichInline text={item.question.answer}/>
                                                    : '（没填参考答案）'}
                                            </span>
                                        </div>
                                        {item.question.analysis && (
                                            <details className="teach-analysis">
                                                <summary>解析</summary>
                                                <Rich text={item.question.analysis}/>
                                            </details>
                                        )}
                                        <div className="teach-grade-form">
                                            <Field label="得分">
                                                <input
                                                    type="number"
                                                    min="0"
                                                    max={item.maxScore}
                                                    step="0.5"
                                                    value={draft.score}
                                                    onChange={(event) =>
                                                        setDrafts({
                                                            ...drafts,
                                                            [item.attemptId]: {
                                                                ...draft,
                                                                score: event.target.value,
                                                            },
                                                        })
                                                    }
                                                />
                                            </Field>
                                            <Field label="批改备注（仅本机）" wide>
                                                <input
                                                    value={draft.remark}
                                                    placeholder="写给自己的话：错在哪、讲评时提一句"
                                                    onChange={(event) =>
                                                        setDrafts({
                                                            ...drafts,
                                                            [item.attemptId]: {
                                                                ...draft,
                                                                remark: event.target.value,
                                                            },
                                                        })
                                                    }
                                                />
                                            </Field>
                                        </div>
                                    </section>
                                );
                            })}
                        </>
                    )}
                </Drawer>
            )}

            {review && (
                <ReviewSheetView
                    assignmentId={review.id}
                    title={review.title}
                    onClose={() => setReview(null)}
                />
            )}
        </div>
    );
}

/**
 * 中心同步：把本地作业绑到中心作业，再把学生在家提交的照片拉进批改台。
 *
 * 两边作业各自独立创建，没有天然对应关系，所以让老师亲手选一次（选错比猜错好）。
 * 照片按「中心账号 → 本地学生」匹配：绑过的按 id 认，没绑过的看班级内唯一同名；
 * 对不上的列在下面，老师挑一个学生点「绑定」就补上了，然后重新拉取即可。
 */
function CenterSyncDialog({assignment, localClassId, onClose, onPulled}) {
    const [binding, setBinding] = useState(null);
    const [loading, setLoading] = useState(true);
    const [classes, setClasses] = useState([]);
    const [centerClassId, setCenterClassId] = useState('');
    const [centerRows, setCenterRows] = useState([]);
    const [centerId, setCenterId] = useState('');
    const [members, setMembers] = useState([]);
    const [busy, setBusy] = useState(false);
    const [result, setResult] = useState(null);
    const [error, setError] = useState('');

    const loadClasses = useCallback(async () => {
        const list = await centerClasses();
        setClasses(list);
        setCenterClassId(list[0]?.id || '');
    }, []);

    useEffect(() => {
        let alive = true;
        (async () => {
            try {
                const found = await centerBinding(assignment.id);
                if (!alive) return;
                setBinding(found || null);
                if (!found) await loadClasses();
            } catch (err) {
                if (alive) setError(err.message);
            } finally {
                if (alive) setLoading(false);
            }
        })();
        return () => {
            alive = false;
        };
    }, [assignment.id, loadClasses]);

    useEffect(() => {
        if (!localClassId) return;
        classMembers(localClassId).then(setMembers).catch(() => setMembers([]));
    }, [localClassId]);

    useEffect(() => {
        if (!centerClassId) {
            setCenterRows([]);
            return undefined;
        }
        let alive = true;
        centerAssignments(centerClassId)
            .then((rows) => {
                if (!alive) return;
                setCenterRows(rows);
                setCenterId((current) => (rows.some((row) => row.id === current) ? current : rows[0]?.id || ''));
            })
            .catch((err) => {
                if (alive) setError(err.message);
            });
        return () => {
            alive = false;
        };
    }, [centerClassId]);

    const pull = async () => {
        setBusy(true);
        setError('');
        try {
            const summary = await centerPullAssets(assignment.id);
            setResult(summary);
            if (summary.pulled > 0) onPulled?.(summary);
            else if (!summary.unmatched.length) notify.info('没有新照片；学生还没传，或之前已全部拉过');
        } catch (err) {
            setError(err.message);
        } finally {
            setBusy(false);
        }
    };

    const bindAndPull = async () => {
        const chosen = centerRows.find((row) => row.id === centerId);
        if (!chosen) {
            notify.error('先选一份中心作业');
            return;
        }
        setBusy(true);
        setError('');
        try {
            await centerBindAssignment(assignment.id, chosen.id, chosen.title);
            setBinding({centerAssignmentId: chosen.id, centerAssignmentTitle: chosen.title});
            const summary = await centerPullAssets(assignment.id);
            setResult(summary);
            if (summary.pulled > 0) onPulled?.(summary);
        } catch (err) {
            setError(err.message);
        } finally {
            setBusy(false);
        }
    };

    const rebind = async () => {
        setBinding(null);
        setResult(null);
        setError('');
        try {
            await loadClasses();
        } catch (err) {
            setError(err.message);
        }
    };

    const bindStudent = async (centerUserId, studentId) => {
        if (!studentId) return;
        setBusy(true);
        setError('');
        try {
            await studentBindCenter(studentId, centerUserId);
            notify.success('已绑定；再点一次「拉取照片」就会落到这个学生名下');
            const fresh = await classMembers(localClassId);
            setMembers(fresh);
        } catch (err) {
            setError(err.message);
        } finally {
            setBusy(false);
        }
    };

    return (
        <Modal
            title="中心照片"
            subtitle={`${assignment.title} · 学生在家用手机提交的作答照片拉进本机批改台`}
            onClose={onClose}
            footer={
                <>
                    <button className="btn ghost" onClick={onClose}>
                        关闭
                    </button>
                    {binding ? (
                        <>
                            <button className="btn ghost" onClick={rebind} disabled={busy}>
                                重新绑定
                            </button>
                            <button className="btn primary" onClick={pull} disabled={busy}>
                                {busy ? '拉取中…' : '拉取照片'}
                            </button>
                        </>
                    ) : (
                        <button className="btn primary" onClick={bindAndPull} disabled={busy || !centerId}>
                            {busy ? '绑定并拉取…' : '绑定并拉取'}
                        </button>
                    )}
                </>
            }
        >
            {loading ? (
                <div className="loading">正在读中心作业…</div>
            ) : (
                <div className="form-grid">
                    {error && (
                        <p className="error-text" style={{gridColumn: '1 / -1'}}>
                            {error}
                        </p>
                    )}
                    {binding ? (
                        <div className="center-sync-note">
                            已绑定中心作业：<strong>{binding.centerAssignmentTitle || binding.centerAssignmentId}</strong>
                            。拉取只下载还没拉过的照片，同一张不会重复占空间；照片落到对应学生名下，
                            到「拍照批改」里切题给分即可。
                        </div>
                    ) : (
                        <>
                            <Field label="中心班级" wide>
                                <select
                                    value={centerClassId}
                                    onChange={(event) => setCenterClassId(event.target.value)}
                                    disabled={!classes.length}
                                >
                                    {classes.length === 0 && <option value="">中心里没有你的班级</option>}
                                    {classes.map((row) => (
                                        <option key={row.id} value={row.id}>
                                            {row.name}
                                            {row.term ? ` · ${row.term}` : ''}
                                        </option>
                                    ))}
                                </select>
                            </Field>
                            <Field label="对应的中心作业" wide hint="本地作业与中心作业各自独立创建，需要你选一次">
                                <select
                                    value={centerId}
                                    onChange={(event) => setCenterId(event.target.value)}
                                    disabled={!centerRows.length}
                                >
                                    {centerRows.length === 0 && <option value="">这个班级还没有作业</option>}
                                    {centerRows.map((row) => (
                                        <option key={row.id} value={row.id}>
                                            {row.title} · {row.submittedCount}/{row.studentCount} 人提交
                                        </option>
                                    ))}
                                </select>
                            </Field>
                        </>
                    )}
                    {result && (
                        <div className="center-sync-note">
                            本次新增 <strong>{result.pulled}</strong> 张照片
                            {result.skipped > 0 ? `，跳过 ${result.skipped} 张（之前拉过）` : ''}。
                            {result.unmatched.length > 0 && ' 下面这些中心账号没能自动匹配，绑定后再拉一次：'}
                        </div>
                    )}
                    {result && result.unmatched.length > 0 && (
                        <div className="center-sync-unmatched">
                            {result.unmatched.map((item) => (
                                <div key={item.centerUserId} className="center-sync-row">
                                    <span>
                                        <strong>{item.name || item.username || item.centerUserId}</strong>
                                        {item.name && item.username ? ` · ${item.username}` : ''}
                                        <em>（{item.photos} 张）</em>
                                    </span>
                                    <select
                                        value=""
                                        disabled={busy}
                                        onChange={(event) => bindStudent(item.centerUserId, event.target.value)}
                                    >
                                        <option value="">绑到本班学生…</option>
                                        {members
                                            .filter((member) => !member.centerUserId)
                                            .map((member) => (
                                                <option key={member.id} value={member.id}>
                                                    {member.name}
                                                </option>
                                            ))}
                                    </select>
                                </div>
                            ))}
                        </div>
                    )}
                </div>
            )}
        </Modal>
    );
}
