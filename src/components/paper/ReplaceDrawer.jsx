import {useEffect, useState} from 'react';
import {paperReplaceCandidates} from '../../lib/api.js';
import {RichInline} from '../../lib/Math.jsx';
import {DIFFICULTIES} from '../../lib/format.js';
import {Drawer, Empty, Loading, Pill, notify} from '../ui.jsx';

const difficultyLabel = (value) =>
    DIFFICULTIES.find((item) => Number(item.value) === Number(value))?.label || `难度 ${value}`;

/**
 * 换题抽屉：先出候选、再点替换，不直接改卷子。
 * 候选来自「同学科 + 同题型 + 至少共享一个知识点」的已入库题，卷内已有的题不会出现。
 */
export function ReplaceDrawer({paperId, sectionId, ord, number, sectionLabel, onClose, onPick}) {
    const [filters, setFilters] = useState({keyword: '', difficulty: ''});
    const [rows, setRows] = useState([]);
    const [loading, setLoading] = useState(true);
    const [busyId, setBusyId] = useState('');

    useEffect(() => {
        let alive = true;
        const timer = setTimeout(async () => {
            setLoading(true);
            try {
                const list = await paperReplaceCandidates(paperId, {
                    sectionId,
                    ord,
                    keyword: filters.keyword || null,
                    difficulty: filters.difficulty ? Number(filters.difficulty) : null,
                    limit: 30,
                });
                if (!alive) return;
                setRows(list || []);
            } catch (error) {
                if (alive) notify.error(error.message);
            } finally {
                if (alive) setLoading(false);
            }
        }, 240);
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [paperId, sectionId, ord, filters]);

    const pick = async (row) => {
        setBusyId(row.id);
        try {
            await onPick(row.id);
        } finally {
            setBusyId('');
        }
    };

    return (
        <Drawer
            title="换一道题"
            subtitle={`第 ${number || ord + 1} 题将换成你点的这道；题号、分值与「${sectionLabel}」都不变，换完可以撤销`}
            onClose={onClose}
            footer={
                <>
                    <span className="muted">
                        候选 {rows.length} 道 · 只取已入库、同学科同题型、共用知识点的题
                    </span>
                    <div className="drawer-actions-right">
                        <button className="btn ghost" onClick={onClose}>
                            不改了
                        </button>
                    </div>
                </>
            }
        >
            <div className="paper-picker-filters">
                <input
                    className="paper-search"
                    placeholder="搜索题干"
                    value={filters.keyword}
                    onChange={(event) =>
                        setFilters((current) => ({...current, keyword: event.target.value}))
                    }
                />
                <select
                    value={filters.difficulty}
                    onChange={(event) =>
                        setFilters((current) => ({...current, difficulty: event.target.value}))
                    }
                >
                    <option value="">全部难度</option>
                    {DIFFICULTIES.map((level) => (
                        <option key={level.value} value={level.value}>
                            {level.label}
                        </option>
                    ))}
                </select>
            </div>

            {loading ? (
                <Loading text="正在找同类型的题…"/>
            ) : rows.length === 0 ? (
                <Empty
                    title="没有可换的题"
                    hint="题库里同题型、已入库的题都用上了；可以先去题库补题，或改判难度筛选"
                />
            ) : (
                <div className="pick-list">
                    {rows.map((row) => (
                        <div className="pick replace-pick" key={row.id}>
                            <span className="pick-stem">
                                <RichInline text={row.stem}/>
                            </span>
                            <span className="replace-meta">
                                <Pill tone="line">{row.typeLabel}</Pill>
                                <Pill tone="muted">{difficultyLabel(row.difficulty)}</Pill>
                                {row.knowledge?.slice(0, 2).map((name) => (
                                    <Pill key={name} tone="muted">
                                        {name}
                                    </Pill>
                                ))}
                                <span className="muted">
                                    {row.usedCount > 0 ? `被用过 ${row.usedCount} 次` : '没用过'}
                                </span>
                            </span>
                            <button
                                className="btn ghost small"
                                onClick={() => pick(row)}
                                disabled={Boolean(busyId)}
                            >
                                {busyId === row.id ? '正在换…' : '换成这道'}
                            </button>
                        </div>
                    ))}
                </div>
            )}
        </Drawer>
    );
}
