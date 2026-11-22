import {useCallback, useEffect, useMemo, useRef, useState} from 'react';
import {questionStats} from '../lib/api.js';
import {Card, Empty, Loading, PageHead, Pill} from '../components/ui.jsx';

/**
 * 题库统计：先看结构与覆盖，再看「缺什么」。
 *
 * 题库大概率是单学科的，所以口径跟着顶部的学科选择走：
 * 全库 → 按学科看分布；选一个学科 → 单学科口径（章节覆盖、章节分布，
 * 建议不再按学科点名）；选多个学科 → 只算选中的学科。
 * 这一页的落点是**补题建议**，图表都做得很轻（横向条形 + 月度柱），
 * 每条建议都带一个能直接跳过去的按钮。数字口径与题库列表一致。
 */

const LEVELS = {
    high: {label: '优先', tone: 'danger'},
    mid: {label: '建议', tone: 'warn'},
    low: {label: '整理', tone: 'muted'},
};

const TYPE_ORDER = ['SINGLE', 'MULTI', 'FILL', 'SOLVED'];

function percent(part, total) {
    if (!total) return 0;
    return Math.round((part * 100) / total);
}

function Kpi({label, value, unit = '', hint}) {
    return (
        <div className="stat-kpi">
            <span className="stat-kpi-label">{label}</span>
            <span className="stat-kpi-value">
                {value}
                {unit && <em>{unit}</em>}
            </span>
            {hint && <span className="stat-kpi-hint">{hint}</span>}
        </div>
    );
}

/** 横向条形：学科 / 章节 / 题型 / 难度都用同一套，读起来不用重新适应。 */
function BucketBars({items, total, onPick, hint}) {
    const max = Math.max(1, ...items.map((item) => item.count));
    if (!items.length) return <Empty title="还没有数据" hint={hint}/>;
    return (
        <div className="stat-bars">
            {items.map((item) => (
                <button
                    type="button"
                    key={item.key}
                    className={`stat-bar-row ${item.count === 0 ? 'empty' : ''}`}
                    onClick={() => onPick?.(item)}
                    disabled={!onPick || !item.count}
                    title={onPick ? `查看「${item.label}」的题目` : undefined}
                >
                    <span className="stat-bar-label">{item.label}</span>
                    <span className="stat-bar-track">
                        <span className="stat-bar-fill" style={{width: `${Math.max(2, (item.count / max) * 100)}%`}}/>
                    </span>
                    <span className="stat-bar-value">
                        {item.count}
                        {total > 0 && <em>{percent(item.count, total)}%</em>}
                    </span>
                </button>
            ))}
        </div>
    );
}

function MonthChart({months}) {
    const max = Math.max(1, ...months.map((month) => month.count));
    if (!months.length) return null;
    return (
        <div className="stat-months">
            {months.map((month) => (
                <div className="stat-month" key={month.key} title={`${month.key} 新增 ${month.count} 道`}>
                    <span className="stat-month-count">{month.count}</span>
                    <span className="stat-month-track">
                        <span
                            className="stat-month-bar"
                            style={{height: `${month.count === 0 ? 0 : Math.max(6, (month.count / max) * 100)}%`}}
                        />
                    </span>
                    <span className="stat-month-label">{month.label}</span>
                </div>
            ))}
        </div>
    );
}

export default function Stats({onGo, loader = questionStats}) {
    const [data, setData] = useState(null);
    const [error, setError] = useState('');
    const [loading, setLoading] = useState(true);
    // 选中的学科 id；空数组 = 全库。可以多选。
    const [subjects, setSubjects] = useState([]);
    // 首次全量加载时缓存的学科清单（含各科总题数），选择器不跟随过滤变。
    const [allSubjects, setAllSubjects] = useState([]);
    const subjectsRef = useRef(subjects);
    subjectsRef.current = subjects;

    const load = useCallback(async () => {
        setLoading(true);
        try {
            const picked = subjectsRef.current;
            const result = await loader(picked);
            setData(result);
            // 只在全量视图下刷新选择器上的学科清单与总题数。
            if (picked.length === 0) {
                setAllSubjects(result.bySubject || []);
            }
            setError('');
        } catch (failure) {
            setError(failure.message || '统计失败');
        } finally {
            setLoading(false);
        }
    }, [loader]);

    useEffect(() => {
        load();
    }, [load, subjects]);

    const toggle = (key) => {
        setSubjects((current) =>
            current.includes(key) ? current.filter((id) => id !== key) : [...current, key],
        );
    };

    const single = Boolean(data?.singleSubject);
    const types = useMemo(() => {
        if (!data) return [];
        // 按固定顺序展示，没题的题型也留着 0，缺口一眼可见。
        return TYPE_ORDER.map((kind) => data.byType.find((item) => item.key === kind)).filter(Boolean);
    }, [data]);

    if (error && !data) {
        return (
            <div className="page">
                <PageHead title="题库统计"/>
                <Empty title="统计没跑起来" hint={error} action={<button className="btn ghost" onClick={load}>重试</button>}/>
            </div>
        );
    }

    if (!data) {
        return (
            <div className="page">
                <PageHead title="题库统计"/>
                <Loading text="正在统计题库…"/>
            </div>
        );
    }

    const goLibrary = (subjectId) => onGo?.(subjectId ? `library?subject=${encodeURIComponent(subjectId)}` : 'library');
    // 去出题：科目必带，难度 / 题型建议里有就带，出题页直接预选好。
    const goGenerate = (item) => {
        const params = new URLSearchParams();
        if (item.subjectId) params.set('subject', item.subjectId);
        if (item.difficulty) params.set('difficulty', String(item.difficulty));
        if (item.types?.length) params.set('types', item.types.join(','));
        onGo?.(params.toString() ? `generate?${params.toString()}` : 'generate');
    };

    const pointsRate = percent(data.pointsCovered, data.pointsTotal);
    const chaptersRate = percent(data.chaptersCovered, data.chaptersTotal);
    // 学科分布：全库时就是 bySubject；多选时只画选中的那几个。
    const subjectBars = subjects.length
        ? data.bySubject.filter((bucket) => subjects.includes(bucket.key))
        : data.bySubject;

    const subtitle = single
        ? '单学科口径：按章节、题型、难度看结构，并按缺口给出补题建议'
        : subjects.length
            ? '已按选中的学科过滤：题型、难度与建议都只算这几个学科'
            : '按学科、题型、难度看结构，并按缺口给出补题建议；点学科可切换口径';

    return (
        <div className="page">
            <PageHead
                title="题库统计"
                subtitle={subtitle}
                actions={
                    <div className="inline-actions">
                        <button className="btn ghost small" onClick={load} disabled={loading}>
                            {loading ? '刷新中…' : '刷新'}
                        </button>
                    </div>
                }
            />

            {/* 学科选择：默认全库；点学科进单学科口径，再点几个就是多学科范围 */}
            <div className="chip-row stat-subjects">
                <button
                    type="button"
                    className={`chip ${subjects.length === 0 ? 'active' : ''}`}
                    onClick={() => setSubjects([])}
                >
                    全部学科
                </button>
                {allSubjects.map((subject) => (
                    <button
                        type="button"
                        key={subject.key}
                        className={`chip ${subjects.includes(subject.key) ? 'active' : ''}`}
                        onClick={() => toggle(subject.key)}
                    >
                        {subject.label}
                        <span className="chip-count">{subject.count}</span>
                    </button>
                ))}
            </div>

            <div className="stat-kpis">
                <Kpi
                    label="题目总数"
                    value={data.total}
                    unit="道"
                    hint={`已发布 ${data.published} · 草稿 ${data.draft}`}
                />
                {single ? (
                    <Kpi
                        label="章节覆盖"
                        value={`${chaptersRate}%`}
                        hint={`${data.chaptersCovered} / ${data.chaptersTotal} 章有题（按挂知识点归章）`}
                    />
                ) : (
                    <Kpi
                        label="学科覆盖"
                        value={`${data.subjectsCovered}/${data.subjectsTotal}`}
                        hint={data.subjectsCovered === data.subjectsTotal ? '选中的学科都有题' : '还有学科是空的'}
                    />
                )}
                <Kpi
                    label="知识点覆盖"
                    value={`${pointsRate}%`}
                    hint={`${data.pointsCovered} / ${data.pointsTotal} 个考点有题`}
                />
                <Kpi
                    label="待复核"
                    value={data.needsReview}
                    unit="道"
                    hint={data.needsReview ? '答案没人工确认过' : '都确认过了'}
                />
                <Kpi label="近 30 天新增" value={data.recent30} unit="道"/>
            </div>

            {single ? (
                <Card title="章节分布" subtitle="题目按挂的知识点归章；点一下去题库看这个学科" className="stat-span">
                    <BucketBars
                        items={data.byChapter}
                        total={data.total}
                        onPick={() => goLibrary(subjects[0])}
                        hint="先在「录入」或「生成」里补题"
                    />
                </Card>
            ) : (
                <Card title="学科分布" subtitle="点一下按学科查看题目" className="stat-span">
                    <BucketBars
                        items={subjectBars}
                        total={data.total}
                        onPick={(item) => goLibrary(item.key)}
                        hint="先在「录入」或「生成」里补题"
                    />
                </Card>
            )}

            <div className="stat-columns">
                <Card title="题型结构" subtitle="客观题与主观题的比例">
                    <BucketBars items={types} total={data.total}/>
                </Card>
                <Card title="难度分布" subtitle="1 星最容易，5 星最难">
                    <BucketBars items={data.byDifficulty} total={data.total}/>
                </Card>
            </div>

            <Card title="月度新增" subtitle="最近 6 个月录入 / 生成的题量" className="stat-span">
                <MonthChart months={data.months}/>
            </Card>

            <Card
                title="补题建议"
                subtitle="按影响排序：先说会让组卷组不出来的，再说整理类的"
                actions={<Pill tone="muted">{data.suggestions.length} 条</Pill>}
            >
                {data.suggestions.length === 0 ? (
                    <Empty title="结构挺健康" hint="题型、难度、章节都有覆盖，暂时没什么要补的"/>
                ) : (
                    <ul className="stat-suggest">
                        {data.suggestions.map((item, index) => {
                            const level = LEVELS[item.level] || LEVELS.low;
                            return (
                                <li className={`stat-suggest-item ${item.level}`} key={`${item.title}-${index}`}>
                                    <span className="stat-suggest-level">{level.label}</span>
                                    <div className="stat-suggest-body">
                                        <strong>{item.title}</strong>
                                        <p>{item.detail}</p>
                                    </div>
                                    <button
                                        type="button"
                                        className="btn ghost small"
                                        onClick={() =>
                                            item.action === 'generate'
                                                ? goGenerate(item)
                                                : goLibrary(item.subjectId)
                                        }
                                    >
                                        {item.action === 'generate' ? '去出题' : '去整理'}
                                    </button>
                                </li>
                            );
                        })}
                    </ul>
                )}
            </Card>

            {data.emptyChapters.length > 0 && (
                <Card
                    title="还没题的章节"
                    subtitle={`共 ${data.emptyChapters.length} 个；补题时从这里挑最省事`}
                >
                    <div className="stat-chapters">
                        {data.emptyChapters.slice(0, 18).map((gap) => (
                            <button
                                type="button"
                                className="chip"
                                key={`${gap.subjectId}-${gap.chapter}`}
                                onClick={() => goLibrary(gap.subjectId)}
                                title="按这个学科查看题目"
                            >
                                {single ? gap.chapter : `${gap.subject} · ${gap.chapter}`}
                            </button>
                        ))}
                        {data.emptyChapters.length > 18 && (
                            <span className="field-hint">还有 {data.emptyChapters.length - 18} 个…</span>
                        )}
                    </div>
                </Card>
            )}
        </div>
    );
}
