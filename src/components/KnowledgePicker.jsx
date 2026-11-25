import {useMemo, useState} from 'react';
import {cx} from '../lib/format.js';

/**
 * 知识点选择器：可搜索、跨学科、最多 max 个，并且允许当场新建。
 *
 * 为什么必须有「新建」：内置词表再全，也有校本专题、新题型、跨章节综合覆盖不到，
 * 只能从列表里挑等于逼老师把题塞进一个不贴切的章节。输入框里敲完按回车（或点
 * 「新建」）就会在当前学科下建一个自建知识点，立刻可以选中；下次进来自建点会
 * 出现在知识树的「自建知识点 / 自定义」下。
 *
 * 为什么跨学科：OCR / 网页取题偶尔把学科判错，只列当前学科的知识点会让用户
 * 「明明有这个知识点却选不到」。这里把全部学科的知识点都列出来，当前学科排在
 * 最前、其余带学科名前缀，搜索按名字 / 章节 / 学科过滤。
 *
 * value 是已选 id 数组，onChange 回传新的 id 数组；tree 是 curriculum() 的教材树。
 * onCreate 是异步新建：传入名字，返回 {id, name, path}。
 */
export function KnowledgePicker({
    tree = [],
    value = [],
    subjectId,
    max = 6,
    knownNames = {},
    onChange,
    onCreate,
}) {
    const [query, setQuery] = useState('');
    const [created, setCreated] = useState([]);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState('');

    // 拍平成 [{id, name, label, subjectId, own}]，当前学科优先。
    const all = useMemo(() => {
        const own = [];
        const others = [];
        for (const subject of tree) {
            const isOwn = subject.id === subjectId;
            for (const textbook of subject.textbooks || []) {
                for (const chapter of textbook.chapters || []) {
                    for (const point of chapter.points || []) {
                        const entry = {
                            id: point.id,
                            name: point.name,
                            label: `${chapter.name} · ${point.name}`,
                            search: `${point.name} ${chapter.name} ${textbook.name} ${subject.name}`.toLowerCase(),
                            subjectName: subject.name,
                            own: isOwn,
                        };
                        (isOwn ? own : others).push(entry);
                    }
                }
            }
        }
        return [...own, ...others];
    }, [tree, subjectId]);

    // 本次会话里刚建的点还没进 tree，先并入候选与名字解析。
    const merged = useMemo(() => {
        if (!created.length) return all;
        const known = new Set(all.map((entry) => entry.id));
        const fresh = created
            .filter((point) => !known.has(point.id))
            .map((point) => ({
                id: point.id,
                name: point.name,
                label: point.path ? `${point.path} · ${point.name}` : point.name,
                search: point.name.toLowerCase(),
                subjectName: '自建',
                own: true,
                fresh: true,
            }));
        return [...fresh, ...all];
    }, [all, created]);

    const q = query.trim().toLowerCase();
    const candidates = useMemo(() => {
        const list = q ? merged.filter((entry) => entry.search.includes(q)) : merged;
        // 搜索时优先当前学科；没搜索时全量本就排好序。截取上限避免一次铺太多。
        return list.slice(0, 40);
    }, [merged, q]);

    const toggle = (id) => {
        if (value.includes(id)) {
            onChange?.(value.filter((each) => each !== id));
        } else if (value.length < max) {
            onChange?.([...value, id].slice(0, max));
        }
    };

    const full = value.length >= max;
    // 名字解析三级兜底：本次新建 > 当前教材树 > 题目存下来的名字。
    // 新建的知识点可能还没进 tree，此时若直接回退显示 id，就会变成一串字符。
    const nameOf = (id) =>
        merged.find((entry) => entry.id === id)?.name || knownNames[id] || id;

    const exact = merged.find(
        (entry) => entry.name.trim().toLowerCase() === q && entry.own,
    );
    const canCreate = Boolean(onCreate) && q.length > 0 && !exact && !full && !busy;

    const create = async () => {
        const name = query.trim();
        if (!name || !onCreate || busy) return;
        setBusy(true);
        setError('');
        try {
            const point = await onCreate(name);
            if (point?.id) {
                setCreated((current) => [...current, point]);
                if (!value.includes(point.id) && value.length < max) {
                    onChange?.([...value, point.id]);
                }
                setQuery('');
            }
        } catch (failure) {
            setError(failure.message || '新建失败，换个名字再试');
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="kp-picker">
            <input
                type="search"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={(event) => {
                    if (event.key !== 'Enter') return;
                    event.preventDefault();
                    if (exact && !value.includes(exact.id)) {
                        toggle(exact.id);
                        setQuery('');
                        return;
                    }
                    if (canCreate) create();
                }}
                placeholder="搜知识点，没有就直接敲名字回车新建"
                aria-label="搜索知识点"
            />
            {value.length > 0 && (
                <div className="chip-row">
                    {value.map((id) => (
                        <button
                            type="button"
                            key={id}
                            className="chip active"
                            onClick={() => toggle(id)}
                            title="点击移除"
                        >
                            {nameOf(id)} ✕
                        </button>
                    ))}
                </div>
            )}
            <div className="kp-candidates">
                {canCreate && (
                    <button
                        type="button"
                        className="chip kp-create"
                        onClick={create}
                        title={`在当前学科下新建知识点「${query.trim()}」`}
                    >
                        {busy ? '新建中…' : `＋ 新建「${query.trim()}」`}
                    </button>
                )}
                {candidates.map((entry) => {
                    const selected = value.includes(entry.id);
                    return (
                        <button
                            type="button"
                            key={entry.id}
                            className={cx('chip', selected && 'active', entry.fresh && 'kp-fresh')}
                            disabled={!selected && full}
                            onClick={() => toggle(entry.id)}
                            title={entry.own ? entry.label : `${entry.subjectName} · ${entry.label}`}
                        >
                            {entry.own ? entry.name : `${entry.subjectName} · ${entry.name}`}
                        </button>
                    );
                })}
                {candidates.length === 0 && !canCreate && (
                    <span className="muted small">没找到匹配的知识点；直接输入名字可以新建。</span>
                )}
            </div>
            {error && <span className="field-hint kp-error">{error}</span>}
            <span className="field-hint">
                已选 {value.length}/{max}，当前学科排在前，可跨其它学科搜索；没有的词直接新建
            </span>
        </div>
    );
}
