import {useCallback, useEffect, useMemo, useState} from 'react';
import {
    deleteQuestions,
    exportQuestions,
    exportQuestionsZip,
    getQuestion,
    listQuestions,
    listTags,
    purgeQuestion,
    questionSources,
    restoreQuestions,
    reviewQuestions,
} from '../lib/api.js';
import {DIFFICULTIES, TYPES, cx, formatTime, pickSavePath, saveTextFile, typeLabel} from '../lib/format.js';
import {Rich, NbImage} from '../lib/Math.jsx';
import {QuestionCard} from '../components/QuestionCard.jsx';
import {QuestionEditor} from '../components/QuestionEditor.jsx';
import {Pager} from '../components/Pager.jsx';
import {AddToPaper} from '../components/paper/AddToPaper.jsx';
import {Drawer, Dropdown, Empty, Field, Loading, Modal, PageHead, Pill, confirmDialog, notify} from '../components/ui.jsx';

const DEFAULT_PAGE_SIZE = 20;
const TAG_COLLAPSED = 14;

const SORTS = [
    {value: '', label: '最近更新'},
    {value: 'created_desc', label: '最近创建'},
    {value: 'difficulty_asc', label: '难度从易到难'},
    {value: 'difficulty_desc', label: '难度从难到易'},
];

const EMPTY_FILTERS = {
    keyword: '',
    subjectId: '',
    kind: '',
    difficulty: '',
    source: '',
    tag: '',
    sort: '',
    needsReview: false,
    rejected: false,
    trashed: false,
    page: 1,
};

function hostOf(url) {
    try {
        return new URL(url).hostname.replace(/^www\./, '') || url;
    } catch {
        return url;
    }
}

/** 放大镜图标：用 SVG 而不是字符，避免 ⌕ 在小字号下糊成一撇。 */
function SearchIcon() {
    return (
        <svg viewBox="0 0 16 16" width="13" height="13" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden="true">
            <circle cx="7" cy="7" r="4.3"/>
            <path d="M10.2 10.2 13.6 13.6" strokeLinecap="round"/>
        </svg>
    );
}

/** 标签搜索命中的片段高亮，方便一眼看到为什么匹配。 */
function Highlight({text, query}) {
    const needle = (query || '').toLowerCase();
    if (!needle) return text;
    const index = text.toLowerCase().indexOf(needle);
    if (index < 0) return text;
    return (
        <>
            {text.slice(0, index)}
            <mark>{text.slice(index, index + needle.length)}</mark>
            {text.slice(index + needle.length)}
        </>
    );
}

export default function Library({tree, initialSubject = '', scopes, onChanged, onGo}) {
    const [filters, setFilters] = useState(() => ({
        ...EMPTY_FILTERS,
        // 统计页点学科进来时直接按这个学科筛，不用再选一次。
        subjectId: initialSubject || '',
    }));
    const [keyword, setKeyword] = useState('');
    const [pageSize, setPageSize] = useState(DEFAULT_PAGE_SIZE);
    const [tagQuery, setTagQuery] = useState('');
    const [tagOpen, setTagOpen] = useState(false);
    const [data, setData] = useState({items: [], total: 0, page: 1, size: DEFAULT_PAGE_SIZE});
    const [loading, setLoading] = useState(true);
    const [tags, setTags] = useState([]);
    const [sources, setSources] = useState([]);
    const [selected, setSelected] = useState(() => new Set());
    const [detailId, setDetailId] = useState('');
    const [detail, setDetail] = useState(null);
    const [editing, setEditing] = useState(null);
    const [rejecting, setRejecting] = useState(null); // {ids, note} 驳回原因弹窗
    const [addingPaper, setAddingPaper] = useState(null); // 选中的题要加进哪份试卷
    // 标签云与来源只在题量/内容真变了时刷新；翻页不该再把它们整个拉一遍。
    const [catalogKey, setCatalogKey] = useState(0);

    useEffect(() => {
        const timer = setTimeout(() => setFilters((current) => ({...current, keyword, page: 1})), 260);
        return () => clearTimeout(timer);
    }, [keyword]);

    // 从统计页换一个学科再点进来：组件不会重挂，得跟着 prop 走。
    useEffect(() => {
        if (!initialSubject) return;
        setFilters((current) =>
            current.subjectId === initialSubject ? current : {...current, subjectId: initialSubject, page: 1},
        );
    }, [initialSubject]);

    const load = useCallback(async () => {
        setLoading(true);
        try {
            const query = {
                keyword: filters.keyword || null,
                subjectId: filters.subjectId || null,
                kind: filters.kind || null,
                difficulty: filters.difficulty ? Number(filters.difficulty) : null,
                tag: filters.tag || null,
                source: filters.source || null,
                sort: filters.sort || null,
                needsReview: filters.needsReview || null,
                rejected: filters.rejected || null,
                trashed: filters.trashed,
                page: filters.page,
                size: pageSize,
            };
            const result = await listQuestions(query);
            setData(result);
            // 翻页/筛选本来就没有跨页勾选；已经是空集时别再换一个 Set 触发整页重渲染。
            setSelected((current) => (current.size ? new Set() : current));
        } catch (error) {
            notify.error(error.message);
        } finally {
            setLoading(false);
        }
    }, [filters, pageSize]);

    useEffect(() => {
        load();
    }, [load]);

    // 标签与来源随题库内容变化刷新（采集入库、编辑归类后由 refreshAll 递增）；
    // 单纯翻页 / 改筛选不动 catalogKey，省掉每次翻页的两次额外查询与整片重渲染。
    useEffect(() => {
        listTags().then(setTags).catch(() => {});
        if (filters.trashed) return;
        questionSources().then(setSources).catch(() => {});
    }, [catalogKey, filters.trashed]);

    // 删到当前页空了，或筛选后页码越界：自动退到最后一页，避免停在空白页。
    useEffect(() => {
        if (loading || filters.page <= 1) return;
        if (data.items.length > 0) return;
        const last = Math.max(1, Math.ceil(data.total / pageSize));
        if (filters.page > last) setFilters((current) => ({...current, page: last}));
    }, [loading, data, pageSize, filters.page]);

    useEffect(() => {
        if (!detailId) {
            setDetail(null);
            return;
        }
        getQuestion(detailId)
            .then(setDetail)
            .catch((error) => notify.error(error.message));
    }, [detailId]);

    const pageCount = Math.max(1, Math.ceil(data.total / pageSize));
    const pageStart = data.total === 0 ? 0 : (filters.page - 1) * pageSize + 1;
    const pageEnd = Math.min(data.total, filters.page * pageSize);
    const selectedIds = useMemo(() => [...selected], [selected]);
    const allSelected = data.items.length > 0 && data.items.every((question) => selected.has(question.id));

    const filteredTags = useMemo(() => {
        const needle = tagQuery.trim().toLowerCase();
        if (!needle) return tags;
        return tags.filter((item) => item.tag.toLowerCase().includes(needle));
    }, [tags, tagQuery]);

    const shownTags = tagQuery.trim() || tagOpen ? filteredTags : filteredTags.slice(0, TAG_COLLAPSED);

    const activeChips = useMemo(() => {
        const out = [];
        const patch = (values) => setFilters((current) => ({...current, ...values, page: 1}));
        if (filters.keyword) {
            out.push({
                key: 'keyword',
                label: `关键词：${filters.keyword}`,
                clear: () => {
                    setKeyword('');
                    patch({keyword: ''});
                },
            });
        }
        if (filters.tag) {
            out.push({key: 'tag', label: `标签：${filters.tag}`, clear: () => patch({tag: ''})});
        }
        if (filters.subjectId) {
            const name = tree.find((subject) => subject.id === filters.subjectId)?.name || '已选学科';
            out.push({key: 'subject', label: `学科：${name}`, clear: () => patch({subjectId: ''})});
        }
        if (filters.kind) {
            const name = TYPES.find((type) => type.value === filters.kind)?.label || filters.kind;
            out.push({key: 'kind', label: `题型：${name}`, clear: () => patch({kind: ''})});
        }
        if (filters.difficulty) {
            const name = DIFFICULTIES.find((item) => String(item.value) === String(filters.difficulty))?.label || filters.difficulty;
            out.push({key: 'difficulty', label: `难度：${name}`, clear: () => patch({difficulty: ''})});
        }
        if (filters.source) {
            out.push({key: 'source', label: `来源：${hostOf(filters.source)}`, clear: () => patch({source: ''})});
        }
        return out;
    }, [filters, tree]);

    const scrollTop = () => {
        // 翻页要跟手：平滑滚动会和整页重渲染撞在一起，看起来像卡住。
        document.querySelector('.main')?.scrollTo({top: 0, behavior: 'auto'});
    };

    const goPage = (next) => {
        const target = Math.min(pageCount, Math.max(1, next));
        if (target === filters.page) return;
        setFilters((current) => ({...current, page: target}));
        scrollTop();
    };

    const changeSize = (size) => {
        if (size === pageSize) return;
        setPageSize(size);
        setFilters((current) => ({...current, page: 1}));
        scrollTop();
    };

    const pickTag = (tag) => {
        setFilters((current) => ({...current, tag: current.tag === tag ? '' : tag, page: 1}));
    };

    const clearAll = () => {
        setKeyword('');
        setTagQuery('');
        // 清空筛选不该把人从当前列表（草稿/驳回/回收站）踢回题库。
        setFilters((current) => ({
            ...EMPTY_FILTERS,
            needsReview: current.needsReview,
            rejected: current.rejected,
            trashed: current.trashed,
        }));
    };

    const toggle = (id, checked) => {
        setSelected((current) => {
            const next = new Set(current);
            if (checked) next.add(id);
            else next.delete(id);
            return next;
        });
    };

    const toggleAll = () => {
        setSelected(() => (allSelected ? new Set() : new Set(data.items.map((question) => question.id))));
    };

    const refreshAll = async () => {
        await load();
        setCatalogKey((value) => value + 1);
        onChanged?.();
    };

    const remove = async (ids, label) => {
        const ok = await confirmDialog({
            title: label === '彻底删除' ? '彻底删除这些题？' : '移到回收站？',
            message:
                label === '彻底删除'
                    ? '图片与解析会一起删除，之后无法恢复。'
                    : '题目会进回收站，可以随时恢复；同步过的题会在下次同步时通知中心。',
            danger: label === '彻底删除',
            confirmText: label,
        });
        if (!ok) return;
        try {
            if (label === '彻底删除') {
                for (const id of ids) await purgeQuestion(id);
            } else if (label === '恢复') {
                await restoreQuestions(ids);
            } else {
                await deleteQuestions(ids);
            }
            notify.success(`${label}成功`);
            await refreshAll();
            if (ids.includes(detailId)) setDetailId('');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const approve = async (ids) => {
        try {
            const n = await reviewQuestions(ids, 'approve');
            notify.success(`已入库 ${n} 道题（复核通过，移出草稿箱）`);
            await refreshAll();
            if (ids.includes(detailId)) {
                setDetailId('');
            }
        } catch (error) {
            notify.error(error.message);
        }
    };

    /** 驳回的题改完后转回草稿，重新走复核。 */
    const backToDraft = async (ids) => {
        try {
            const n = await reviewQuestions(ids, 'draft');
            notify.success(`已转回草稿 ${n} 道题，可以继续修改后复核`);
            await refreshAll();
            if (ids.includes(detailId)) setDetailId('');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const submitReject = async () => {
        if (!rejecting) return;
        const note = (rejecting.note || '').trim();
        if (!note) {
            notify.warning('请填写驳回原因');
            return;
        }
        try {
            const n = await reviewQuestions(rejecting.ids, 'reject', note);
            notify.success(`已驳回 ${n} 道题，退回草稿并写明原因`);
            setRejecting(null);
            await refreshAll();
            if (rejecting.ids.includes(detailId)) {
                setDetailId('');
            }
        } catch (error) {
            notify.error(error.message);
        }
    };

    const exportSelected = async (withImages) => {
        if (selectedIds.length === 0) return;
        try {
            if (withImages) {
                const path = await pickSavePath('questions.zip', [{name: '题目包', extensions: ['zip']}]);
                if (!path) return;
                const result = await exportQuestionsZip(selectedIds, path);
                const tail = result.missing > 0 ? `，另有 ${result.missing} 张图文件不在了，已留占位` : '';
                notify.success(`已导出 ${result.count} 道题、${result.images} 张图${tail}`);
                if (result.warnings?.length) {
                    const more = result.warnings.length > 1 ? `（共 ${result.warnings.length} 条）` : '';
                    notify.warning(`${result.warnings[0]}${more}`);
                }
                return;
            }
            const pack = await exportQuestions(selectedIds);
            const path = await saveTextFile('questions.json', pack, [{name: '题目包', extensions: ['json']}]);
            if (path) notify.success(`已导出到 ${path}`);
        } catch (error) {
            notify.error(error.message);
        }
    };

    return (
        <div className="page">
            <PageHead
                title={
                    filters.trashed
                        ? '回收站'
                        : filters.rejected
                            ? '驳回列表'
                            : filters.needsReview
                                ? '草稿箱'
                                : '题库'
                }
                subtitle={
                    filters.trashed
                        ? '删掉的题在这里，可以恢复或彻底删除'
                        : filters.rejected
                            ? '复核没通过的题在这里；按原因改完可以转回草稿重新复核'
                            : filters.needsReview
                                ? '识别和 AI 生成的题先落在这里；复核通过才算正式入库'
                                : '所有题目都在这台电脑上，断网也能用'
                }
                help={filters.trashed ? 'backup' : 'library'}
                actions={
                    !filters.trashed && (
                        <button className="btn primary" onClick={() => setEditing({question: null})}>
                            新建题目
                        </button>
                    )
                }
            />

            {/* 四个列表互斥：题库（已入库）/ 草稿箱 / 驳回列表 / 回收站。
                三个箱子的数量来自 boot.stats（入库、改状态后 onChanged 会刷新），一眼看到欠账。 */}
            <div className="lib-scope" role="tablist" aria-label="题目列表">
                {[
                    {key: 'library', label: '题库列表', count: null, active: !filters.needsReview && !filters.rejected && !filters.trashed},
                    {key: 'draft', label: '草稿箱', count: scopes?.needsReview ?? null, active: filters.needsReview},
                    {key: 'rejected', label: '驳回列表', count: scopes?.rejected ?? null, active: filters.rejected},
                    {key: 'trash', label: '回收站', count: scopes?.trashed ?? null, active: filters.trashed},
                ].map((scope) => (
                    <button
                        key={scope.key}
                        role="tab"
                        aria-selected={scope.active}
                        className={cx('lib-scope-tab', scope.active && 'active')}
                        onClick={() => {
                            if (scope.active) return;
                            setDetailId('');
                            setKeyword('');
                            setTagQuery('');
                            setFilters({
                                ...EMPTY_FILTERS,
                                needsReview: scope.key === 'draft',
                                rejected: scope.key === 'rejected',
                                trashed: scope.key === 'trash',
                            });
                        }}
                    >
                        {scope.label}
                        {scope.count !== null && <span className="lib-scope-count">{scope.count}</span>}
                    </button>
                ))}
            </div>

            {!filters.trashed && (
                <section className="lib-bar">
                    <div className="lib-searchline">
                        <div className="lib-search">
                            <span className="lib-search-icon">
                                <SearchIcon/>
                            </span>
                            <input
                                value={keyword}
                                placeholder="搜题干、答案、解析或标签…"
                                onChange={(event) => setKeyword(event.target.value)}
                            />
                            {keyword && (
                                <button className="lib-search-clear" onClick={() => setKeyword('')} title="清空搜索">
                                    ✕
                                </button>
                            )}
                        </div>
                        <select
                            value={filters.subjectId}
                            title="按学科筛选"
                            onChange={(event) => setFilters((current) => ({...current, subjectId: event.target.value, page: 1}))}
                        >
                            <option value="">全部学科</option>
                            {tree.map((subject) => (
                                <option key={subject.id} value={subject.id}>
                                    {subject.name}
                                </option>
                            ))}
                        </select>
                        <select
                            value={filters.kind}
                            title="按题型筛选"
                            onChange={(event) => setFilters((current) => ({...current, kind: event.target.value, page: 1}))}
                        >
                            <option value="">全部题型</option>
                            {TYPES.map((type) => (
                                <option key={type.value} value={type.value}>
                                    {type.label}
                                </option>
                            ))}
                        </select>
                        <select
                            value={filters.difficulty}
                            title="按难度筛选"
                            onChange={(event) => setFilters((current) => ({...current, difficulty: event.target.value, page: 1}))}
                        >
                            <option value="">全部难度</option>
                            {DIFFICULTIES.map((item) => (
                                <option key={item.value} value={item.value}>
                                    {item.label}
                                </option>
                            ))}
                        </select>
                        {sources.length > 0 && (
                            <select
                                value={filters.source}
                                title={filters.source || '按采集来源筛选'}
                                onChange={(event) => setFilters((current) => ({...current, source: event.target.value, page: 1}))}
                            >
                                <option value="">全部来源</option>
                                {sources.map((item) => (
                                    <option key={item.source} value={item.source}>
                                        {hostOf(item.source)}（{item.count}）
                                    </option>
                                ))}
                            </select>
                        )}
                        <select
                            value={filters.sort}
                            title="排序方式"
                            onChange={(event) => setFilters((current) => ({...current, sort: event.target.value, page: 1}))}
                        >
                            {SORTS.map((item) => (
                                <option key={item.value} value={item.value}>
                                    {item.label}
                                </option>
                            ))}
                        </select>
                    </div>

                    {tags.length > 0 && (
                        <div className="lib-tagbar">
                            <div className="lib-tagbar-head">
                                <span className="lib-tagbar-title">常用标签</span>
                                <div className="lib-tagsearch">
                                    <span aria-hidden="true">
                                        <SearchIcon/>
                                    </span>
                                    <input
                                        value={tagQuery}
                                        placeholder="搜标签，模糊匹配"
                                        onChange={(event) => setTagQuery(event.target.value)}
                                    />
                                    {tagQuery && (
                                        <button onClick={() => setTagQuery('')} title="清空标签搜索">
                                            ✕
                                        </button>
                                    )}
                                </div>
                                {filteredTags.length > TAG_COLLAPSED && (
                                    <button className="link-btn" onClick={() => setTagOpen((value) => !value)}>
                                        {tagOpen ? '收起' : `展开全部 ${filteredTags.length} 个`}
                                    </button>
                                )}
                            </div>
                            <div className="chip-row">
                                {shownTags.map((item) => (
                                    <button
                                        key={item.tag}
                                        className={cx('chip', filters.tag === item.tag && 'active')}
                                        title={filters.tag === item.tag ? '再点一下取消这个标签' : `只看标签「${item.tag}」`}
                                        onClick={() => pickTag(item.tag)}
                                    >
                                        <Highlight text={item.tag} query={tagQuery.trim()}/>
                                        <span className="chip-count">{item.count}</span>
                                    </button>
                                ))}
                                {filteredTags.length === 0 && (
                                    <span className="muted small">
                                        没有匹配「{tagQuery.trim()}」的标签，换个词试试；也可以直接用上面的搜索框搜题
                                    </span>
                                )}
                            </div>
                        </div>
                    )}

                    {activeChips.length > 0 && (
                        <div className="lib-active">
                            <span className="lib-active-label">已筛选</span>
                            {activeChips.map((chip) => (
                                <button key={chip.key} className="filter-pill" onClick={chip.clear} title="点掉这个条件">
                                    {chip.label}
                                    <span aria-hidden="true">✕</span>
                                </button>
                            ))}
                            <button className="link-btn" onClick={clearAll}>
                                清空全部
                            </button>
                        </div>
                    )}
                </section>
            )}

            <div className="list-head">
                <div className="list-count">
                    共 <strong>{data.total}</strong> 道
                    {data.total > 0 && (
                        <span className="muted">
                            {' '}
                            · 当前 {pageStart}–{pageEnd} 道 · 第 {filters.page}/{pageCount} 页
                        </span>
                    )}
                    {pageCount > 1 && (
                        <span className="list-flip">
                            <button
                                className="pg"
                                disabled={filters.page <= 1}
                                onClick={() => goPage(filters.page - 1)}
                                title="上一页"
                            >
                                ‹
                            </button>
                            <button
                                className="pg"
                                disabled={filters.page >= pageCount}
                                onClick={() => goPage(filters.page + 1)}
                                title="下一页"
                            >
                                ›
                            </button>
                        </span>
                    )}
                </div>
                <div className="list-tools">
                    {data.items.length > 0 && (
                        <label className="check small list-check">
                            <input type="checkbox" checked={allSelected} onChange={toggleAll}/>
                            全选本页
                        </label>
                    )}
                    {selectedIds.length > 0 && (
                        <>
                            <span className="muted">已选 {selectedIds.length} 道</span>
                            {filters.trashed ? (
                                <>
                                    <button className="btn ghost small" onClick={() => remove(selectedIds, '恢复')}>
                                        恢复
                                    </button>
                                    <button className="btn danger small" onClick={() => remove(selectedIds, '彻底删除')}>
                                        彻底删除
                                    </button>
                                </>
                            ) : filters.rejected ? (
                                <>
                                    <button
                                        className="btn ghost small"
                                        title="改完的题退回草稿，重新走复核"
                                        onClick={() => backToDraft(selectedIds)}
                                    >
                                        转回草稿
                                    </button>
                                    <Dropdown label="导出" className="inline">
                                        <button onClick={() => exportSelected(true)}>导出 ZIP（含图片）</button>
                                        <button onClick={() => exportSelected(false)}>导出 JSON（只有文字）</button>
                                    </Dropdown>
                                    <button
                                        className="btn ghost small"
                                        onClick={() => {
                                            setSelected(new Set());
                                        }}
                                    >
                                        取消选择
                                    </button>
                                    <button className="btn ghost small" onClick={() => remove(selectedIds, '删除')}>
                                        移到回收站
                                    </button>
                                </>
                            ) : filters.needsReview ? (
                                <>
                                    {/* 复核 / 驳回是草稿箱的动作：题库列表不掺和，看中的题用「加入试卷」 */}
                                    <button
                                        className="btn ghost small"
                                        title="复核通过后正式入库"
                                        onClick={async () => {
                                            const ok = await confirmDialog({
                                                title: `通过这 ${selectedIds.length} 道题？`,
                                                message: '答案、解析和知识点都补齐后才能入库；通过后移出草稿箱。',
                                                confirmText: '复核通过',
                                            });
                                            if (ok) await approve(selectedIds);
                                        }}
                                    >
                                        复核通过（入库）
                                    </button>
                                    <button
                                        className="btn ghost small"
                                        onClick={() => setRejecting({ids: selectedIds, note: ''})}
                                    >
                                        驳回
                                    </button>
                                    <Dropdown label="导出" className="inline">
                                        <button onClick={() => exportSelected(true)}>导出 ZIP（含图片）</button>
                                        <button onClick={() => exportSelected(false)}>导出 JSON（只有文字）</button>
                                    </Dropdown>
                                    <button
                                        className="btn ghost small"
                                        onClick={() => {
                                            setSelected(new Set());
                                        }}
                                    >
                                        取消选择
                                    </button>
                                    <button className="btn ghost small" onClick={() => remove(selectedIds, '删除')}>
                                        移到回收站
                                    </button>
                                </>
                            ) : (
                                <>
                                    <button
                                        className="btn ghost small"
                                        title="把选中的题加进一份草稿试卷"
                                        onClick={() => setAddingPaper(selectedIds)}
                                    >
                                        加入试卷
                                    </button>
                                    <Dropdown label="导出" className="inline">
                                        <button onClick={() => exportSelected(true)}>导出 ZIP（含图片）</button>
                                        <button onClick={() => exportSelected(false)}>导出 JSON（只有文字）</button>
                                    </Dropdown>
                                    <button
                                        className="btn ghost small"
                                        onClick={() => {
                                            setSelected(new Set());
                                        }}
                                    >
                                        取消选择
                                    </button>
                                    <button className="btn ghost small" onClick={() => remove(selectedIds, '删除')}>
                                        移到回收站
                                    </button>
                                </>
                            )}
                        </>
                    )}
                </div>
            </div>

            {loading ? (
                <Loading/>
            ) : data.items.length === 0 ? (
                <Empty
                    title={
                        filters.keyword || filters.tag || activeChips.length > 0
                            ? '没有符合条件的题'
                            : filters.trashed
                                ? '回收站是空的'
                                : filters.rejected
                                    ? '驳回列表是空的'
                                    : filters.needsReview
                                        ? '草稿箱是空的'
                                        : '题库还是空的'
                    }
                    hint={
                        filters.keyword || filters.tag || activeChips.length > 0
                            ? '去掉一两个条件，或用上面的标签换个方向看'
                            : filters.trashed
                                ? '删掉的题会出现在这里'
                                : filters.rejected
                                    ? '复核没通过的题会出现在这里，按原因改完可以转回草稿'
                                    : filters.needsReview
                                        ? '录入或 AI 生成后「存为草稿」的题会落在这里，复核通过才算正式入库'
                                        : '先截一张图识别，或者导入一个题目包'
                    }
                    action={
                        !filters.trashed && (
                            <div className="empty-actions">
                                {activeChips.length > 0 ? (
                                    <button className="btn ghost" onClick={clearAll}>
                                        清空筛选
                                    </button>
                                ) : (
                                    <>
                                        <button className="btn primary" onClick={() => onGo('capture')}>
                                            截图录题
                                        </button>
                                        <button className="btn ghost" onClick={() => onGo('import')}>
                                            批量导入
                                        </button>
                                    </>
                                )}
                            </div>
                        )
                    }
                />
            ) : (
                <div className="q-list">
                    {data.items.map((question) => (
                        <QuestionCard
                            key={question.id}
                            question={question}
                            selected={selected.has(question.id)}
                            onSelect={toggle}
                            onOpen={setDetailId}
                        />
                    ))}
                </div>
            )}

            <Pager
                page={filters.page}
                pageCount={pageCount}
                total={data.total}
                size={pageSize}
                onPage={goPage}
                onSize={changeSize}
            />

            {detail && (
                <Drawer
                    title={typeLabel(detail.type)}
                    subtitle={`难度 ${detail.difficulty} · ${detail.subjectName}${detail.source ? ` · ${detail.source}` : ''}`}
                    onClose={() => setDetailId('')}
                    footer={
                        <>
                            {filters.trashed ? (
                                <>
                                    <button className="btn ghost" onClick={() => remove([detail.id], '恢复')}>
                                        恢复
                                    </button>
                                    <button className="btn danger" onClick={() => remove([detail.id], '彻底删除')}>
                                        彻底删除
                                    </button>
                                </>
                            ) : (
                                <>
                                    {/* 复核 / 驳回只在草稿箱里提供；题库列表的详情只管看和改 */}
                                    {filters.needsReview && detail.needsReview && (
                                        <>
                                            <button
                                                className="btn ghost"
                                                onClick={async () => {
                                                    const ok = await confirmDialog({
                                                        title: '通过这道题的复核？',
                                                        message: '答案、解析和知识点都补齐后才能入库；通过后移出草稿箱。',
                                                        confirmText: '复核通过',
                                                    });
                                                    if (ok) await approve([detail.id]);
                                                }}
                                            >
                                                复核通过（入库）
                                            </button>
                                            <button
                                                className="btn ghost"
                                                onClick={() => setRejecting({ids: [detail.id], note: ''})}
                                            >
                                                驳回
                                            </button>
                                        </>
                                    )}
                                    {filters.rejected && !detail.needsReview && detail.reviewNote && (
                                        <button className="btn ghost" onClick={() => backToDraft([detail.id])}>
                                            转回草稿
                                        </button>
                                    )}
                                    <button className="btn ghost" onClick={() => remove([detail.id], '删除')}>
                                        删除
                                    </button>
                                    <button className="btn primary" onClick={() => setEditing({question: detail})}>
                                        编辑
                                    </button>
                                </>
                            )}
                        </>
                    }
                >
                    <div className="detail-meta">
                        <span className="detail-id" title={`本地题目编号：${detail.id}`}>
                            #{detail.id.slice(0, 8)}
                        </span>
                        <Pill tone="line">更新于 {formatTime(detail.updatedAt)}</Pill>
                        {detail.needsReview && <Pill tone="warn">草稿</Pill>}
                        {detail.status === 'PUBLISHED' && <Pill tone="ok">已入库</Pill>}
                        {/* 草稿本来就没有「驳回」这回事，只有复核过（needsReview=false）带原因才是真被驳回 */}
                        {!detail.needsReview && detail.reviewNote && (
                            <Pill tone="danger">已驳回：{detail.reviewNote}</Pill>
                        )}
                        {detail.aiAnswer && <Pill tone="line">答案由 AI 补写</Pill>}
                        {detail.aiAnalysis && <Pill tone="line">解析由 AI 补写</Pill>}
                        {detail.origin === 'CAPTURE' && <Pill tone="line">截图识别</Pill>}
                        {detail.origin === 'AI' && <Pill tone="line">AI 生成</Pill>}
                        {detail.origin === 'IMPORT' && <Pill tone="line">批量导入</Pill>}
                        {detail.origin === 'CENTER' && <Pill tone="line">中心同步</Pill>}
                    </div>

                    <section className="detail-block">
                        <h4>题干</h4>
                        <Rich text={detail.stem}/>
                    </section>

                    {detail.options.length > 0 && (
                        <section className="detail-block">
                            <h4>选项</h4>
                            <ul className="option-view">
                                {detail.options.map((option) => (
                                    <li key={option.key}>
                                        <span className="option-key">{option.key}</span>
                                        <Rich text={option.text} className="inline"/>
                                    </li>
                                ))}
                            </ul>
                        </section>
                    )}

                    {detail.assets.length > 0 && (
                        <section className="detail-block">
                            <h4>配图</h4>
                            <div className="asset-row">
                                {detail.assets.map((asset) => (
                                    <div className="asset-thumb" key={asset.id}>
                                        <NbImage id={asset.id} alt={asset.kind}/>
                                        <span className="asset-kind">{asset.kind}</span>
                                    </div>
                                ))}
                            </div>
                        </section>
                    )}

                    <section className="detail-block answer">
                        <h4>答案</h4>
                        <Rich text={detail.answer}/>
                    </section>

                    {detail.analysis && (
                        <section className="detail-block">
                            <h4>解析</h4>
                            <Rich text={detail.analysis}/>
                        </section>
                    )}

                    <section className="detail-block">
                        <h4>知识点</h4>
                        <div className="chip-row">
                            {detail.knowledge.map((point) => (
                                <span key={point.id} className="tag kp" title={point.path}>
                                    {point.name}
                                </span>
                            ))}
                            {detail.tags.map((tag) => (
                                <span key={tag} className="tag">
                                    {tag}
                                </span>
                            ))}
                            {detail.knowledge.length === 0 && (
                                <span className="muted">还没有归类，可以用「生成 → 批量补充」补上</span>
                            )}
                        </div>
                    </section>
                </Drawer>
            )}

            {editing && (
                <QuestionEditor
                    question={editing.question}
                    tree={tree}
                    onClose={() => setEditing(null)}
                    onSaved={async (id) => {
                        await refreshAll();
                        if (detailId) setDetailId(id);
                    }}
                />
            )}

            {addingPaper && (
                <AddToPaper
                    questionIds={addingPaper}
                    onClose={() => setAddingPaper(null)}
                    onDone={() => setSelected(new Set())}
                />
            )}

            {rejecting && (
                <Modal
                    title={`驳回 ${rejecting.ids.length} 道题`}
                    subtitle="驳回后会退回草稿，请写明需要修改的地方"
                    onClose={() => setRejecting(null)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={() => setRejecting(null)}>取消</button>
                            <button className="btn danger" onClick={submitReject}>确认驳回</button>
                        </>
                    }
                >
                    <Field label="驳回原因" hint="例如：答案与解析不一致、题干缺图">
                        <textarea
                            className="textarea"
                            rows={4}
                            autoFocus
                            value={rejecting.note}
                            onChange={(event) => setRejecting((r) => ({...r, note: event.target.value}))}
                            placeholder="写明这道题需要修改的地方…"
                        />
                    </Field>
                </Modal>
            )}
        </div>
    );
}
