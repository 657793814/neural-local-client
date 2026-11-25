import {useCallback, useEffect, useMemo, useState} from 'react';
import {classList, insightsClass, insightsRoster} from '../lib/api.js';
import {Card, Empty, PageHead, Tabs} from '../components/ui.jsx';
import {CountUp} from '../components/effects.jsx';
import {cx} from '../lib/format.js';
import {RichInline} from '../lib/Math.jsx';
import StudentInsight from './StudentInsight.jsx';

/** 掌握度分档：与中心的 85 线一致，颜色只分五档，避免一眼看不出深浅。 */
function toneOf(mastery) {
    if (mastery < 0) return 'none';
    if (mastery >= 85) return 'high';
    if (mastery >= 60) return 'mid';
    if (mastery >= 30) return 'low';
    return mastery > 0 ? 'weak' : 'empty';
}

const TONE_LABEL = {high: '已掌握', mid: '较稳', low: '待巩固', weak: '薄弱', empty: '未开始', none: '没有作答'};

const ROSTER_SORTS = [
    {key: 'weak', label: '薄弱优先'},
    {key: 'drop', label: '退步最多'},
    {key: 'due', label: '待复习多'},
    {key: 'idle', label: '最久没练'},
];

function shortDate(raw) {
    if (!raw) return '';
    return String(raw).replace('T', ' ').slice(0, 10);
}

/**
 * 学情两个视角：
 * - 班级：横向看一个班的整体与知识点矩阵；
 * - 学生：一个人一个人地看（跨班可筛），点进去是个人整页。
 *
 * 路由：`#/insights` 班级、`#/insights/student` 名单、`#/insights/student/{id}` 个人。
 * 把视角放进 URL，是为了从名单点进个人再返回时还停在「学生」这个视角上。
 */
export default function Insights({tab = 'class', studentId = '', onGo, onChanged}) {
    if (studentId) {
        return (
            <StudentInsight
                studentId={studentId}
                onBack={() => onGo?.('insights/student')}
                onGo={onGo}
                onChanged={onChanged}
            />
        );
    }
    return (
        <div className="page">
            <PageHead
                title="学情"
                subtitle="掌握度 = 按时间衰减加权的正确率（两周半衰）；未批改的主观题不计入"
                actions={
                    <div className="inline-actions">
                        <a className="btn ghost small" href="#/help/insights">
                            说明
                        </a>
                    </div>
                }
            />
            <Tabs
                value={tab}
                onChange={(next) => onGo?.(next === 'student' ? 'insights/student' : 'insights')}
                items={[
                    {value: 'class', label: '班级'},
                    {value: 'student', label: '学生'},
                ]}
            />
            {tab === 'student' ? (
                <RosterView onGo={onGo}/>
            ) : (
                <ClassView onGo={onGo}/>
            )}
        </div>
    );
}

// ---------------------------------------------------------------------------
// 班级视角
// ---------------------------------------------------------------------------

function ClassView({onGo}) {
    const [classes, setClasses] = useState([]);
    const [classId, setClassId] = useState('');
    const [data, setData] = useState(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState('');

    const loadClasses = useCallback(async () => {
        try {
            const list = await classList();
            setClasses(list);
            setClassId((current) => {
                if (current && list.some((row) => row.id === current)) return current;
                return list[0]?.id || '';
            });
        } catch (err) {
            setError(err.message);
        }
    }, []);

    const loadClass = useCallback(async (id) => {
        if (!id) {
            setData(null);
            setLoading(false);
            return;
        }
        setLoading(true);
        try {
            setData(await insightsClass(id));
            setError('');
        } catch (err) {
            setError(err.message);
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => {
        loadClasses();
    }, [loadClasses]);

    useEffect(() => {
        loadClass(classId);
    }, [classId, loadClass]);

    const refresh = () => loadClass(classId);

    const summary = useMemo(() => {
        if (!data) return null;
        const active = data.students.filter((row) => row.answered > 0);
        const overall = active.length
            ? Math.round(active.reduce((sum, row) => sum + row.mastery, 0) / active.length)
            : 0;
        return {
            overall,
            students: data.students.length,
            active: active.length,
            weak: active.filter((row) => row.mastery > 0 && row.mastery < 40).length,
            mastered: active.filter((row) => row.mastery >= 85).length,
        };
    }, [data]);

    if (classes.length === 0 && !loading) {
        return (
            <Card>
                <Empty
                    title="还没有班级"
                    hint="先在「班级」里建班、加学生、布置一次作业，这里才会长出掌握度与错题本"
                    action={
                        <button className="btn primary" onClick={() => onGo?.('classes')}>
                            去班级
                        </button>
                    }
                />
            </Card>
        );
    }

    return (
        <>
            {error && <p className="error-text">{error}</p>}

            <div className="ins-bar">
                {classes.map((row) => (
                    <button
                        key={row.id}
                        className={cx('ins-chip', row.id === classId && 'on')}
                        onClick={() => setClassId(row.id)}
                    >
                        {row.name}
                        <span className="muted">{row.memberCount ? `${row.memberCount} 人` : ''}</span>
                    </button>
                ))}
                <span className="grow"/>
                <button className="btn ghost small" onClick={refresh} disabled={loading}>
                    重新统计
                </button>
            </div>

            {loading && !data && <p className="muted small">正在算…</p>}

            {data && summary && (
                <>
                    <div className="ins-kpis">
                        <Kpi label="班级掌握度" value={summary.overall} unit="分" hint={`${summary.active} 人交过作业`}/>
                        <Kpi label="得分率" value={Math.round(data.scoreRate * 100)} unit="%" hint={`已判 ${data.gradedTotal} 题`}/>
                        <Kpi label="已掌握" value={summary.mastered} unit="人" hint="掌握度 ≥ 85"/>
                        <Kpi label="薄弱" value={summary.weak} unit="人" hint="掌握度 1–39，优先补" tone="warn"/>
                        <Kpi
                            label="待批改"
                            value={data.pendingGrade}
                            unit="题"
                            tone={data.pendingGrade > 0 ? 'warn' : undefined}
                            hint="不计入掌握度"
                            action={
                                data.pendingGrade > 0 ? (
                                    <button className="btn ghost small" onClick={() => onGo?.(`assignments/${classId}`)}>
                                        去批改
                                    </button>
                                ) : null
                            }
                        />
                    </div>

                    <Card
                        title="知识点矩阵"
                        subtitle={
                            data.kps.length === 0
                                ? '这个班的题目还没挂知识点，先去题库补上，矩阵才有列'
                                : '行是学生（薄弱的在上），列是作业覆盖到的知识点；点姓名看个人学情'
                        }
                    >
                        {data.kps.length === 0 ? (
                            <Empty
                                title="还没有知识点列"
                                hint="布置作业时选带知识点的题，或到题库里给题目补挂知识点"
                                action={
                                    <button className="btn ghost" onClick={() => onGo?.('library')}>
                                        去题库
                                    </button>
                                }
                            />
                        ) : (
                            <div className="mx-wrap">
                                <table className="mx-table">
                                    <thead>
                                        <tr>
                                            <th className="mx-name">学生</th>
                                            {data.kps.map((kp) => (
                                                <th key={kp.id} title={`${kp.name} · 已判 ${kp.answered} 题`}>
                                                    <span>{kp.name}</span>
                                                    {kp.pending > 0 && <em>{kp.pending} 待批</em>}
                                                </th>
                                            ))}
                                            <th className="mx-total">掌握度</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {data.students.map((row) => (
                                            <tr key={row.id}>
                                                <td className="mx-name">
                                                    <button
                                                        className="mx-open"
                                                        onClick={() => onGo?.(`insights/student/${row.id}`)}
                                                    >
                                                        {row.name}
                                                    </button>
                                                    {row.studentNo && <em className="mx-no">{row.studentNo}</em>}
                                                </td>
                                                {row.cells.map((value, index) => (
                                                    <td key={index}>
                                                        <span
                                                            className={cx('mx-cell', `t-${toneOf(value)}`)}
                                                            title={`${data.kps[index].name} · ${
                                                                value < 0 ? '没有作答' : `${value} 分（${TONE_LABEL[toneOf(value)]}）`
                                                            }`}
                                                        >
                                                            {value < 0 ? '–' : value}
                                                        </span>
                                                    </td>
                                                ))}
                                                <td className="mx-total">
                                                    <b>{row.answered > 0 ? row.mastery : '–'}</b>
                                                    <span className={cx('mx-dot', `t-${toneOf(row.answered > 0 ? row.mastery : -1)}`)}/>
                                                    {row.pending > 0 && <em>{row.pending} 待批</em>}
                                                </td>
                                            </tr>
                                        ))}
                                    </tbody>
                                </table>
                            </div>
                        )}
                        <div className="mx-legend">
                            {['high', 'mid', 'low', 'weak', 'none'].map((tone) => (
                                <span key={tone}>
                                    <i className={cx('mx-cell', `t-${tone}`)}/>
                                    {TONE_LABEL[tone]}
                                </span>
                            ))}
                        </div>
                    </Card>

                    <Card title="共性错题" subtitle="同一个班做错人次最多的题，优先讲这些">
                        {data.commonWrong.length === 0 ? (
                            <Empty title="还没有做错的题" hint="学生交过作业、判过分之后，这里会自动排出来"/>
                        ) : (
                            <div className="cw-list">
                                {data.commonWrong.map((row, index) => (
                                    <div className="cw-row" key={row.questionId}>
                                        <span className="cw-rank">{index + 1}</span>
                                        <div className="cw-main">
                                            <div className="cw-stem"><RichInline text={row.excerpt}/></div>
                                            <div className="cw-meta">
                                                {row.knowledge.length > 0 && <span>{row.knowledge.join(' · ')}</span>}
                                                <span className="muted">做错 {row.wrong} 人次 / 共 {row.total} 人作答</span>
                                            </div>
                                        </div>
                                        <div className="cw-rate">
                                            <b>{row.rate}%</b>
                                            <span>错误率</span>
                                        </div>
                                    </div>
                                ))}
                            </div>
                        )}
                    </Card>
                </>
            )}
        </>
    );
}

function Kpi({label, value, unit, hint, tone, action}) {
    return (
        <div className={cx('ins-kpi', tone && `t-${tone}`)}>
            <span className="ins-kpi-label">{label}</span>
            <div className="ins-kpi-value">
                <b><CountUp value={value}/></b>
                <em>{unit}</em>
            </div>
            <span className="ins-kpi-hint">{hint}</span>
            {action && <div className="ins-kpi-act">{action}</div>}
        </div>
    );
}

// ---------------------------------------------------------------------------
// 学生视角
// ---------------------------------------------------------------------------

function RosterView({onGo}) {
    const [data, setData] = useState(null);
    const [classId, setClassId] = useState('');
    const [keyword, setKeyword] = useState('');
    const [sort, setSort] = useState('weak');
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState('');

    useEffect(() => {
        let alive = true;
        // 打字时别每敲一下就查一次库；空搜索立刻出结果。
        const timer = setTimeout(
            async () => {
                setLoading(true);
                try {
                    const next = await insightsRoster(classId, keyword.trim());
                    if (!alive) return;
                    setData(next);
                    setError('');
                } catch (err) {
                    if (alive) setError(err.message);
                } finally {
                    if (alive) setLoading(false);
                }
            },
            keyword ? 260 : 0,
        );
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [classId, keyword]);

    const rows = useMemo(() => {
        const list = [...(data?.students || [])];
        if (sort === 'drop') return list.sort((a, b) => a.delta - b.delta || a.mastery - b.mastery);
        if (sort === 'due') return list.sort((a, b) => b.dueReview - a.dueReview || a.mastery - b.mastery);
        if (sort === 'idle') {
            return list.sort((a, b) => (a.lastAt || '').localeCompare(b.lastAt || '') || a.mastery - b.mastery);
        }
        // 「薄弱优先」后端已经排好：有数据的按掌握度升序，没数据的在最后。
        return list;
    }, [data, sort]);

    const summary = data?.summary;

    if (error && !data) {
        return <p className="error-text">{error}</p>;
    }

    if (data && summary && summary.total === 0 && !keyword && !classId) {
        return (
            <Card>
                <Empty
                    title="还没有学生"
                    hint="先在「班级」里加学生（或批量导入名单），再布置一次作业，个人学情才有东西可看"
                    action={
                        <button className="btn primary" onClick={() => onGo?.('classes')}>
                            去班级
                        </button>
                    }
                />
            </Card>
        );
    }

    return (
        <>
            {error && <p className="error-text">{error}</p>}

            {summary && (
                <div className="ins-kpis">
                    <Kpi label="名单人数" value={summary.total} unit="人" hint={classId ? '当前班级' : '全部班级'}/>
                    <Kpi label="有数据" value={summary.active} unit="人" hint="交过作业并判过分"/>
                    <Kpi label="薄弱" value={summary.weak} unit="人" hint="掌握度 1–39，优先补" tone={summary.weak > 0 ? 'warn' : undefined}/>
                    <Kpi label="已掌握" value={summary.mastered} unit="人" hint="掌握度 ≥ 85"/>
                    <Kpi
                        label="待复习"
                        value={summary.dueReview}
                        unit="题"
                        hint="错题到期没复习"
                        tone={summary.dueReview > 0 ? 'warn' : undefined}
                    />
                    <Kpi
                        label="久未练"
                        value={summary.idle}
                        unit="人"
                        hint="超过两周没做题"
                        tone={summary.idle > 0 ? 'warn' : undefined}
                    />
                </div>
            )}

            <Card
                title="学生名单"
                subtitle="横向比人：谁掉队、谁在退步、谁该复习；点一行看他的完整学情"
                actions={
                    <div className="roster-filter">
                        <input
                            className="roster-search"
                            value={keyword}
                            onChange={(event) => setKeyword(event.target.value)}
                            placeholder="搜姓名 / 学号 / 年级"
                        />
                    </div>
                }
            >
                <div className="ins-bar">
                    <button className={cx('ins-chip', !classId && 'on')} onClick={() => setClassId('')}>
                        全部
                        <span className="muted">{data ? data.classes.reduce((sum, row) => sum + row.count, 0) : ''}</span>
                    </button>
                    {(data?.classes || []).map((row) => (
                        <button
                            key={row.id}
                            className={cx('ins-chip', row.id === classId && 'on')}
                            onClick={() => setClassId(row.id)}
                        >
                            {row.name}
                            <span className="muted">{row.count} 人</span>
                        </button>
                    ))}
                </div>

                <div className="roster-sorts">
                    <span className="muted small">排序</span>
                    {ROSTER_SORTS.map((item) => (
                        <button
                            key={item.key}
                            className={cx('chip', sort === item.key && 'active')}
                            onClick={() => setSort(item.key)}
                        >
                            {item.label}
                        </button>
                    ))}
                    <span className="grow"/>
                    {loading && <span className="muted small">刷新中…</span>}
                </div>

                {!data && loading ? (
                    <p className="muted small">正在算…</p>
                ) : rows.length === 0 ? (
                    <Empty
                        title={keyword ? '没搜到人' : '这个班还没有学生'}
                        hint={keyword ? '换个名字，或清空搜索框看看全部' : '到「班级 → 学生」里加人'}
                    />
                ) : (
                    <div className="roster-list">
                        <div className="roster-head">
                            <span>学生</span>
                            <span>掌握度</span>
                            <span>分数 / 变化</span>
                            <span>薄弱知识点</span>
                            <span>提醒</span>
                        </div>
                        {rows.map((row) => (
                            <button
                                className="roster-row"
                                key={row.id}
                                onClick={() => onGo?.(`insights/student/${row.id}`)}
                            >
                                <span className="roster-who">
                                    <b>{row.name}</b>
                                    <span className="roster-sub">
                                        {row.studentNo && <em className="roster-no">{row.studentNo}</em>}
                                        {row.grade && <em>{row.grade}</em>}
                                        {row.classNames.map((name) => (
                                            <i key={name}>{name}</i>
                                        ))}
                                        {row.answered === 0 && <i className="empty">未开始</i>}
                                    </span>
                                </span>
                                <span className="roster-bar">
                                    <i
                                        className={cx('roster-fill', `t-${toneOf(row.answered > 0 ? row.mastery : -1)}`)}
                                        style={{width: `${Math.max(row.answered > 0 ? row.mastery : 0, 1)}%`}}
                                    />
                                </span>
                                <span className="roster-score">
                                    <b>{row.answered > 0 ? row.mastery : '–'}</b>
                                    {row.answered > 0 && (
                                        <em
                                            className={cx(
                                                'roster-delta',
                                                row.delta > 0 && 'up',
                                                row.delta < 0 && 'down',
                                            )}
                                        >
                                            {row.delta > 0 ? `+${row.delta}` : row.delta === 0 ? '持平' : row.delta}
                                        </em>
                                    )}
                                </span>
                                <span className="roster-weak">
                                    {row.weak.length === 0 ? (
                                        <i className="muted small">
                                            {row.answered > 0 ? '没有明显薄弱' : '还没有作答'}
                                        </i>
                                    ) : (
                                        row.weak.map((kp) => (
                                            <i key={kp.id} title={`${kp.name} · ${kp.mastery} 分`}>
                                                {kp.name}
                                            </i>
                                        ))
                                    )}
                                </span>
                                <span className="roster-meta">
                                    {row.dueReview > 0 && <i className="warn-text">待复习 {row.dueReview}</i>}
                                    {row.pending > 0 && <i>{row.pending} 待批</i>}
                                    <i className="muted">{row.lastAt ? shortDate(row.lastAt) : '未交'}</i>
                                </span>
                            </button>
                        ))}
                    </div>
                )}
            </Card>
        </>
    );
}
