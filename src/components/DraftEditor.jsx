import {useState} from 'react';
import {DIFFICULTIES, TYPES, choiceAnswerError, reindexChoice} from '../lib/format.js';
import {knowledgePointCreate} from '../lib/api.js';
import {Field, notify} from './ui.jsx';
import {KnowledgePicker} from './KnowledgePicker.jsx';
import {DiagramDialog} from './DiagramDialog.jsx';
import {MathPreview} from './MathPreview.jsx';

/**
 * 预览里就地改一道题（还没存草稿的时候）。
 *
 * 和题库里的 QuestionEditor 不是一回事：它只改任务结果、不写库，
 * 所以没有必要带上来源、标签这些「归属」字段，只留能让人把题
 * 从「不能存」改成「能存」的那几项。AI 制图生成的图以 {ref, path}
 * 挂在 item.figures 上，正文里用 ![](ref) 引用，存草稿时一起入库。
 */

const LETTERS = 'ABCDEFG'.split('');

function toForm(item) {
    return {
        type: item.type || 'SOLVED',
        difficulty: Number(item.difficulty) || 3,
        stem: item.stem || '',
        options: (item.options || []).map((option) => ({key: option.key, text: option.text})),
        answer: item.answer || '',
        analysis: item.analysis || '',
        knowledgeIds: item.knowledgeIds || [],
    };
}

/** 按现有正文里已经用到的「图N」算出下一个引用名，避免重名。 */
function nextRef(parts) {
    const used = new Set();
    for (const part of parts) {
        const matches = String(part || '').matchAll(/!\[[^\]]*\]\((图\d+)\)/g);
        for (const match of matches) used.add(match[1]);
    }
    let n = 1;
    while (used.has(`图${n}`)) n += 1;
    return `图${n}`;
}

export function DraftEditor({item, tree, onSave, onCancel, saving = false}) {
    const [form, setForm] = useState(() => toForm(item));
    const [figures, setFigures] = useState(() => (item.figures || []).slice());
    const [diagramTarget, setDiagramTarget] = useState(null);
    const isChoice = form.type === 'SINGLE' || form.type === 'MULTI';

    const update = (patch) => setForm((current) => ({...current, ...patch}));

    /** 从解答题改成选择题时，空选项表会让人一头雾水：先摆四个空位出来。 */
    const changeType = (type) => {
        const next = type === 'SINGLE' || type === 'MULTI';
        update({
            type,
            options:
                next && form.options.length < 2
                    ? ['A', 'B', 'C', 'D'].map((key) => ({key, text: ''}))
                    : form.options,
        });
    };

    const setOption = (index, text) => {
        update({
            options: form.options.map((option, position) =>
                position === index ? {...option, text} : option,
            ),
        });
    };

    const addOption = () => {
        if (form.options.length >= 7) return;
        update({options: [...form.options, {key: LETTERS[form.options.length], text: ''}]});
    };

    const removeOption = (index) => {
        update(reindexChoice(form.options.filter((_, position) => position !== index), form.answer));
    };

    /** 打开制图弹窗：target 可以是 'stem' / 'analysis' / 选项下标。 */
    const openDiagram = (target) => setDiagramTarget(target);

    /** 制图完成：把引用插到目标文本末尾，并登记 figure。 */
    const insertDiagram = (svg, dataUrl, path, refName) => {
        const tag = `\n\n![](${refName})`;
        if (diagramTarget === 'stem') {
            update({stem: (form.stem || '').trimEnd() + tag});
        } else if (diagramTarget === 'analysis') {
            update({analysis: (form.analysis || '').trimEnd() + tag});
        } else if (typeof diagramTarget === 'number' && form.options[diagramTarget]) {
            const index = diagramTarget;
            update({
                options: form.options.map((option, position) =>
                    position === index ? {...option, text: (option.text || '').trimEnd() + ` ![](${refName})`} : option,
                ),
            });
        }
        setFigures((current) => {
            const next = current.filter((figure) => figure.ref !== refName);
            next.push({ref: refName, path});
            return next;
        });
        notify.success(`${refName} 已插入，保存后随题目一起入库`);
    };

    // 制图弹窗预填：题干目标用题干，选项目标用该选项文本。
    const diagramStem =
        diagramTarget === 'stem'
            ? form.stem
            : diagramTarget === 'analysis'
              ? form.analysis
              : typeof diagramTarget === 'number'
                ? form.options[diagramTarget]?.text || form.stem
                : form.stem;
    const diagramRef = nextRef([form.stem, form.analysis, ...form.options.map((option) => option.text)]);

    const submit = async () => {
        if (!form.stem.trim()) {
            notify.error('题干不能为空');
            return;
        }
        if (isChoice) {
            const filled = form.options.filter((option) => option.text.trim());
            if (filled.length < 2) {
                notify.error('选择题至少要两个有内容的选项');
                return;
            }
        }
        const choice = isChoice
            ? reindexChoice(form.options.filter((option) => option.text.trim()), form.answer, false)
            : {options: [], answer: form.answer};
        const answerError = choiceAnswerError(form.type, choice.options, choice.answer);
        if (answerError) notify.warning(`${answerError}；请先保存草稿，再修正`);
        await onSave?.({
            type: form.type,
            difficulty: Number(form.difficulty) || 3,
            stem: form.stem,
            options: choice.options,
            answer: choice.answer,
            analysis: form.analysis,
            knowledgeIds: form.knowledgeIds,
            figures,
        });
    };

    return (
        <div className="draft-editor">
            <div className="draft-row">
                <Field label="题型">
                    <select value={form.type} onChange={(event) => changeType(event.target.value)}>
                        {TYPES.map((type) => (
                            <option key={type.value} value={type.value}>
                                {type.label}
                            </option>
                        ))}
                    </select>
                </Field>
                <Field label="难度">
                    <select
                        value={form.difficulty}
                        onChange={(event) => update({difficulty: Number(event.target.value)})}
                    >
                        {DIFFICULTIES.map((level) => (
                            <option key={level.value} value={level.value}>
                                {level.label}
                            </option>
                        ))}
                    </select>
                </Field>
            </div>

            <Field label="题干" wide>
                <div className="draft-field-row">
                    <textarea
                        rows={4}
                        value={form.stem}
                        onChange={(event) => update({stem: event.target.value})}
                        placeholder="从原图抄下来的题干，公式用 $...$ 包裹"
                    />
                    <button type="button" className="btn ghost small" onClick={() => openDiagram('stem')}>
                        AI 制图
                    </button>
                </div>
                <MathPreview text={form.stem}/>
            </Field>

            {isChoice && (
                <Field label="选项" wide hint="最多 7 个；每个选项都能单独配图">
                    <div className="option-list">
                        {form.options.map((option, index) => (
                            <div className="option-block" key={index}>
                                <div className="option-row">
                                    <span className="option-key">{LETTERS[index]}</span>
                                    <input
                                        value={option.text}
                                        onChange={(event) => setOption(index, event.target.value)}
                                    />
                                    <button
                                        type="button"
                                        className="btn ghost small"
                                        title="给这个选项配图"
                                        onClick={() => openDiagram(index)}
                                    >
                                        图
                                    </button>
                                    <button className="icon-btn" onClick={() => removeOption(index)} aria-label="删除选项">
                                        ✕
                                    </button>
                                </div>
                                <MathPreview text={option.text} label={`选项 ${LETTERS[index]} 预览`} compact/>
                            </div>
                        ))}
                        {form.options.length < 7 && (
                            <button className="btn ghost small" onClick={addOption}>
                                + 加一个选项
                            </button>
                        )}
                    </div>
                </Field>
            )}

            <Field label="答案" wide hint="选择题填字母，多选写 AC；其它题型写最终结论">
                <input value={form.answer} onChange={(event) => update({answer: event.target.value})}/>
                <MathPreview text={form.answer}/>
            </Field>

            <Field label="解析" wide>
                <div className="draft-field-row">
                    <textarea
                        rows={3}
                        value={form.analysis}
                        onChange={(event) => update({analysis: event.target.value})}
                    />
                    <button type="button" className="btn ghost small" onClick={() => openDiagram('analysis')}>
                        AI 制图
                    </button>
                </div>
                <MathPreview text={form.analysis}/>
            </Field>

            {figures.length > 0 && (
                <p className="muted small">已配 {figures.length} 张图：{figures.map((figure) => figure.ref).join('、')}，保存后随题目入库</p>
            )}

            <Field label="知识点" wide hint="决定题目归到哪一章；可搜索、最多 6 个">
                <KnowledgePicker
                    tree={tree}
                    subjectId={item.subjectId}
                    value={form.knowledgeIds}
                    onChange={(ids) => update({knowledgeIds: ids})}
                    onCreate={(name) => knowledgePointCreate(item.subjectId, name)}
                />
            </Field>

            <div className="draft-actions">
                <button className="btn ghost small" onClick={onCancel} disabled={saving}>
                    取消
                </button>
                <button className="btn primary small" onClick={submit} disabled={saving}>
                    {saving ? '保存中…' : '保存修改'}
                </button>
            </div>

            {diagramTarget !== null && (
                <DiagramDialog
                    stem={diagramStem}
                    subjectId={item.subjectId}
                    refName={diagramRef}
                    onInsert={insertDiagram}
                    onClose={() => setDiagramTarget(null)}
                />
            )}
        </div>
    );
}
