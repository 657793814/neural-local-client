import {useEffect, useMemo, useState} from 'react';
import {listQuestions} from '../../lib/api.js';
import {RichInline} from '../../lib/Math.jsx';
import {DIFFICULTIES, TYPES, subjectList, typeLabel} from '../../lib/format.js';
import {Empty, Loading, Modal, Pill, notify} from '../ui.jsx';
import {Pager} from '../Pager.jsx';
import {QuestionDetailModal} from '../QuestionDetailModal.jsx';

const PAGE_SIZE = 20;

/**
 * 大题 kind → 题库题型。界面上叫「解答题」，题库里存的是 `SOLVED`，
 * 与 paper.rs 的 `kind_types` 保持同一映射；映射只在组卷/选题这一处生效，
 * 否则按大题搜不到同类型题目。没有对应题库题型的 kind（如 JUDGE）返回空，
 * 不做题型限制，避免整组选题搜不到任何题。
 */
function sectionKindToQuestionType(kind) {
    const value = String(kind || '').toUpperCase();
    if (value === 'SINGLE' || value === 'MULTI' || value === 'FILL' || value === 'SOLVED') {
        return value;
    }
    if (value === 'CALC') return 'SOLVED';
    return '';
}

/**
 * 从题库选题的宽窗口：复用题库筛选，多选后加入指定大题。
 * 已在卷内的题置灰并标「已在本卷」，避免同一道题出现两次。
 * 打开时按大题的题型（单选 / 多选 / 填空…）预选，并按创建时间倒序，新题优先。
 */
export function QuestionPicker({
    subjectId,
    subjectTree = [],
    defaultKind = '',
    usedIds,
    sectionLabel,
    onClose,
    onPick,
}) {
    const subjects = useMemo(() => subjectList(subjectTree), [subjectTree]);
    const [filters, setFilters] = useState({
        keyword: '',
        subjectId: subjectId || '',
        kind: defaultKind || '',
        difficulty: '',
    });
    const [rows, setRows] = useState([]);
    const [total, setTotal] = useState(0);
    const [page, setPage] = useState(1);
    const [size, setSize] = useState(PAGE_SIZE);
    const [loading, setLoading] = useState(true);
    const [selected, setSelected] = useState(new Set());
    const [hideUsed, setHideUsed] = useState(true);
    const [busy, setBusy] = useState(false);
    // 「详情」就地打开，不离开选题窗口；关掉继续选。
    const [detailId, setDetailId] = useState('');

    // 筛选条件变了就回到第 1 页，否则会出现「第 3 页没结果」的错觉。
    const patch = (next) => {
        setFilters((current) => ({...current, ...next}));
        setPage(1);
    };

    useEffect(() => {
        let alive = true;
        const timer = setTimeout(async () => {
            setLoading(true);
            try {
                const data = await listQuestions({
                    keyword: filters.keyword || null,
                    subjectId: filters.subjectId || null,
                    kind: sectionKindToQuestionType(filters.kind) || null,
                    difficulty: filters.difficulty ? Number(filters.difficulty) : null,
                    // 默认按创建时间倒序：新录的题排在前面，选题时先看到新鲜内容。
                    sort: 'created_desc',
                    page,
                    size,
                });
                if (!alive) return;
                setRows(data.items || []);
                setTotal(data.total || 0);
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
    }, [filters, page, size]);

    const visible = useMemo(
        () => (hideUsed ? rows.filter((row) => !usedIds.has(row.id)) : rows),
        [rows, usedIds, hideUsed],
    );
    const available = useMemo(
        () => visible.filter((row) => !usedIds.has(row.id)),
        [visible, usedIds],
    );
    const pageCount = Math.max(1, Math.ceil(total / size));
    const kindOptions =
        defaultKind && !TYPES.some((type) => type.value === defaultKind)
            ? [
                  ...TYPES,
                  {
                      value: defaultKind,
                      label: defaultKind === 'JUDGE' ? '判断题' : typeLabel(defaultKind),
                  },
              ]
            : TYPES;

    const toggle = (id) => {
        setSelected((current) => {
            const next = new Set(current);
            if (next.has(id)) next.delete(id);
            else next.add(id);
            return next;
        });
    };

    const submit = async () => {
        if (selected.size === 0) return;
        setBusy(true);
        try {
            await onPick([...selected]);
        } finally {
            setBusy(false);
        }
    };

    return (
        <Modal
            xwide
            bodyClassName="picker-window"
            title="从题库选题"
            subtitle={`选中的题会加入「${sectionLabel}」，加入时按该大题的分值记分`}
            onClose={onClose}
            footer={
                <>
                    <span className="muted picker-count">
                        共 {total} 道 · 本页 {visible.length} 道 · 已选 {selected.size} 道
                    </span>
                    <div className="picker-actions">
                        <button
                            className="btn ghost"
                            onClick={() => setSelected(new Set(available.map((row) => row.id)))}
                            disabled={available.length === 0}
                        >
                            全选本页
                        </button>
                        <button className="btn ghost" onClick={() => setSelected(new Set())}>
                            清空
                        </button>
                        <button className="btn primary" onClick={submit} disabled={busy || selected.size === 0}>
                            {busy ? '正在加入…' : `加入 ${selected.size} 道`}
                        </button>
                    </div>
                </>
            }
        >
            <div className="paper-picker-filters">
                <input
                    className="paper-search"
                    placeholder="搜索题干 / 答案 / 知识点"
                    value={filters.keyword}
                    onChange={(event) => patch({keyword: event.target.value})}
                />
                <select
                    value={filters.subjectId}
                    onChange={(event) => patch({subjectId: event.target.value})}
                    title="按学科筛选"
                >
                    <option value="">全部学科</option>
                    {subjects.map((subject) => (
                        <option key={subject.id} value={subject.id}>
                            {subject.name}
                        </option>
                    ))}
                </select>
                <select
                    value={filters.kind}
                    onChange={(event) => patch({kind: event.target.value})}
                    title="已按本大题题型预选"
                >
                    <option value="">全部题型</option>
                    {kindOptions.map((type) => (
                        <option key={type.value} value={type.value}>
                            {type.label}
                        </option>
                    ))}
                </select>
                <select
                    value={filters.difficulty}
                    onChange={(event) => patch({difficulty: event.target.value})}
                >
                    <option value="">全部难度</option>
                    {DIFFICULTIES.map((level) => (
                        <option key={level.value} value={level.value}>
                            {level.label}
                        </option>
                    ))}
                </select>
            </div>
            <label className="pe-check paper-picker-hide">
                <input
                    type="checkbox"
                    checked={hideUsed}
                    onChange={(event) => setHideUsed(event.target.checked)}
                />
                隐藏已在本卷的题
            </label>

            <div className="picker-results">
                {loading ? (
                    <Loading text="正在找题…"/>
                ) : rows.length === 0 ? (
                    <Empty title="没有符合条件的题" hint="换关键词或放宽筛选试试"/>
                ) : visible.length === 0 ? (
                    <Empty title="本页的题都已在本卷" hint="取消「隐藏已在本卷」可以查看，或者翻到下一页"/>
                ) : (
                    <div className="pick-list picker-grid">
                        {visible.map((row) => {
                            const used = usedIds.has(row.id);
                            const knowledge = row.knowledge?.[0]?.name;
                            return (
                                <label
                                    key={row.id}
                                    className={`pick ${selected.has(row.id) ? 'active' : ''} ${used ? 'disabled' : ''}`}
                                >
                                    <input
                                        type="checkbox"
                                        disabled={used}
                                        checked={selected.has(row.id)}
                                        onChange={() => toggle(row.id)}
                                    />
                                    <span className="pick-stem">
                                        <RichInline text={row.stem}/>
                                        {knowledge && <span className="pick-meta">{knowledge}</span>}
                                    </span>
                                    <Pill tone="line">{row.typeLabel}</Pill>
                                    {used ? <Pill tone="warn">已在本卷</Pill> : null}
                                    {/* 在 label 里点按钮会顺带勾选，拦住冒泡。 */}
                                    <button
                                        type="button"
                                        className="btn ghost small pick-detail"
                                        title="查看题目详情（不离开选题）"
                                        onClick={(event) => {
                                            event.preventDefault();
                                            event.stopPropagation();
                                            setDetailId(row.id);
                                        }}
                                    >
                                        详情
                                    </button>
                                </label>
                            );
                        })}
                    </div>
                )}
            </div>

            <Pager
                page={page}
                pageCount={pageCount}
                total={total}
                size={size}
                onPage={setPage}
                onSize={(next) => {
                    setSize(next);
                    setPage(1);
                }}
            />

            {detailId && <QuestionDetailModal questionId={detailId} onClose={() => setDetailId('')}/>}
        </Modal>
    );
}
