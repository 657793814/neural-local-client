import {useEffect, useMemo, useRef, useState} from 'react';
import {open} from '@tauri-apps/plugin-dialog';
import {
    coursewareAiGenerate,
    coursewareImageAdd,
    coursewareImageDir,
    coursewareParseDraft,
    coursewarePrompt,
    openPath,
    settingGet,
    settingSet,
} from '../lib/api.js';
import {splitList} from '../lib/format.js';
import {Field, notify} from '../components/ui.jsx';

const STORE_KEY = 'courseware.ai.workbench';
const FILE_TYPES = ['HTML', 'DOCX', 'PPTX', 'PDF'];

function customToText(values) {
    return Object.entries(values || {}).map(([key, value]) => `${key}=${value}`).join('\n');
}

function customFromText(text) {
    return Object.fromEntries(
        String(text || '')
            .split('\n')
            .map((line) => line.trim())
            .filter(Boolean)
            .map((line) => {
                const at = line.indexOf('=');
                return at > 0 ? [line.slice(0, at).trim(), line.slice(at + 1).trim()] : [line, ''];
            }),
    );
}

function cleanSpec(spec) {
    return {...spec, images: (spec.images || []).map((image) => ({...image, dataUrl: ''}))};
}

export default function CoursewareAi({baseSpec, subjects, onClose, onAccept}) {
    const [draftId, setDraftId] = useState(() => crypto.randomUUID());
    const [spec, setSpec] = useState(baseSpec);
    const [config, setConfig] = useState({
        referenceQuestions: '',
        referenceText: '',
        fileTypes: FILE_TYPES,
        slideCount: 12,
        documentLength: '约 6–8 页',
        outputDirectory: '',
        exampleCount: 3,
        exerciseCount: 4,
    });
    const [customText, setCustomText] = useState('');
    const [prompt, setPrompt] = useState('');
    const [promptCustom, setPromptCustom] = useState(false);
    const [visionImageId, setVisionImageId] = useState('');
    const [raw, setRaw] = useState('');
    const [externalRaw, setExternalRaw] = useState('');
    const [ready, setReady] = useState(false);
    const [busy, setBusy] = useState(false);
    const promptSeq = useRef(0);

    const patch = (values) => setSpec((current) => ({...current, ...values}));
    const promptInput = useMemo(() => ({
        spec: cleanSpec(spec),
        ...config,
        slideCount: Number(config.slideCount) || 12,
        exampleCount: Number(config.exampleCount) || 1,
        exerciseCount: Number(config.exerciseCount) || 1,
        customVariables: customFromText(customText),
    }), [spec, config, customText]);

    useEffect(() => {
        let alive = true;
        (async () => {
            try {
                const saved = await settingGet(STORE_KEY);
                if (saved) {
                    const data = JSON.parse(saved);
                    if (!alive) return;
                    if (data.draftId) setDraftId(data.draftId);
                    if (data.spec) setSpec({...baseSpec, ...data.spec});
                    if (data.config) setConfig((current) => ({...current, ...data.config}));
                    setCustomText(data.customText || customToText(data.config?.customVariables));
                    setPrompt(data.prompt || '');
                    setPromptCustom(Boolean(data.promptCustom));
                    setRaw(data.raw || '');
                    setVisionImageId(data.visionImageId || '');
                }
            } catch {
                // 旧草稿损坏不影响新建；下一次保存会写成新的 JSON。
            } finally {
                if (alive) setReady(true);
            }
        })();
        return () => { alive = false; };
        // baseSpec 只在打开工作台时作为缺省结构。
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    useEffect(() => {
        if (!ready || promptCustom) return undefined;
        const seq = ++promptSeq.current;
        const timer = setTimeout(async () => {
            try {
                const result = await coursewarePrompt(promptInput);
                if (seq === promptSeq.current) setPrompt(result.prompt);
            } catch (error) {
                if (seq === promptSeq.current) notify.error(error.message);
            }
        }, 180);
        return () => clearTimeout(timer);
    }, [ready, promptCustom, promptInput]);

    const persist = async (overrides = {}) => {
        await settingSet(STORE_KEY, JSON.stringify({
            draftId,
            spec: cleanSpec(spec),
            config,
            customText,
            prompt,
            promptCustom,
            raw,
            visionImageId,
            ...overrides,
        }));
    };

    const save = async () => {
        try {
            await persist();
            notify.success('提示词与变量草稿已保存');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const applyTemplate = async () => {
        try {
            const result = await coursewarePrompt(promptInput);
            setPrompt(result.prompt);
            setPromptCustom(false);
        } catch (error) {
            notify.error(error.message);
        }
    };

    const copy = async () => {
        try {
            await navigator.clipboard.writeText(prompt);
            notify.success('提示词已复制；可打开图片目录，把图片一并提供给外部智能体');
        } catch {
            notify.warning('复制失败，请在提示词框里全选后手动复制');
        }
    };

    const addImages = async () => {
        const picked = await open({
            multiple: true,
            filters: [{name: '本地图片', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif']}],
        });
        if (!picked) return;
        const paths = Array.isArray(picked) ? picked : [picked];
        setBusy(true);
        try {
            const added = [];
            for (const [index, path] of paths.entries()) {
                const id = `图${spec.images.length + added.length + 1}`;
                added.push(await coursewareImageAdd(draftId, id, path));
            }
            patch({images: [...spec.images, ...added]});
            if (added.length) notify.success(`已复制 ${added.length} 张图片到课件草稿目录`);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const openImageDir = async () => {
        try {
            await openPath(await coursewareImageDir(draftId));
        } catch (error) {
            notify.error(error.message);
        }
    };

    const patchImage = (index, values) => {
        patch({images: spec.images.map((image, at) => at === index ? {...image, ...values} : image)});
    };

    const generate = async () => {
        if (!spec.title.trim() || !spec.subject.trim()) {
            notify.warning('先填课题和学科');
            return;
        }
        setBusy(true);
        try {
            const result = await coursewareAiGenerate(promptInput, prompt, visionImageId);
            const latestRaw = result.raw || '';
            setRaw(latestRaw);
            if (!result.spec) {
                await persist({raw: latestRaw});
                notify.error(result.error || '模型返回无法整理成课件，请查看原始返回');
                return;
            }
            await persist({raw: latestRaw});
            notify.success('草稿已生成，请逐项校对后再入库');
            onAccept(result.spec);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const acceptExternal = async () => {
        if (!externalRaw.trim()) {
            notify.warning('先粘贴外部智能体返回的 JSON');
            return;
        }
        setBusy(true);
        try {
            const draft = await coursewareParseDraft(externalRaw, cleanSpec(spec));
            setRaw(externalRaw);
            await persist({raw: externalRaw});
            notify.success('外部草稿已解析，请逐项校对后再入库');
            onAccept(draft);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="modal-mask" onMouseDown={onClose}>
            <div className="modal cw-editor cw-ai" onMouseDown={(event) => event.stopPropagation()}>
                <div className="modal-head">
                    <div>
                        <h3>AI 生成课件</h3>
                        <p>调整教学变量与素材，复制提示词或调用模型，校对后再入库</p>
                    </div>
                    <button className="icon-btn" onClick={onClose} aria-label="关闭">✕</button>
                </div>
                <div className="modal-body cw-editor-body cw-ai-grid">
                    <div className="cw-ai-controls">
                        <section className="cw-sec">
                            <h4>课程与教学要求</h4>
                            <Field label="课题"><input value={spec.title} onChange={(e) => patch({title: e.target.value})}/></Field>
                            <div className="form-row">
                                <Field label="学科">
                                    <select value={spec.subjectId} onChange={(e) => {
                                        const subject = subjects.find((item) => item.id === e.target.value);
                                        patch({subjectId: e.target.value, subject: subject?.name || ''});
                                    }}>
                                        <option value="">请选择</option>
                                        {subjects.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
                                    </select>
                                </Field>
                                <Field label="年级"><input value={spec.grade} onChange={(e) => patch({grade: e.target.value})}/></Field>
                                <Field label="课时"><input value={spec.duration} placeholder="例如：1 课时 / 45 分钟" onChange={(e) => patch({duration: e.target.value})}/></Field>
                                <Field label="内容结构">
                                    <select value={spec.contentMode || 'REGULAR'} onChange={(e) => patch({contentMode: e.target.value})}>
                                        <option value="REGULAR">常规课程</option>
                                        <option value="TOPIC">专题课程</option>
                                    </select>
                                </Field>
                            </div>
                            <div className="form-row">
                                <Field label="章节"><input value={spec.chapter} onChange={(e) => patch({chapter: e.target.value})}/></Field>
                                <Field label="知识点" hint="顿号或逗号分隔"><input value={spec.knowledge.join('、')} onChange={(e) => patch({knowledge: splitList(e.target.value)})}/></Field>
                            </div>
                            <Field label="学生情况"><input value={spec.audience} placeholder="例如：已学相似三角形，计算容易跳步" onChange={(e) => patch({audience: e.target.value})}/></Field>
                            <Field label="教学目标" hint="一行一条"><textarea rows={3} value={(spec.objectives || []).join('\n')} onChange={(e) => patch({objectives: e.target.value.split('\n').filter(Boolean)})}/></Field>
                            <div className="form-row">
                                <Field label="难度"><input value={spec.difficulty} placeholder="例如：中等，最后一题拔高" onChange={(e) => patch({difficulty: e.target.value})}/></Field>
                                <Field label="教学风格"><input value={spec.teachingStyle} placeholder="例如：问题驱动、少讲多练" onChange={(e) => patch({teachingStyle: e.target.value})}/></Field>
                            </div>
                            <Field label="教学重点"><textarea rows={2} value={spec.teachingFocus} onChange={(e) => patch({teachingFocus: e.target.value})}/></Field>
                            <Field label="教学难点"><textarea rows={2} value={spec.teachingDifficulty} onChange={(e) => patch({teachingDifficulty: e.target.value})}/></Field>
                            <Field label="补充要求"><textarea rows={2} value={spec.customInstruction} onChange={(e) => patch({customInstruction: e.target.value})}/></Field>
                        </section>

                        <section className="cw-sec">
                            <h4>素材与输出</h4>
                            <div className="form-row">
                                <Field label="例题数量"><input type="number" min="1" max="12" value={config.exampleCount} onChange={(e) => setConfig({...config, exampleCount: e.target.value})}/></Field>
                                <Field label="练习数量"><input type="number" min="1" max="16" value={config.exerciseCount} onChange={(e) => setConfig({...config, exerciseCount: e.target.value})}/></Field>
                                <Field label="幻灯片参考页数"><input type="number" min="1" max="80" value={config.slideCount} onChange={(e) => setConfig({...config, slideCount: e.target.value})}/></Field>
                            </div>
                            <Field label="目标文件类型">
                                <div className="cw-file-types">
                                    {FILE_TYPES.map((type) => <label key={type}><input type="checkbox" checked={config.fileTypes.includes(type)} onChange={() => setConfig({...config, fileTypes: config.fileTypes.includes(type) ? config.fileTypes.filter((item) => item !== type) : [...config.fileTypes, type]})}/>{type}</label>)}
                                </div>
                            </Field>
                            <div className="form-row">
                                <Field label="文档篇幅"><input value={config.documentLength} onChange={(e) => setConfig({...config, documentLength: e.target.value})}/></Field>
                                <Field label="输出目录" hint="仅给有文件工具的外部智能体"><input value={config.outputDirectory} onChange={(e) => setConfig({...config, outputDirectory: e.target.value})}/></Field>
                            </div>
                            <Field label="参考题目"><textarea rows={3} value={config.referenceQuestions} onChange={(e) => setConfig({...config, referenceQuestions: e.target.value})}/></Field>
                            <Field label="参考文字"><textarea rows={3} value={config.referenceText} onChange={(e) => setConfig({...config, referenceText: e.target.value})}/></Field>
                            <Field label="自定义变量" hint="一行一个：变量名=内容"><textarea rows={2} value={customText} onChange={(e) => setCustomText(e.target.value)} placeholder={'school=南山中学\nactivity=小组实验'}/></Field>
                        </section>

                        <section className="cw-sec">
                            <h4>本地图片 {spec.images.length > 0 && <button className="btn small ghost" onClick={openImageDir}>打开图片目录</button>} <button className="btn small ghost" onClick={addImages} disabled={busy}>+ 添加图片</button></h4>
                            {spec.images.length === 0 && <p className="field-hint">可添加 PNG、JPEG、WebP、GIF；添加后会复制到本机课件目录。</p>}
                            {spec.images.map((image, index) => (
                                <div className="cw-item" key={`${image.id}-${index}`}>
                                    <div className="cw-item-head">
                                        <span className="cw-no">{image.id}</span>
                                        <button className="btn small ghost" onClick={() => openPath(image.sourcePath)}>{image.name || '打开原图'}</button>
                                        <button className="icon-btn tiny" onClick={() => patch({images: spec.images.filter((_, at) => at !== index)})}>✕</button>
                                    </div>
                                    <input placeholder="图片说明" value={image.caption} onChange={(e) => patchImage(index, {caption: e.target.value})}/>
                                    <div className="form-row">
                                        <input placeholder="使用位置，例如：新知讲解后" value={image.placement} onChange={(e) => patchImage(index, {placement: e.target.value})}/>
                                        <label className="cw-required"><input type="checkbox" checked={image.required} onChange={(e) => patchImage(index, {required: e.target.checked})}/>必须使用</label>
                                    </div>
                                </div>
                            ))}
                        </section>
                    </div>

                    <div className="cw-ai-output">
                        <section className="cw-sec">
                            <h4>{promptCustom ? '自定义提示词' : '通用提示词'} <button className="btn small ghost" onClick={applyTemplate}>重新套用模板</button></h4>
                            {promptCustom && <p className="field-hint">你已手改提示词；变量调整不会覆盖它，点击“重新套用模板”才更新。</p>}
                            <textarea className="cw-prompt" value={prompt} onChange={(e) => {setPrompt(e.target.value); setPromptCustom(true);}} spellCheck={false}/>
                            <div className="cw-prompt-actions">
                                <button className="btn" onClick={copy}>复制提示词</button>
                                <button className="btn" onClick={save}>保存草稿</button>
                            </div>
                        </section>
                        <section className="cw-sec">
                            <h4>调用模型</h4>
                            {spec.images.length > 0 && <Field label="附图给视觉模型" hint="只附一张；完整图片清单仍在提示词中"><select value={visionImageId} onChange={(e) => setVisionImageId(e.target.value)}><option value="">不附图，仅使用文字说明</option>{spec.images.map((image) => <option key={image.id} value={image.id}>{image.id} · {image.caption || image.name}</option>)}</select></Field>}
                            <button className="btn primary cw-generate" onClick={generate} disabled={busy || !ready}>{busy ? '正在生成…' : '调用模型生成并校对'}</button>
                        </section>
                        <details className="cw-external">
                            <summary>粘贴外部智能体的 JSON 回传</summary>
                            <textarea rows={8} value={externalRaw} onChange={(e) => setExternalRaw(e.target.value)} placeholder="粘贴 HandoutSpec JSON；本地图片与来源信息不会被覆盖"/>
                            <button className="btn" onClick={acceptExternal} disabled={busy}>解析并校对</button>
                        </details>
                        {raw && <details className="cw-external"><summary>查看上一次模型原始返回</summary><textarea rows={10} readOnly value={raw}/></details>}
                    </div>
                </div>
                <div className="modal-actions"><button className="btn" onClick={onClose}>关闭</button><button className="btn" onClick={save}>保存草稿</button></div>
            </div>
        </div>
    );
}
