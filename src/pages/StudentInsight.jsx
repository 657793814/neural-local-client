import {useCallback, useEffect, useState} from 'react';
import {
    insightAdvice,
    insightAdviceGenerate,
    insightsStudent,
    scoreStudentExams,
    studentProgress,
    wrongBookList,
    wrongBookRemove,
    wrongBookReview,
} from '../lib/api.js';
import {Card, Empty, Pill, PageHead, confirmDialog, notify} from '../components/ui.jsx';
import {RichInline} from '../lib/Math.jsx';
import {cx} from '../lib/format.js';
import {buildReportHtml} from '../lib/report.js';
import {openInBrowser} from '../lib/paper/print.js';

/** 掌握度分档：与中心 85 线一致，五档颜色只表达深浅。 */
function toneOf(mastery) {
    if (mastery < 0) return 'none';
    if (mastery >= 85) return 'high';
    if (mastery >= 60) return 'mid';
    if (mastery >= 30) return 'low';
    return mastery > 0 ? 'weak' : 'empty';
}

const TONE_LABEL = {high: '已掌握', mid: '较稳', low: '待巩固', weak: '薄弱', empty: '未开始', none: '没有作答'};

const LEVEL_TONE = {
    EMPTY: 'muted',
    CRITICAL: 'danger',
    WATCH: 'warn',
    STEADY: 'info',
    SOLID: 'ok',
};

const STATE_PILL = {
    NEW: {text: '待复习', tone: 'warn'},
    REVIEWING: {text: '复习中', tone: 'info'},
    MASTERED: {text: '已掌握', tone: 'ok'},
};

function shortDate(raw) {
    if (!raw) return '';
    return String(raw).replace('T', ' ').slice(0, 16);
}

function agoText(raw) {
    if (!raw) return '还没有交过作业';
    const at = new Date(raw);
    if (Number.isNaN(at.getTime())) return '';
    const days = Math.floor((Date.now() - at.getTime()) / 86400000);
    if (days <= 0) return '今天练过';
    if (days === 1) return '昨天练过';
    if (days < 30) return `${days} 天前练过`;
    return `${Math.floor(days / 30)} 个月前练过`;
}

/**
 * 一个人的学情：从「学生」名单点进来，也能从班级矩阵下钻进来。
 *
 * 设计取向——先回答「他怎么样、在班里什么位置」，再回答「差在哪、怎么补」，
 * 最后才是明细。所以顺序是：位置 → 薄弱点 → 趋势 → 知识点 → 错题本 → 作业。
 */
export default function StudentInsight({studentId, onBack, onGo, onChanged}) {
    const [data, setData] = useState(null);
    const [book, setBook] = useState([]);
    const [bookState, setBookState] = useState('');
    const [error, setError] = useState('');
    const [busy, setBusy] = useState('');
    const [advice, setAdvice] = useState(null);
    const [exams, setExams] = useState([]);
    const [progress, setProgress] = useState(null);
    const [thinking, setThinking] = useState(false);

    const load = useCallback(async () => {
        try {
            const [detail, wrong, plan, examRows, timeline] = await Promise.all([
                insightsStudent(studentId),
                wrongBookList(studentId, bookState),
                // 诊断是锦上添花：模型挂了也不能把整页拖垮。
                insightAdvice(studentId).catch(() => null),
                scoreStudentExams(studentId).catch(() => []),
                studentProgress(studentId).catch(() => null),
            ]);
            setData(detail);
            setBook(wrong);
            setAdvice(plan);
            setExams(examRows);
            setProgress(timeline);
            setError('');
        } catch (err) {
            setError(err.message);
        }
    }, [studentId, bookState]);

    const exportReport = async () => {
        try {
            const html = buildReportHtml({insight: data, advice, progress});
            await openInBrowser(html, `${data.name}-学情周报`);
            notify.success('已交给系统浏览器，按 ⌘P 打印或另存为 PDF');
        } catch (err) {
            notify.error(err.message);
        }
    };

    const diagnose = async () => {
        setThinking(true);
        try {
            const next = await insightAdviceGenerate(studentId, true);
            setAdvice(next);
            notify.success(next.source === 'AI' ? '诊断已更新' : '模型不可用，已用本地规则生成');
        } catch (err) {
            notify.error(err.message);
        } finally {
            setThinking(false);
        }
    };

    useEffect(() => {
        load();
    }, [load]);

    const review = async (row, state) => {
        setBusy(row.id);
        try {
            await wrongBookReview({id: row.id, state, note: ''});
            await load();
            onChanged?.();
        } catch (err) {
            notify.error(err.message);
        } finally {
            setBusy('');
        }
    };

    const remove = async (row) => {
        const ok = await confirmDialog({
            title: '从错题本移出？',
            message: '只删掉这条错题记录，作答与成绩都还在。下次再做错会重新记进来。',
            confirmText: '移出',
            danger: true,
        });
        if (!ok) return;
        setBusy(row.id);
        try {
            await wrongBookRemove(row.id);
            await load();
            onChanged?.();
        } catch (err) {
            notify.error(err.message);
        } finally {
            setBusy('');
        }
    };

    if (error && !data) {
        return (
            <div className="page">
                <PageHead
                    title="学情"
                    actions={
                        <button className="btn ghost small" onClick={onBack}>
                            ← 返回名单
                        </button>
                    }
                />
                <p className="error-text">{error}</p>
            </div>
        );
    }

    if (!data) {
        return (
            <div className="page">
                <p className="muted small">正在算…</p>
            </div>
        );
    }

    const delta = data.delta || 0;
    const hasData = data.answered > 0;
    const weak = data.weak || [];
    const kps = data.kps || [];
    const recent = data.recent || [];

    return (
        <div className="page">
            <PageHead
                title={`${data.name} · 学情`}
                subtitle={[
                    data.studentNo ? `学号 ${data.studentNo}` : '',
                    data.grade || '未填年级',
                    data.classNames?.length ? data.classNames.join(' / ') : '不在任何班级',
                    hasData ? `已判 ${data.answered} 题，对 ${data.correct} 题` : '还没有已判分的作答',
                ]
                    .filter(Boolean)
                    .join(' · ')}
                actions={
                    <div className="inline-actions">
                        <button className="btn ghost small" onClick={onBack}>
                            ← 返回名单
                        </button>
                        <a className="btn ghost small" href="#/help/insights">
                            说明
                        </a>
                        <button className="btn ghost small" onClick={() => onGo?.('library')}>
                            去题库找同类题
                        </button>
                        <button className="btn primary small" onClick={exportReport}>
                            家长周报
                        </button>
                    </div>
                }
            />

            {/* 位置：一个人最想先知道的三件事——我现在几分、比班里如何、在进步还是退步 */}
            <div className="stu-hero">
                <div className="stu-score">
                    <span className="stu-score-label">掌握度</span>
                    <div className="stu-score-value">
                        <b>{hasData ? data.overall : '–'}</b>
                        <em>{hasData ? TONE_LABEL[toneOf(data.overall)] : '未开始'}</em>
                    </div>
                    <div className="stu-score-bar">
                        <i
                            className={cx('stu-score-fill', `t-${toneOf(hasData ? data.overall : -1)}`)}
                            style={{width: `${Math.max(hasData ? data.overall : 0, 1)}%`}}
                        />
                    </div>
                    <div className="stu-score-meta">
                        <span>得分率 {Math.round(data.scoreRate * 100)}%</span>
                        {data.pending > 0 && <span className="warn-text">{data.pending} 题待批</span>}
                        <span>{agoText(recent[0]?.submittedAt)}</span>
                    </div>
                </div>

                <div className="stu-compare">
                    <div className="stu-compare-row">
                        <span className="stu-compare-label">近两周</span>
                        <b className={cx('stu-delta', delta > 0 && 'up', delta < 0 && 'down')}>
                            {delta > 0 ? `+${delta}` : delta === 0 ? '持平' : delta}
                        </b>
                        <span className="muted small">掌握度变化，越新的作答权重越高</span>
                    </div>
                    <div className="stu-compare-row">
                        <span className="stu-compare-label">班内位置</span>
                        {data.rank && data.classSize ? (
                            <>
                                <b>
                                    第 {data.rank} / {data.classSize}
                                </b>
                                <span className="muted small">只有交过作业的同学参与排名</span>
                            </>
                        ) : (
                            <b className="muted">还没有可比的同学</b>
                        )}
                    </div>
                    <div className="stu-compare-row">
                        <span className="stu-compare-label">班级平均</span>
                        {data.classAvg != null ? (
                            <>
                                <b>{data.classAvg}</b>
                                <span className="muted small">
                                    {data.overall >= data.classAvg
                                        ? `高于班均 ${data.overall - data.classAvg} 分`
                                        : `低于班均 ${data.classAvg - data.overall} 分`}
                                </span>
                            </>
                        ) : (
                            <b className="muted">还没有班级数据</b>
                        )}
                    </div>
                </div>

                <div className="stu-counts">
                    <div>
                        <b>{data.wrongCount}</b>
                        <span>待复习错题</span>
                    </div>
                    <div>
                        <b>{weak.length}</b>
                        <span>薄弱知识点</span>
                    </div>
                    <div>
                        <b>{data.answered}</b>
                        <span>已判题数</span>
                    </div>
                </div>
            </div>

            {error && <p className="error-text">{error}</p>}

            <AdviceCard
                advice={advice}
                thinking={thinking}
                onDiagnose={diagnose}
                onGo={onGo}
            />

            <div className="stu-grid">
                <div className="stu-col">
                    <Card title="近 8 周掌握度" subtitle="每周把截至那周的作答重算一遍，看趋势不是看快照">
                        {!hasData ? (
                            <Empty title="还没有数据" hint="交一次作业、判完分之后，曲线就有了"/>
                        ) : (
                            <Trend points={data.trend}/>
                        )}
                    </Card>

                    <Card
                        title="作业与考试"
                        subtitle={progress?.note || '作业得分率与考试分数换成百分制后放在一起看'}
                        actions={
                            <button className="btn ghost small" onClick={() => onGo?.('scores')}>
                                去录成绩
                            </button>
                        }
                    >
                        {!progress || progress.points.length === 0 ? (
                            <Empty
                                title="还没有可比较的分数"
                                hint="交一次作业，或者在「成绩」里录一次考试"
                            />
                        ) : (
                            <ProgressChart points={progress.points}/>
                        )}
                    </Card>

                    <Card
                        title="知识点明细"
                        subtitle={kps.length > 0 ? '薄弱的排前面；没有作答的排在最后，不等于 0 分' : undefined}
                    >
                        {kps.length === 0 ? (
                            <Empty
                                title="还没有挂知识点的作答"
                                hint="作业里的题要先挂知识点，这里才能按点看掌握度"
                                action={
                                    <button className="btn ghost" onClick={() => onGo?.('library')}>
                                        去题库补知识点
                                    </button>
                                }
                            />
                        ) : (
                            <div className="kp-list">
                                {kps.map((kp) => (
                                    <div className="kp-row" key={kp.id}>
                                        <div className="kp-name" title={kp.name}>
                                            {kp.name}
                                            {kp.pending > 0 && <em>{kp.pending} 待批</em>}
                                        </div>
                                        <div className="kp-bar">
                                            <i
                                                className={cx('kp-fill', `t-${toneOf(kp.answered > 0 ? kp.mastery : -1)}`)}
                                                style={{width: `${Math.max(kp.answered > 0 ? kp.mastery : 0, 2)}%`}}
                                            />
                                        </div>
                                        <div className="kp-num">
                                            <b>{kp.answered > 0 ? kp.mastery : '–'}</b>
                                            <span>{kp.answered > 0 ? `${kp.correct}/${kp.answered} 对` : '未作答'}</span>
                                        </div>
                                    </div>
                                ))}
                            </div>
                        )}
                    </Card>
                </div>

                <div className="stu-col">
                    <Card
                        title="先补这几个"
                        subtitle="掌握度低于 70 的知识点，按薄弱程度排序"
                        actions={
                            weak.length > 0 ? (
                                <button className="btn ghost small" onClick={() => onGo?.('library')}>
                                    找同类题
                                </button>
                            ) : null
                        }
                    >
                        {weak.length === 0 ? (
                            <p className="muted small">
                                {hasData ? '没有明显薄弱的知识点，保持节奏就好。' : '有作答之后才谈得上薄弱点。'}
                            </p>
                        ) : (
                            <div className="weak-list">
                                {weak.map((kp) => (
                                    <div className="weak-row" key={kp.id}>
                                        <div className="weak-main">
                                            <span className="weak-name" title={kp.name}>
                                                {kp.name}
                                            </span>
                                            <span className="muted small">{kp.answered} 题已判</span>
                                        </div>
                                        <div className="weak-bar">
                                            <i style={{width: `${Math.max(kp.mastery, 2)}%`}}/>
                                        </div>
                                        <b>{kp.mastery}</b>
                                    </div>
                                ))}
                            </div>
                        )}
                    </Card>

                    <Card
                        title="错题本"
                        subtitle={data.wrongCount > 0 ? `待复习 ${data.wrongCount} 题` : '做错的题会自动进来'}
                    >
                        <div className="ins-filter">
                            {[
                                {key: '', label: '全部'},
                                {key: 'NEW', label: '待复习'},
                                {key: 'REVIEWING', label: '复习中'},
                                {key: 'MASTERED', label: '已掌握'},
                            ].map((item) => (
                                <button
                                    key={item.key || 'all'}
                                    className={cx('chip', bookState === item.key && 'active')}
                                    onClick={() => setBookState(item.key)}
                                >
                                    {item.label}
                                </button>
                            ))}
                        </div>
                        {book.length === 0 ? (
                            <p className="muted small">
                                {bookState ? '这个状态下没有错题。' : '还没有错题，做错的题会自动进这里。'}
                            </p>
                        ) : (
                            <div className="wb-list">
                                {book.map((row) => {
                                    const pill = STATE_PILL[row.state] || {text: row.state, tone: 'muted'};
                                    return (
                                        <div className="wb-row" key={row.id}>
                                            <div className="wb-head">
                                                <Pill tone={pill.tone}>{pill.text}</Pill>
                                                <span className="wb-stem"><RichInline text={row.excerpt}/></span>
                                            </div>
                                            <div className="wb-meta">
                                                <span>答案 <RichInline text={row.answer || '—'}/></span>
                                                <span>错 {row.wrongCount} 次</span>
                                                <span>复习 {row.reviewCount} 次</span>
                                                <span className="muted">{shortDate(row.lastWrongAt)}</span>
                                                {row.knowledge.length > 0 && (
                                                    <span className="muted">{row.knowledge.join(' · ')}</span>
                                                )}
                                            </div>
                                            {row.note && <div className="wb-note">{row.note}</div>}
                                            <div className="wb-acts">
                                                {row.state !== 'MASTERED' && (
                                                    <>
                                                        <button
                                                            className="btn ghost small"
                                                            disabled={busy === row.id}
                                                            onClick={() => review(row, 'REVIEWING')}
                                                        >
                                                            复习过
                                                        </button>
                                                        <button
                                                            className="btn ghost small"
                                                            disabled={busy === row.id}
                                                            onClick={() => review(row, 'MASTERED')}
                                                        >
                                                            已掌握
                                                        </button>
                                                    </>
                                                )}
                                                {row.state === 'MASTERED' && (
                                                    <button
                                                        className="btn ghost small"
                                                        disabled={busy === row.id}
                                                        onClick={() => review(row, 'NEW')}
                                                    >
                                                        再做一次
                                                    </button>
                                                )}
                                                <button
                                                    className="btn ghost small danger"
                                                    disabled={busy === row.id}
                                                    onClick={() => remove(row)}
                                                >
                                                    移出
                                                </button>
                                            </div>
                                        </div>
                                    );
                                })}
                            </div>
                        )}
                    </Card>

                    <Card
                        title="考试成绩"
                        subtitle={exams.length > 0 ? '最近 12 次；名次只算有分的同学' : '整卷总分在「成绩」里录'}
                        actions={
                            exams.length > 0 ? (
                                <button className="btn ghost small" onClick={() => onGo?.('scores')}>
                                    去成绩
                                </button>
                            ) : null
                        }
                    >
                        {exams.length === 0 ? (
                            <p className="muted small">还没有考试记录。录一次月考，这里会和掌握度一起看。</p>
                        ) : (
                            <div className="exam-list">
                                {exams.map((exam) => {
                                    const low = exam.score != null && exam.score / exam.fullScore < 0.6;
                                    return (
                                        <div className="exam-row" key={exam.examId}>
                                            <div className="exam-main">
                                                <span title={exam.title}>{exam.title}</span>
                                                <em>
                                                    {exam.takenAt} · {exam.className}
                                                </em>
                                            </div>
                                            <div className="exam-score">
                                                <b className={cx(low && 'low')}>
                                                    {exam.score == null ? '缺考' : Math.round(exam.score * 10) / 10}
                                                    <em> / {Math.round(exam.fullScore)}</em>
                                                </b>
                                                <em>
                                                    {exam.rank ? `第 ${exam.rank}/${exam.classSize}` : '没有名次'}
                                                    {exam.classAvg != null ? ` · 班均 ${exam.classAvg}` : ''}
                                                </em>
                                            </div>
                                        </div>
                                    );
                                })}
                            </div>
                        )}
                    </Card>

                    <Card title="最近的作业">
                        {recent.length === 0 ? (
                            <p className="muted small">还没有交过作业。</p>
                        ) : (
                            <div className="recent-list">
                                {recent.map((row) => (
                                    <div className="recent-row" key={row.assignmentId}>
                                        <div className="recent-main">
                                            <span>{row.title}</span>
                                            <em className="muted">{shortDate(row.submittedAt)}</em>
                                        </div>
                                        <div className="recent-score">
                                            <b>
                                                {Math.round(row.score * 10) / 10}/{row.maxScore}
                                            </b>
                                            {row.pending > 0 && <span>{row.pending} 待批</span>}
                                            {row.late && <span className="warn-text">迟交</span>}
                                        </div>
                                    </div>
                                ))}
                            </div>
                        )}
                    </Card>
                </div>
            </div>
        </div>
    );
}

/** 掌握度折线：纯 SVG，不引图表库；85 分线画实一点，一眼看到「掌握了没」。 */
function Trend({points}) {
    const width = 320;
    const height = 96;
    const pad = {left: 6, right: 6, top: 10, bottom: 18};
    const inner = width - pad.left - pad.right;
    const stepX = points.length > 1 ? inner / (points.length - 1) : 0;
    const y = (value) => pad.top + (1 - value / 100) * (height - pad.top - pad.bottom);
    const coords = points.map((point, index) => [pad.left + index * stepX, y(point.mastery)]);
    return (
        <div className="trend">
            <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label="近八周掌握度折线">
                {[0, 50, 85, 100].map((line) => (
                    <line
                        key={line}
                        x1={pad.left}
                        x2={width - pad.right}
                        y1={y(line)}
                        y2={y(line)}
                        className={cx('trend-grid', line === 85 && 'strong')}
                    />
                ))}
                <polyline
                    className="trend-line"
                    points={coords.map(([x, value]) => `${x},${value}`).join(' ')}
                />
                {coords.map(([x, value], index) => (
                    <circle
                        key={index}
                        className="trend-dot"
                        cx={x}
                        cy={value}
                        r={index === coords.length - 1 ? 3.2 : 2.2}
                    />
                ))}
                <text className="trend-tick" x={pad.left} y={height - 4}>
                    {points[0].week}
                </text>
                <text className="trend-tick" x={width - pad.right} y={height - 4} textAnchor="end">
                    本周 · {points[points.length - 1].mastery}
                </text>
            </svg>
        </div>
    );
}

/**
 * 作业 + 考试混排：圆点是作业（按题汇总），方块是考试（整卷总分）。
 * 统一百分制之后才敢放在同一根轴上；60 分线当及格参考，别让两条线各说各的。
 */
function ProgressChart({points}) {
    const width = 360;
    const height = 118;
    const pad = {left: 24, right: 10, top: 12, bottom: 18};
    const inner = width - pad.left - pad.right;
    const stepX = points.length > 1 ? inner / (points.length - 1) : 0;
    const y = (value) => pad.top + (1 - Math.min(Math.max(value, 0), 100) / 100) * (height - pad.top - pad.bottom);
    const coords = points.map((point, index) => [pad.left + index * stepX, y(point.percent)]);
    const last = points[points.length - 1];

    return (
        <div className="trend prog">
            <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-label="作业与考试得分率趋势">
                {[0, 60, 100].map((line) => (
                    <line
                        key={line}
                        x1={pad.left}
                        x2={width - pad.right}
                        y1={y(line)}
                        y2={y(line)}
                        className={cx('trend-grid', line === 60 && 'strong')}
                    />
                ))}
                <polyline
                    className="prog-line"
                    points={coords.map(([x, value]) => `${x},${value}`).join(' ')}
                />
                {points.map((point, index) => {
                    const [x, value] = coords[index];
                    const tip = `${point.at} ${point.title}：${point.percent}%`;
                    return point.kind === 'ASSIGNMENT' ? (
                        <circle key={index} cx={x} cy={value} r={2.6} className="prog-dot">
                            <title>{tip}</title>
                        </circle>
                    ) : (
                        <rect
                            key={index}
                            x={x - 3}
                            y={value - 3}
                            width={6}
                            height={6}
                            className="prog-square"
                        >
                            <title>{tip}</title>
                        </rect>
                    );
                })}
                <text className="trend-tick" x={pad.left} y={height - 4}>
                    {points[0].at.slice(5)}
                </text>
                <text className="trend-tick" x={width - pad.right} y={height - 4} textAnchor="end">
                    最近 {Math.round(last.percent)}% · {last.at.slice(5)}
                </text>
            </svg>
            <div className="prog-legend">
                <span>
                    <i className="prog-key dot"/>作业
                </span>
                <span>
                    <i className="prog-key square"/>考试
                </span>
                <span className="muted">
                    {points.length} 次 · 及格线 60
                </span>
            </div>
        </div>
    );
}

/**
 * 学情诊断卡：结论 → 优势 / 问题 / 本周安排 → 给家长的话。
 * 规则版和 AI 版共用同一套结构，所以这里不需要判断来源，只在页脚说清是谁给的结论。
 * 「依据」默认折起来：结论要能追溯，但不能让老师先读十行数字。
 */
function AdviceCard({advice, thinking, onDiagnose, onGo}) {
    const sourceText =
        advice?.source === 'AI'
            ? 'AI 综合掌握度、错题与考试分数给出'
            : '本地规则生成：不联网也能用，配了模型后可让模型补「为什么」';

    return (
        <Card
            title="学情诊断与本周安排"
            subtitle={advice ? sourceText : '正在读取…'}
            actions={
                <div className="inline-actions">
                    {advice?.stale && <span className="ad-stale">数据已更新，建议重新诊断</span>}
                    <button className="btn primary small" onClick={onDiagnose} disabled={thinking}>
                        {thinking ? '诊断中…' : advice?.source === 'AI' ? '重新诊断' : 'AI 诊断'}
                    </button>
                </div>
            }
        >
            {!advice ? (
                <p className="ad-empty">正在准备诊断…</p>
            ) : (
                <>
                    <div className="ad-head">
                        <Pill tone={LEVEL_TONE[advice.level] || 'muted'}>{advice.levelLabel}</Pill>
                        <h3 className="ad-headline">{advice.headline}</h3>
                    </div>

                    <div className="ad-cols">
                        <section className="ad-sec">
                            <h4>已经站住的地方</h4>
                            {advice.strengths.length === 0 ? (
                                <p className="ad-empty">
                                    {advice.level === 'EMPTY'
                                        ? '还没有作答数据，先交一次作业。'
                                        : '还没有稳定在 85 分以上的知识点，先把薄弱点补上来。'}
                                </p>
                            ) : (
                                <ul>
                                    {advice.strengths.map((item) => (
                                        <li key={item.point}>
                                            <b>{item.point}</b>
                                            {item.evidence && <em> · {item.evidence}</em>}
                                        </li>
                                    ))}
                                </ul>
                            )}
                        </section>

                        <section className="ad-sec">
                            <h4>卡在哪、怎么补</h4>
                            <ul>
                                {advice.issues.map((item) => (
                                    <li
                                        key={`${item.kp}-${item.action.slice(0, 8)}`}
                                        className={cx('ad-issue', advice.level !== 'SOLID' && 'weak')}
                                    >
                                        <strong>{item.kp}</strong>
                                        {item.why && <div>{item.why}</div>}
                                        <div>
                                            <em>怎么做：</em>
                                            {item.action}
                                        </div>
                                    </li>
                                ))}
                            </ul>
                        </section>

                        <section className="ad-sec">
                            <h4>
                                本周安排 · 共 {advice.plan.totalMinutes} 分钟
                                {advice.plan.focus.length > 0 && ` · 集中 ${advice.plan.focus.join('、')}`}
                            </h4>
                            <ul>
                                {advice.plan.days.map((day, index) => (
                                    <li className="ad-day" key={`${day.day}-${index}`}>
                                        <span>{day.day}</span>
                                        <b>{day.task}</b>
                                        <em>{day.minutes} 分</em>
                                    </li>
                                ))}
                            </ul>
                        </section>
                    </div>

                    {advice.parentNote && (
                        <div className="ad-parent">
                            <b>给家长的一句话</b>
                            {advice.parentNote}
                        </div>
                    )}

                    {advice.facts.length > 0 && (
                        <details className="ad-facts">
                            <summary>
                                依据 {advice.facts.length} 条 · {shortDate(advice.createdAt)}
                            </summary>
                            <ul>
                                {advice.facts.map((line) => (
                                    <li key={line}>{line}</li>
                                ))}
                            </ul>
                        </details>
                    )}

                    {advice.plan.focus.length > 0 && (
                        <div className="inline-actions" style={{marginTop: 10}}>
                            <button className="btn ghost small" onClick={() => onGo?.('library')}>
                                按这些知识点找题
                            </button>
                            <button className="btn ghost small" onClick={() => onGo?.('scores')}>
                                录一次考试成绩
                            </button>
                        </div>
                    )}
                </>
            )}
        </Card>
    );
}
