import {useEffect, useMemo, useState} from 'react';
import {TYPES} from '../../lib/format.js';
import {
    ANSWER_SHEET_STYLES,
    newAnswerSheetTemplate,
    saveCustomAnswerSheets,
} from '../../lib/paper/answerSheet.js';
import {Field, Modal, Pill, confirmDialog, notify} from '../ui.jsx';

const PX_PER_MM = 96 / 25.4;

function clone(item) {
    return {
        ...item,
        config: {
            ...item.config,
            fields: [...(item.config?.fields || [])],
            rules: (item.config?.rules || []).map((rule) => ({
                ...rule,
                types: [...(rule.types || [])],
            })),
        },
    };
}

function clamp(value, min, max) {
    return Math.max(min, Math.min(max, value));
}

/**
 * 模板的实时预览：按毫米换算成屏幕像素画一张答题卡。
 * 只画「题号 + 作答区」——与试卷打印出来的答题卡一致，不展示题干或试卷标题。
 * 若传入 entries（当前试卷的真实题号/分值/题型），则按真题分布示意；
 * 否则用代表题号示意规制。真实分页仍由试卷渲染器负责。
 */
export function SheetPreview({draft, activeIndex, onPick, entries}) {
    const config = draft?.config || {};
    const pageColumns = Number(config.pageColumns) === 1 ? 1 : 2;
    const fields = (config.fields || []).filter(Boolean);
    const rules = config.rules || [];
    const scale = 0.4;
    const mm = (value) => value * PX_PER_MM * scale;
    const pageWidth = mm(210);
    const padding = mm(8);
    const contentWidth = pageWidth - padding * 2;
    const columnWidth = (contentWidth - mm(6) * (pageColumns - 1)) / pageColumns;
    // 有真实试卷时按真题号/题型展示；没有时按每个分区各给几道代表题，
    // 否则内置模板（选择/填空/解答多条规则）只会画解答题那一段，
    // 选择题涂卡与填空横线在预览里永远看不到。
    const sample = entries?.length
        ? entries
        : rules.length
            ? rules.flatMap((rule) => {
                const type = (rule.types || [])[0] || 'SOLVED';
                const count = rule.style === 'bubble' ? 8 : 2;
                return Array.from({length: count}, () => ({number: 0, score: 0, type}));
            }).map((entry, index) => ({...entry, number: index + 1}))
            : [1, 2].map((number) => ({number, score: 0, type: 'SOLVED'}));

    const ruleOf = (entry) =>
        rules.find((rule) =>
            (rule.types || []).some((type) => String(type).toUpperCase() === String(entry.type).toUpperCase()),
        ) || rules[0];

    return (
        <div className="as-preview">
            <div className="as-preview-head">
                <strong>答题卡预览</strong>
                <span className="muted">
                    A4 · {pageColumns === 1 ? '单栏' : '双栏'} · 仅题号与作答区
                </span>
            </div>
            <div className="as-preview-canvas">
                <div className="as-sheet" style={{width: pageWidth, padding: `${padding}px`}}>
                    <div className="as-sheet-title" style={{fontSize: mm(4.2)}}>答题卡</div>
                    <div className="as-sheet-fields" style={{fontSize: mm(2.6), gap: mm(3)}}>
                        {fields.length ? (
                            fields.map((field) => (
                                <span key={field} style={{flex: '1 1 auto'}}>
                                    {field}：______
                                </span>
                            ))
                        ) : (
                            <span className="as-sheet-muted">（未设卷头字段）</span>
                        )}
                    </div>
                    <div
                        className="as-sheet-body"
                        style={{columnCount: pageColumns, columnGap: mm(6)}}
                    >
                        {!rules.length && (
                            <div className="as-sheet-muted" style={{fontSize: mm(2.6)}}>
                                还没有答题分区
                            </div>
                        )}
                        {sample.map((entry) => {
                            const rule = ruleOf(entry) || {};
                            const index = rules.indexOf(rule);
                            const itemWidth = Math.max(columnWidth, mm(20));
                            const bubbles = clamp(Number(rule.columns) || 1, 1, 4);
                            const bubbleSize = clamp(
                                Math.min(mm(4.6), (itemWidth / bubbles - mm(5.5)) / 2),
                                mm(1.6),
                                mm(4.6),
                            );
                            return (
                                <div
                                    key={entry.number}
                                    className={`as-sheet-answer-item ${activeIndex === index && index >= 0 ? 'active' : ''}`}
                                    style={{breakInside: 'avoid', marginBottom: mm(2.4)}}
                                    onClick={() => index >= 0 && onPick?.(index)}
                                    role="button"
                                    tabIndex={0}
                                    onKeyDown={(event) => {
                                        if ((event.key === 'Enter' || event.key === ' ') && index >= 0) onPick?.(index);
                                    }}
                                >
                                    <div className="as-sheet-item-head" style={{fontSize: mm(2.5)}}>
                                        <span>{entry.number}.</span>
                                        {entry.score > 0 && <span className="muted">{entry.score} 分</span>}
                                    </div>
                                    {rule.style === 'bubble' ? (
                                        <div
                                            className="as-sheet-bubbles"
                                            style={{
                                                gridTemplateColumns: `repeat(${bubbles}, minmax(0, 1fr))`,
                                                gap: mm(1.2),
                                            }}
                                        >
                                            {['A', 'B', 'C', 'D'].map((key) => (
                                                <span key={key} className="as-sheet-bubble-row" style={{gap: mm(1), fontSize: mm(2.4)}}>
                                                    <span className="as-sheet-bubble" style={{
                                                        width: bubbleSize, height: bubbleSize,
                                                        lineHeight: `${bubbleSize}px`,
                                                        fontSize: Math.max(5, bubbleSize * 0.62),
                                                    }}>{key}</span>
                                                </span>
                                            ))}
                                        </div>
                                    ) : (
                                        <div
                                            className={rule.style === 'lines' ? 'as-sheet-lines' : 'as-sheet-box'}
                                            style={{
                                                height: mm(clamp(Number(rule.heightMm) || 36, 12, 120)),
                                                backgroundSize: rule.style === 'lines' ? `100% ${mm(6)}` : undefined,
                                            }}
                                        />
                                    )}
                                </div>
                            );
                        })}
                    </div>
                </div>
            </div>
            <p className="as-preview-note">
                只显示题号与作答区，和打印出的答题卡一致；点作答区可定位到对应分区设置。
            </p>
        </div>
    );
}

/**
 * 答题卡模板设计器：模板与试卷分开维护，试卷只保存一个 templateId。
 * 内置模板只读，复制成自定义模板后可以改分区、栏数和作答区高度。
 * 右侧是按草稿配置实时重画的预览，改一个数字立刻能看到规制变化。
 */
export function AnswerSheetDesigner({templates, initialId, onClose, onChanged}) {
    const fallback = useMemo(
        () => templates.find((item) => item.id === initialId) || templates[0],
        [templates, initialId],
    );
    const [selectedId, setSelectedId] = useState(fallback?.id || '');
    const [draft, setDraft] = useState(() => (fallback ? clone(fallback) : newAnswerSheetTemplate()));
    const [busy, setBusy] = useState(false);
    const [activeRule, setActiveRule] = useState(-1);
    const [dragIndex, setDragIndex] = useState(-1);
    const [overIndex, setOverIndex] = useState(-1);

    const selected = templates.find((item) => item.id === selectedId);
    const readOnly = !!selected?.builtin;

    useEffect(() => {
        if (!selected) return;
        setDraft(clone(selected));
        setActiveRule(-1);
    }, [selected]);

    useEffect(() => {
        if (activeRule < 0) return;
        const node = document.getElementById(`as-rule-${activeRule}`);
        node?.scrollIntoView({block: 'nearest', behavior: 'smooth'});
    }, [activeRule]);

    const patchConfig = (patch) => {
        setDraft((current) => ({
            ...current,
            config: {
                ...current.config,
                ...patch,
                fields: patch.fields ?? current.config.fields,
                rules: patch.rules ?? current.config.rules,
            },
        }));
    };

    const patchRule = (index, patch) => {
        setDraft((current) => ({
            ...current,
            config: {
                ...current.config,
                rules: current.config.rules.map((rule, i) => (i === index ? {...rule, ...patch} : rule)),
            },
        }));
    };

    const toggleType = (index, type) => {
        const rule = draft.config.rules[index];
        const types = rule.types || [];
        patchRule(index, {
            types: types.includes(type) ? types.filter((item) => item !== type) : [...types, type],
        });
    };

    const moveRule = (index, direction) => {
        const rules = [...draft.config.rules];
        const target = index + direction;
        if (target < 0 || target >= rules.length) return;
        [rules[index], rules[target]] = [rules[target], rules[index]];
        patchConfig({rules});
        setActiveRule(target);
    };

    /** 拖拽把分区从 from 挪到 to 的位置（可视化排布，顺序即卷面顺序）。 */
    const reorderRule = (from, to) => {
        if (from < 0 || to < 0 || from === to) return;
        const rules = [...draft.config.rules];
        if (from >= rules.length || to >= rules.length) return;
        const [moved] = rules.splice(from, 1);
        rules.splice(to, 0, moved);
        patchConfig({rules});
        setActiveRule(to);
    };

    const endDrag = () => {
        if (dragIndex >= 0 && overIndex >= 0 && dragIndex !== overIndex) {
            reorderRule(dragIndex, overIndex);
        }
        setDragIndex(-1);
        setOverIndex(-1);
    };

    const addRule = () => {
        patchConfig({
            rules: [
                ...draft.config.rules,
                {title: '新分区', id: `rule-${Date.now().toString(36)}`, types: ['SOLVED'], style: 'box', columns: 1, heightMm: 48},
            ],
        });
        setActiveRule(draft.config.rules.length);
    };

    const removeRule = (index) => {
        patchConfig({rules: draft.config.rules.filter((_, i) => i !== index)});
        setActiveRule(-1);
    };

    const startNew = () => {
        const next = newAnswerSheetTemplate();
        setSelectedId(next.id);
        setDraft(next);
        setActiveRule(-1);
    };

    const duplicate = () => {
        const source = selected || draft;
        setDraft({
            ...clone(source),
            id: `custom-${Date.now().toString(36)}`,
            name: `${source.name}（副本）`,
            description: source.description || '',
            builtin: false,
        });
        setSelectedId('');
        setActiveRule(-1);
    };

    const save = async () => {
        if (!draft.name.trim()) {
            notify.warning('先给答题卡模板起个名字');
            return;
        }
        if (!draft.config.rules.length) {
            notify.warning('至少保留一个答题分区');
            return;
        }
        setBusy(true);
        try {
            const next = {
                ...draft,
                name: draft.name.trim(),
                description: draft.description.trim(),
                builtin: false,
            };
            const merged = await saveCustomAnswerSheets([
                ...templates.filter((item) => item.id !== next.id),
                next,
            ]);
            notify.success('答题卡模板已保存');
            onChanged?.(merged);
            setSelectedId(next.id);
            setDraft(clone(next));
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const remove = async () => {
        if (!selected || selected.builtin) return;
        const ok = await confirmDialog({
            title: '删除答题卡模板',
            message: `「${selected.name}」删除后，已绑定它的试卷会回退到通用模板。`,
            confirmText: '删除',
            danger: true,
        });
        if (!ok) return;
        setBusy(true);
        try {
            const merged = await saveCustomAnswerSheets(
                templates.filter((item) => item.id !== selected.id),
            );
            notify.success('答题卡模板已删除');
            onChanged?.(merged);
            const first = merged[0];
            setSelectedId(first?.id || '');
            setDraft(first ? clone(first) : newAnswerSheetTemplate());
            setActiveRule(-1);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    return (
        <Modal
            xwide
            bodyClassName="as-designer"
            title="答题卡模板"
            subtitle="答题卡单独设计，试卷只绑定模板；换模板不会改动卷面题目"
            onClose={onClose}
            footer={
                <>
                    <span className="muted as-foot-note">
                        {readOnly ? '内置模板只读，点「复制为自定义」后修改' : '保存后，绑定这份模板的试卷会自动重排答题卡'}
                    </span>
                    <div className="picker-actions">
                        <button className="btn ghost" onClick={onClose}>
                            关闭
                        </button>
                        <button className="btn ghost" onClick={duplicate}>
                            复制为自定义
                        </button>
                        {!readOnly && selected && (
                            <button className="btn danger" onClick={remove} disabled={busy}>
                                删除
                            </button>
                        )}
                        {!readOnly && (
                            <button className="btn primary" onClick={save} disabled={busy}>
                                {busy ? '正在保存…' : '保存模板'}
                            </button>
                        )}
                    </div>
                </>
            }
        >
            <aside className="as-list">
                <button className="btn ghost as-new" onClick={startNew}>
                    + 新建模板
                </button>
                {templates.map((item) => (
                    <button
                        key={item.id}
                        className={`as-list-item ${item.id === selectedId ? 'active' : ''}`}
                        onClick={() => {
                            setSelectedId(item.id);
                            setDraft(clone(item));
                        }}
                    >
                        <strong>{item.name}</strong>
                        <span>
                            {item.builtin ? <Pill tone="line">内置</Pill> : <Pill tone="ok">自定义</Pill>}
                            <span className="muted">{item.config?.rules?.length || 0} 个分区</span>
                        </span>
                    </button>
                ))}
            </aside>

            <div className="as-editor">
                <div className="form-grid">
                    <Field label="模板名">
                        <input
                            value={draft.name}
                            disabled={readOnly}
                            onChange={(event) =>
                                setDraft((current) => ({...current, name: event.target.value}))
                            }
                        />
                    </Field>
                    <Field label="排版">
                        <select
                            value={Number(draft.config.pageColumns) || 1}
                            disabled={readOnly}
                            onChange={(event) => patchConfig({pageColumns: Number(event.target.value)})}
                        >
                            <option value={1}>单栏</option>
                            <option value={2}>双栏</option>
                        </select>
                    </Field>
                    <Field label="模板类型" hint="固定规制按题型分区，自适应按卷内实际题目抽区">
                        <select
                            value={draft.config.mode || 'ADAPTIVE'}
                            disabled={readOnly}
                            onChange={(event) => patchConfig({mode: event.target.value})}
                        >
                            <option value="FIXED">固定规制</option>
                            <option value="ADAPTIVE">按题目形态自适应</option>
                        </select>
                    </Field>
                    <Field label="卷头字段" hint="用逗号分隔，如 班级,姓名,学号">
                        <input
                            value={(draft.config.fields || []).join(',')}
                            disabled={readOnly}
                            onChange={(event) =>
                                patchConfig({
                                    fields: event.target.value
                                        .split(/[,，]/)
                                        .map((item) => item.trim())
                                        .filter(Boolean),
                                })
                            }
                        />
                    </Field>
                    <Field label="适用说明" wide>
                        <input
                            value={draft.description || ''}
                            disabled={readOnly}
                            onChange={(event) =>
                                setDraft((current) => ({...current, description: event.target.value}))
                            }
                        />
                    </Field>
                    <Field label="作答须知" wide>
                        <textarea
                            rows={2}
                            value={draft.config.notice || ''}
                            disabled={readOnly}
                            onChange={(event) => patchConfig({notice: event.target.value})}
                        />
                    </Field>
                </div>

                <div className="as-rules-head">
                    <h4>答题分区</h4>
                    {!readOnly && (
                        <button className="btn ghost small" onClick={addRule}>
                            + 加一个分区
                        </button>
                    )}
                    {!readOnly && draft.config.rules.length > 1 && (
                        <span className="as-drag-hint">拖动左侧 ⋮⋮ 调整顺序，预览实时更新</span>
                    )}
                </div>
                <div className="as-rules">
                    {draft.config.rules.map((rule, index) => (
                        <div
                            className={[
                                'as-rule',
                                activeRule === index ? 'active' : '',
                                dragIndex === index ? 'dragging' : '',
                                overIndex === index && dragIndex !== index ? 'drag-over' : '',
                            ].filter(Boolean).join(' ')}
                            id={`as-rule-${index}`}
                            key={rule.id || index}
                            onFocus={() => setActiveRule(index)}
                            draggable={!readOnly}
                            onDragStart={(event) => {
                                setDragIndex(index);
                                event.dataTransfer.effectAllowed = 'move';
                                event.dataTransfer.setData('text/plain', String(index));
                            }}
                            onDragOver={(event) => {
                                if (readOnly || dragIndex < 0) return;
                                event.preventDefault();
                                event.dataTransfer.dropEffect = 'move';
                                setOverIndex(index);
                            }}
                            onDragLeave={() => setOverIndex((current) => (current === index ? -1 : current))}
                            onDrop={(event) => {
                                event.preventDefault();
                                const from = Number(event.dataTransfer.getData('text/plain'));
                                if (!Number.isNaN(from)) reorderRule(from, index);
                                setDragIndex(-1);
                                setOverIndex(-1);
                            }}
                            onDragEnd={endDrag}
                        >
                            <div className="as-rule-head">
                                {!readOnly && (
                                    <span className="as-drag-handle" title="拖动排序" aria-hidden="true">⋮⋮</span>
                                )}
                                <input
                                    value={rule.title}
                                    disabled={readOnly}
                                    onChange={(event) => patchRule(index, {title: event.target.value})}
                                    aria-label="分区标题"
                                />
                                <select
                                    value={rule.style}
                                    disabled={readOnly}
                                    onChange={(event) => patchRule(index, {style: event.target.value})}
                                    aria-label="作答样式"
                                >
                                    {ANSWER_SHEET_STYLES.map((style) => (
                                        <option key={style.value} value={style.value}>
                                            {style.label}
                                        </option>
                                    ))}
                                </select>
                                {!readOnly && (
                                    <div className="as-rule-actions">
                                        <button onClick={() => moveRule(index, -1)} disabled={index === 0} title="上移">
                                            ↑
                                        </button>
                                        <button
                                            onClick={() => moveRule(index, 1)}
                                            disabled={index === draft.config.rules.length - 1}
                                            title="下移"
                                        >
                                            ↓
                                        </button>
                                        <button onClick={() => removeRule(index)} title="删除">
                                            ✕
                                        </button>
                                    </div>
                                )}
                            </div>
                            <div className="as-rule-types">
                                {TYPES.map((type) => (
                                    <label key={type.value} className="pe-check">
                                        <input
                                            type="checkbox"
                                            checked={(rule.types || []).includes(type.value)}
                                            disabled={readOnly}
                                            onChange={() => toggleType(index, type.value)}
                                        />
                                        {type.label}
                                    </label>
                                ))}
                            </div>
                            <div className="as-rule-grid">
                                <Field label="涂卡列数" hint="每一栏里排几列">
                                    <input
                                        type="number"
                                        min="1"
                                        max="8"
                                        value={rule.columns || 1}
                                        disabled={readOnly || rule.style !== 'bubble'}
                                        onChange={(event) =>
                                            patchRule(index, {columns: Number(event.target.value) || 1})
                                        }
                                    />
                                </Field>
                                <Field label="作答区高度 mm">
                                    <input
                                        type="number"
                                        min="8"
                                        max="180"
                                        value={rule.heightMm || 0}
                                        disabled={readOnly || rule.style === 'bubble'}
                                        onChange={(event) =>
                                            patchRule(index, {heightMm: Number(event.target.value) || 0})
                                        }
                                    />
                                </Field>
                                <Field label="横线行数">
                                    <input
                                        type="number"
                                        min="1"
                                        max="30"
                                        value={rule.lines || 3}
                                        disabled={readOnly || rule.style !== 'lines'}
                                        onChange={(event) =>
                                            patchRule(index, {lines: Number(event.target.value) || 1})
                                        }
                                    />
                                </Field>
                            </div>
                        </div>
                    ))}
                </div>
            </div>

            <SheetPreview draft={draft} activeIndex={activeRule} onPick={setActiveRule}/>
        </Modal>
    );
}
