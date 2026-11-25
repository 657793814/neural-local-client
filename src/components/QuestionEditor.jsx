import {useState} from 'react';
import {assetAdd, assetRemove, knowledgePointCreate, saveQuestion} from '../lib/api.js';
import {
    DIFFICULTIES,
    TYPES,
    choiceAnswerError,
    readFileAsDataUrl,
    reindexChoice,
    splitList,
} from '../lib/format.js';
import {NbImage} from '../lib/Math.jsx';
import {Field, Modal, notify} from './ui.jsx';
import {KnowledgePicker} from './KnowledgePicker.jsx';
import {DiagramDialog} from './DiagramDialog.jsx';
import {MathPreview} from './MathPreview.jsx';

const LETTERS = 'ABCDEFG'.split('');

function toForm(question, defaultSubjectId, tree) {
    const subjectId = question?.subjectId || defaultSubjectId || tree[0]?.id || '';
    return {
        id: question?.id,
        subjectId,
        type: question?.type || 'SINGLE',
        stem: question?.stem || '',
        options:
            question?.options?.length
                ? question.options.map((option) => ({key: option.key, text: option.text}))
                : [
                      {key: 'A', text: ''},
                      {key: 'B', text: ''},
                      {key: 'C', text: ''},
                      {key: 'D', text: ''},
                  ],
        answer: question?.answer || '',
        analysis: question?.analysis || '',
        difficulty: question?.difficulty || 3,
        tags: (question?.tags || []).join('；'),
        knowledgeIds: (question?.knowledge || []).map((point) => point.id),
        // 编辑器初开时 tree 可能还没刷新到新建的知识点，带上题目自带的名字，
        // 避免名字解析失败时把 id 当成名字显示出来（一串字符）。
        knowledgeNames: Object.fromEntries(
            (question?.knowledge || []).map((point) => [point.id, point.name]),
        ),
        source: question?.source || '',
        year: question?.year || '',
        reviewed: false,
    };
}

export function QuestionEditor({question, tree, defaultSubjectId, onClose, onSaved}) {
    const [form, setForm] = useState(() => toForm(question, defaultSubjectId, tree));
    const [pending, setPending] = useState([]);
    const [removed, setRemoved] = useState([]);
    const [saving, setSaving] = useState(false);
    const [diagramOpen, setDiagramOpen] = useState(false);
    const isChoice = form.type === 'SINGLE' || form.type === 'MULTI';

    const update = (patch) => setForm((current) => ({...current, ...patch}));

    const setOption = (index, text) => {
        const options = form.options.map((option, position) =>
            position === index ? {...option, text} : option,
        );
        update({options});
    };

    const addOption = () => {
        if (form.options.length >= 7) return;
        update({options: [...form.options, {key: LETTERS[form.options.length], text: ''}]});
    };

    const removeOption = (index) => {
        update(reindexChoice(form.options.filter((_, position) => position !== index), form.answer));
    };

    const onPaste = async (event) => {
        const files = [...(event.clipboardData?.files || [])].filter((file) =>
            file.type.startsWith('image/'),
        );
        if (files.length === 0) return;
        event.preventDefault();
        const images = await Promise.all(files.map((file) => readFileAsDataUrl(file)));
        setPending((current) => [...current, ...images]);
        notify.info('图片已加到「配图」，保存后挂在题目下面');
    };

    const submit = async () => {
        if (!form.stem.trim()) {
            notify.error('题干不能为空');
            return;
        }
        if (isChoice && form.options.filter((option) => option.text.trim()).length < 2) {
            notify.error('选择题至少要两个有内容的选项');
            return;
        }
        const choice = isChoice
            ? reindexChoice(form.options.filter((option) => option.text.trim()), form.answer, false)
            : {options: [], answer: form.answer};
        const answerError = choiceAnswerError(form.type, choice.options, choice.answer);
        const publishing = form.reviewed || (question && question.status !== 'DRAFT');
        if (publishing) {
            const missing = [];
            if (!choice.answer.trim()) missing.push('答案');
            if (!form.analysis.trim()) missing.push('解析');
            if (form.knowledgeIds.length === 0) missing.push('知识点');
            if (missing.length > 0) {
                notify.error(`正式入库前请补齐：${missing.join('、')}`);
                return;
            }
            if (answerError) {
                notify.error(answerError);
                return;
            }
        } else if (answerError) {
            notify.warning(`${answerError}；本次仍保存为草稿`);
        }
        if (isChoice) update(choice);

        let questionSaved = false;
        setSaving(true);
        try {
            const id = await saveQuestion({
                id: form.id,
                subjectId: form.subjectId || null,
                type: form.type,
                stem: form.stem,
                options: choice.options,
                answer: choice.answer,
                analysis: form.analysis,
                difficulty: Number(form.difficulty) || 3,
                tags: splitList(form.tags),
                knowledgeIds: form.knowledgeIds,
                source: form.source || null,
                year: form.year ? Number(form.year) : null,
                // 手工录题先进草稿箱；自己勾了「已核对」才直接入库（通过复核）。
                status: form.reviewed ? 'PUBLISHED' : question?.status || 'DRAFT',
                origin: question?.origin || 'MANUAL',
                needsReview: form.reviewed ? false : question ? !!question.needsReview : true,
                reviewNote: form.reviewed ? null : question?.reviewNote || null,
            });
            questionSaved = true;
            if (id !== form.id) update({id});

            const remainingPending = [...pending];
            for (const dataUrl of pending) {
                await assetAdd(id, dataUrl, 'FIGURE');
                remainingPending.shift();
                setPending([...remainingPending]);
            }
            const remainingRemoved = [...removed];
            for (const assetId of removed) {
                await assetRemove(assetId);
                remainingRemoved.shift();
                setRemoved([...remainingRemoved]);
            }
            notify.success(
                form.reviewed
                    ? '已保存并入库'
                    : question
                        ? '已保存'
                        : '已存入草稿箱，复核通过后才是正式入库',
            );
            onSaved?.(id);
            onClose?.();
        } catch (error) {
            notify.error(
                questionSaved
                    ? `题目已保存，配图还没处理完：${error.message}。请再次保存继续。`
                    : error.message,
            );
        } finally {
            setSaving(false);
        }
    };

    return (
        <>
        <Modal
            wide
            title={question ? '编辑题目' : '手工录题'}
            subtitle="公式用 $...$ 包裹，例如 $x^2+1$；输入框下面会实时显示渲染效果，图片可以粘贴到题干框里"
            onClose={onClose}
            footer={
                <>
                    <button className="btn ghost" onClick={onClose}>
                        取消
                    </button>
                    <button className="btn primary" onClick={submit} disabled={saving}>
                        {saving ? '保存中…' : form.reviewed ? '保存并入库' : question ? '保存' : '存为草稿'}
                    </button>
                </>
            }
        >
            <div className="form-grid">
                <Field label="学科">
                    <select value={form.subjectId} onChange={(event) => update({subjectId: event.target.value, knowledgeIds: []})}>
                        <option value="">未分类</option>
                        {tree.map((subject) => (
                            <option key={subject.id} value={subject.id}>
                                {subject.name}
                            </option>
                        ))}
                    </select>
                </Field>
                <Field label="题型">
                    <select value={form.type} onChange={(event) => update({type: event.target.value})}>
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
                        {DIFFICULTIES.map((item) => (
                            <option key={item.value} value={item.value}>
                                {item.label}
                            </option>
                        ))}
                    </select>
                </Field>
                <Field label="来源" hint="例如「某校月考题」，可留空">
                    <input value={form.source} onChange={(event) => update({source: event.target.value})}/>
                </Field>

                <Field label="题干" wide>
                    <textarea
                        rows={5}
                        value={form.stem}
                        onChange={(event) => update({stem: event.target.value})}
                        onPaste={onPaste}
                        placeholder="把题目粘进来，图片可以直接粘贴"
                    />
                    <MathPreview text={form.stem}/>
                </Field>

                {isChoice && (
                    <Field label="选项" wide hint="最多 7 个；多选题答案写成 AC">
                        <div className="option-list">
                            {form.options.map((option, index) => (
                                <div className="option-block" key={index}>
                                    <div className="option-row">
                                        <span className="option-key">{LETTERS[index]}</span>
                                        <input value={option.text} onChange={(event) => setOption(index, event.target.value)}/>
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

                <Field label="答案" wide hint="选择题填字母；填空题填结果；解答题可写分问结论">
                    <input value={form.answer} onChange={(event) => update({answer: event.target.value})}/>
                    <MathPreview text={form.answer}/>
                </Field>
                <Field label="解析" wide>
                    <textarea
                        rows={4}
                        value={form.analysis}
                        onChange={(event) => update({analysis: event.target.value})}
                    />
                    <MathPreview text={form.analysis}/>
                </Field>

                <Field label="自由标签" wide hint="最多 12 个，用来自己分类">
                    <input value={form.tags} onChange={(event) => update({tags: event.target.value})}/>
                </Field>

                <Field label="知识点" wide hint="决定题目归到哪一章；可搜索、最多 6 个；入库至少要有 1 个">
                    <KnowledgePicker
                        tree={tree}
                        subjectId={form.subjectId}
                        value={form.knowledgeIds}
                        knownNames={form.knowledgeNames}
                        onChange={(ids) => update({knowledgeIds: ids})}
                        onCreate={(name) => knowledgePointCreate(form.subjectId, name)}
                    />
                </Field>

                <Field label="配图" wide hint="几何图形、函数图象放这里；原图会一直保留">
                    <div className="asset-row">
                        {(question?.assets || []).filter((asset) => !removed.includes(asset.id)).map((asset) => (
                            <div className="asset-thumb" key={asset.id}>
                                <NbImage id={asset.id} alt="配图"/>
                                <button
                                    className="asset-remove"
                                    onClick={() => {
                                        setRemoved((current) => [...current, asset.id]);
                                        notify.warning('保存后才会真正移除这张图');
                                    }}
                                >
                                    移除
                                </button>
                            </div>
                        ))}
                        {pending.map((dataUrl, index) => (
                            <div className="asset-thumb" key={index}>
                                <img src={dataUrl} alt="待保存的配图"/>
                                <button
                                    className="asset-remove"
                                    onClick={() => setPending((current) => current.filter((_, i) => i !== index))}
                                >
                                    移除
                                </button>
                            </div>
                        ))}
                        <label className="asset-add">
                            + 选图片
                            <input
                                type="file"
                                accept="image/png,image/jpeg,image/webp,image/gif"
                                hidden
                                onChange={async (event) => {
                                    const file = event.target.files?.[0];
                                    if (!file) return;
                                    const dataUrl = await readFileAsDataUrl(file);
                                    setPending((current) => [...current, dataUrl]);
                                }}
                            />
                        </label>
                        <button
                            type="button"
                            className="asset-add asset-diagram"
                            onClick={() => setDiagramOpen(true)}
                        >
                            ✎ AI 制图
                        </button>
                    </div>
                </Field>

                {(!question || question.needsReview) && (
                    <label className="check wide">
                        <input
                            type="checkbox"
                            checked={form.reviewed}
                            onChange={(event) => update({reviewed: event.target.checked})}
                        />
                        我已核对并补齐答案、解析和知识点，直接入库
                    </label>
                )}
            </div>
        </Modal>

        {diagramOpen && (
            <DiagramDialog
                stem={form.stem}
                subjectId={form.subjectId}
                refName="配图"
                onInsert={(svg, dataUrl) => {
                    setPending((current) => [...current, dataUrl]);
                    notify.success('示意图已加进配图，保存后随题目入库');
                }}
                onClose={() => setDiagramOpen(false)}
            />
        )}
        </>
    );
}
