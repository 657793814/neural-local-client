import {useCallback, useEffect, useMemo, useState} from 'react';
import {open} from '@tauri-apps/plugin-dialog';
import {
    classList,
    coursewareAiDraft,
    coursewareAssign,
    coursewareBuild,
    coursewareDelete,
    coursewareExport,
    coursewareImport,
    coursewareImportCommit,
    coursewareImportSave,
    coursewareList,
    coursewarePreview,
    coursewareSpec,
    coursewareUnassign,
    coursewareUpdate,
    listQuestions,
} from '../lib/api.js';
import {cx, formatTime, pickSavePath, splitList} from '../lib/format.js';
import {RichInline} from '../lib/Math.jsx';
import CoursewareAi from './CoursewareAi.jsx';
import CoursewareImport from './CoursewareImport.jsx';
import {
    Card,
    Empty,
    Field,
    Modal,
    PageHead,
    Pill,
    Tabs,
    confirmDialog,
    notify,
} from '../components/ui.jsx';

const THEMES = [
    {value: 'paper', label: '米纸（默认）'},
    {value: 'science', label: '科学蓝'},
    {value: 'chalk', label: '黑板绿'},
];

const SOURCE_TABS = [
    {value: '', label: '全部'},
    {value: 'LOCAL_BUILD', label: '我做的讲义'},
    {value: 'IMPORT', label: '导入的课件'},
];

function emptyPoint() {
    return {name: '', summary: '', detail: '', misconception: '', formula: ''};
}

function emptyItem() {
    return {questionId: '', stem: '', answer: '', analysis: '', steps: []};
}

function emptyThink() {
    return {prompt: '', hint: '', answer: ''};
}

function emptySpec() {
    return {
        title: '',
        subjectId: '',
        subject: '',
        grade: '',
        chapter: '',
        description: '',
        knowledge: [],
        objectives: ['', '', ''],
        warmup: '',
        points: [emptyPoint()],
        examples: [emptyItem()],
        variants: [],
        challenge: [],
        thinking: [emptyThink()],
        summary: '',
        homework: [''],
        theme: 'paper',
        contentMode: 'REGULAR',
        duration: '',
        audience: '',
        difficulty: '',
        teachingFocus: '',
        teachingDifficulty: '',
        teachingStyle: '',
        customInstruction: '',
        images: [],
        source: {
            originalPath: '',
            sha256: '',
            locations: [],
            mode: 'FORMAT_ONLY',
            warnings: [],
        },
        aiAdded: [],
    };
}

const EXPORT_FORMATS = [
    {kind: 'html', label: 'HTML', hint: '保留完整版式，可离线打开和打印'},
    {kind: 'docx', label: 'Word（DOCX）', hint: '在 Word 或 WPS 中继续编辑', spec: true},
    {kind: 'pptx', label: 'PowerPoint（PPTX）', hint: '按教学内容生成幻灯片', spec: true},
    {kind: 'pdf', label: 'PDF', hint: '用于打印、分享和投屏'},
    {kind: 'json', label: '课件草稿（JSON）', hint: '备份可继续校对的结构化内容', spec: true},
];

function normalizeSpec(data = {}) {
    const base = emptySpec();
    return {
        ...base,
        ...data,
        source: {...base.source, ...(data.source || {})},
        images: (data.images || []).map((image) => ({dataUrl: '', ...image})),
        objectives: data.objectives || base.objectives,
        points: data.points?.length ? data.points : base.points,
        examples: data.examples?.length ? data.examples : base.examples,
        variants: data.variants || [],
        challenge: data.challenge || [],
        thinking: data.thinking?.length ? data.thinking : base.thinking,
        homework: data.homework?.length ? data.homework : base.homework,
    };
}

/** 知识点/目标/作业用一行一个的文本框编辑，存取时拆回数组。 */
function linesToText(list) {
    return (list || []).filter(Boolean).join('\n');
}

function textToLines(text) {
    return String(text || '')
        .split('\n')
        .map((line) => line.trim())
        .filter(Boolean);
}

export default function Courseware({tree, onGo, onChanged}) {
    const [rows, setRows] = useState([]);
    const [loading, setLoading] = useState(true);
    const [keyword, setKeyword] = useState('');
    const [subjectId, setSubjectId] = useState('');
    const [source, setSource] = useState('');
    const [preview, setPreview] = useState(null);
    const [assignFor, setAssignFor] = useState(null);
    const [metaFor, setMetaFor] = useState(null);
    const [specForm, setSpecForm] = useState(null);
    const [specId, setSpecId] = useState(null);
    const [gates, setGates] = useState(null);
    const [busy, setBusy] = useState(false);
    const [picker, setPicker] = useState(null);
    const [aiOpen, setAiOpen] = useState(false);
    const [importOpen, setImportOpen] = useState(false);
    const [importReview, setImportReview] = useState(null);
    const [exportFor, setExportFor] = useState(null);
    const [exportBusy, setExportBusy] = useState('');

    const subjects = useMemo(
        () => (tree || []).map((item) => ({id: item.id, name: item.name})),
        [tree],
    );

    const load = useCallback(async () => {
        setLoading(true);
        try {
            setRows(await coursewareList(keyword, subjectId, source));
        } catch (error) {
            notify.error(error.message);
        } finally {
            setLoading(false);
        }
    }, [keyword, subjectId, source]);

    useEffect(() => {
        load();
    }, [load]);

    // ---------------------------------------------------------------- 导入
    const importFile = async () => {
        const picked = await open({
            multiple: false,
            filters: [{name: 'HTML 课件', extensions: ['html', 'htm']}],
        });
        if (!picked) return;
        try {
            const result = await coursewareImport({path: picked});
            if (result.duplicated) {
                notify.warning(`库里已经有这份课件了：${result.title}`);
            } else {
                notify.success(`已导入：${result.title}`);
            }
            (result.warnings || []).forEach((warning) => notify.warning(warning));
            await load();
            onChanged?.();
        } catch (error) {
            notify.error(error.message);
        }
    };

    // ---------------------------------------------------------------- 预览
    const openPreview = async (row) => {
        try {
            setPreview(await coursewarePreview(row.id));
        } catch (error) {
            notify.error(error.message);
        }
    };

    // ---------------------------------------------------------------- 元信息
    const openMeta = (row) => {
        setMetaFor({
            id: row.id,
            title: row.title,
            subjectId: row.subjectId || '',
            grade: row.grade || '',
            description: row.description || '',
            knowledge: (row.knowledge || []).join('、'),
        });
    };

    const saveMeta = async () => {
        if (!metaFor.title.trim()) {
            notify.warning('先起个名字');
            return;
        }
        setBusy(true);
        try {
            await coursewareUpdate({
                id: metaFor.id,
                title: metaFor.title,
                subjectId: metaFor.subjectId || null,
                grade: metaFor.grade,
                description: metaFor.description,
                knowledge: splitList(metaFor.knowledge),
            });
            setMetaFor(null);
            notify.success('已保存');
            await load();
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    // ---------------------------------------------------------------- 布置
    const saveAssign = async () => {
        if (!assignFor.classIds.length) {
            notify.warning('至少选一个班级');
            return;
        }
        setBusy(true);
        try {
            await coursewareAssign({
                coursewareId: assignFor.id,
                classIds: assignFor.classIds,
                dueAt: assignFor.dueAt || null,
                note: assignFor.note,
            });
            setAssignFor(null);
            notify.success('已布置');
            await load();
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const removeAssign = async (coursewareId, classId) => {
        try {
            await coursewareUnassign(coursewareId, classId);
            await load();
        } catch (error) {
            notify.error(error.message);
        }
    };

    const remove = async (row) => {
        const yes = await confirmDialog({
            title: '删除课件',
            message: `确定删除「${row.title}」？布置记录一并清掉，文件暂留在磁盘上。`,
            danger: true,
            confirmText: '删除',
        });
        if (!yes) return;
        try {
            await coursewareDelete(row.id);
            notify.success('已删除');
            await load();
            onChanged?.();
        } catch (error) {
            notify.error(error.message);
        }
    };

    // ---------------------------------------------------------------- 导出
    const exportFile = async (row, format) => {
        const safeTitle = row.title.replace(/[\\/:*?"<>|]/g, '_').trim() || '课件';
        const path = await pickSavePath(`${safeTitle}.${format.kind}`, [{
            name: format.label,
            extensions: [format.kind],
        }]);
        if (!path) return;
        setExportBusy(`${row.id}:${format.kind}`);
        try {
            const outcome = await coursewareExport(row.id, format.kind, path);
            setExportFor(null);
            if (outcome.needsPrint) {
                await openPreview(row);
                notify.warning(outcome.message);
            } else {
                notify.success(outcome.message || `已导出 ${format.label}`);
            }
        } catch (error) {
            notify.error(error.message);
        } finally {
            setExportBusy('');
        }
    };

    // ---------------------------------------------------------------- 讲义编辑器
    const newSpec = () => {
        setImportReview(null);
        setSpecId(null);
        setGates(null);
        setSpecForm(emptySpec());
    };

    const acceptAiSpec = (draft) => {
        setAiOpen(false);
        setImportReview(null);
        setSpecId(null);
        setGates(null);
        setSpecForm(normalizeSpec(draft));
    };

    const reviewImport = (item) => {
        if (!item.draft) {
            notify.warning('这份文件还没有可校对的草稿，请先重试');
            return;
        }
        setImportOpen(false);
        setImportReview(item);
        setSpecId(null);
        setGates(null);
        setSpecForm(normalizeSpec(item.draft));
    };

    const closeSpec = () => {
        setSpecForm(null);
        setImportReview(null);
        setGates(null);
    };

    const editSpec = async (row) => {
        try {
            const data = await coursewareSpec(row.id);
            if (!data) {
                notify.warning('这份课件是导入的，没有讲义骨架，改不了内容');
                return;
            }
            setImportReview(null);
            setSpecId(row.id);
            setGates(null);
            setSpecForm(normalizeSpec(data));
        } catch (error) {
            notify.error(error.message);
        }
    };

    const patchItem = (list, index, patch) => {
        setSpecForm((form) => ({
            ...form,
            [list]: form[list].map((item, i) => (i === index ? {...item, ...patch} : item)),
        }));
    };

    const addRow = (list, factory) => {
        setSpecForm((form) => ({...form, [list]: [...form[list], factory()]}));
    };

    const dropRow = (list, index) => {
        setSpecForm((form) => ({...form, [list]: form[list].filter((_, i) => i !== index)}));
    };

    /** 从题库挑一道题填进例题/变式/拔高条目。 */
    const pickQuestion = async (list, index) => {
        setPicker({list, index, keyword: '', rows: [], loading: true});
        try {
            const result = await listQuestions({keyword: '', size: 30});
            setPicker((current) => ({...current, rows: result.items || result, loading: false}));
        } catch (error) {
            setPicker(null);
            notify.error(error.message);
        }
    };

    const searchPicker = async () => {
        setPicker((current) => ({...current, loading: true}));
        try {
            const result = await listQuestions({keyword: picker.keyword, size: 30});
            setPicker((current) => ({...current, rows: result.items || result, loading: false}));
        } catch (error) {
            notify.error(error.message);
        }
    };

    const chooseQuestion = (question) => {
        patchItem(picker.list, picker.index, {
            questionId: question.id,
            stem: question.stem || '',
            answer: question.answer || '',
            analysis: question.analysis || '',
        });
        setPicker(null);
        notify.success('已挂上题库题，编译时会把题干与解析内联进讲义');
    };

    /** AI 补题：拿第一道例题当锚点，补变式/拔高/思维训练。 */
    const aiDraft = async () => {
        const anchor = (specForm.examples || []).find((item) => item.stem || item.questionId);
        if (!anchor) {
            notify.warning('先写好一道例题，AI 才好照着补变式');
            return;
        }
        setBusy(true);
        notify.info('正在补题，稍等…');
        try {
            const draft = await coursewareAiDraft({
                subject: specForm.subject || subjects.find((s) => s.id === specForm.subjectId)?.name || '',
                grade: specForm.grade,
                knowledge: splitList(specForm.knowledge.join('、')),
                example: anchor.stem || anchor.answer,
                want: ['变式', '拔高', '思维训练'],
            });
            setSpecForm((form) => ({
                ...form,
                variants: [
                    ...form.variants,
                    ...(draft.variants || []).map((item) => ({...emptyItem(), ...item})),
                ],
                challenge: [
                    ...form.challenge,
                    ...(draft.challenge || []).map((item) => ({...emptyItem(), ...item})),
                ],
                thinking: [
                    ...form.thinking,
                    ...(draft.thinking || []).map((item) => ({...emptyThink(), ...item})),
                ],
            }));
            const count =
                (draft.variants || []).length +
                (draft.challenge || []).length +
                (draft.thinking || []).length;
            notify.success(`AI 补了 ${count} 道草稿，过目改完再生成`);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const specPayload = () => ({
        ...specForm,
        knowledge: splitList(specForm.knowledge.join('、')),
        objectives: textToLines(linesToText(specForm.objectives)),
        points: specForm.points.filter((point) => point.name.trim()),
        examples: specForm.examples.filter((item) => item.stem.trim() || item.questionId),
        variants: specForm.variants.filter((item) => item.stem.trim() || item.questionId),
        challenge: specForm.challenge.filter((item) => item.stem.trim() || item.questionId),
        thinking: specForm.thinking.filter((item) => item.prompt.trim()),
        homework: textToLines(linesToText(specForm.homework)),
    });

    const saveImportDraft = async () => {
        if (!importReview) return;
        if (!specForm.title.trim()) {
            notify.warning('先填课题');
            return;
        }
        setBusy(true);
        try {
            const saved = await coursewareImportSave(importReview.id, specPayload());
            setImportReview(saved);
            setSpecForm(normalizeSpec(saved.draft));
            notify.success('校对草稿已保存，尚未入库');
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const compile = async () => {
        if (!specForm.title.trim()) {
            notify.warning('先填课题');
            return;
        }
        setBusy(true);
        try {
            const spec = specPayload();
            const outcome = importReview
                ? await coursewareImportCommit(importReview.id, spec)
                : await coursewareBuild(spec, specId);
            setGates(outcome.gates || []);
            if (!outcome.ok) {
                notify.error(outcome.message || '有内容还没补齐，请按检查结果修改');
                return;
            }
            notify.success(outcome.message);
            setSpecForm((form) => ({...form, ...spec}));
            setImportReview(null);
            if (outcome.id) {
                setSpecId(outcome.id);
                const data = await coursewarePreview(outcome.id);
                setPreview(data);
            }
            await load();
            onChanged?.();
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="page">
            <PageHead
                title="课件"
                subtitle="生成或整理讲义，校对后入库，再导出授课文件"
                help="courseware"
                actions={
                    <>
                        <button className="btn" onClick={() => setImportOpen(true)}>
                            导入现有讲义
                        </button>
                        <button className="btn" onClick={importFile}>
                            导入 HTML
                        </button>
                        <button className="btn" onClick={newSpec}>
                            手动新建
                        </button>
                        <button className="btn primary" onClick={() => setAiOpen(true)}>
                            AI 生成课件
                        </button>
                    </>
                }
            />

            <Card className="cw-toolbar-card">
                <div className="teach-toolbar">
                    <input
                        className="teach-search"
                        placeholder="搜标题 / 知识点 / 简介"
                        value={keyword}
                        onChange={(event) => setKeyword(event.target.value)}
                    />
                    <select value={subjectId} onChange={(event) => setSubjectId(event.target.value)}>
                        <option value="">全部学科</option>
                        {subjects.map((item) => (
                            <option key={item.id} value={item.id}>
                                {item.name}
                            </option>
                        ))}
                    </select>
                    <Tabs items={SOURCE_TABS} value={source} onChange={setSource}/>
                </div>
            </Card>

            {loading ? (
                <div className="loading">加载中…</div>
            ) : rows.length === 0 ? (
                <Empty
                    title="还没有课件"
                    hint="用 AI 生成一份可校对的课件，或手动编写、导入已有 HTML。"
                    action={
                        <div className="empty-actions">
                            <button className="btn primary" onClick={() => setAiOpen(true)}>
                                AI 生成课件
                            </button>
                            <button className="btn" onClick={newSpec}>
                                手动新建
                            </button>
                            <button className="btn" onClick={() => setImportOpen(true)}>
                                导入现有讲义
                            </button>
                            <button className="btn" onClick={importFile}>
                                导入 HTML
                            </button>
                        </div>
                    }
                />
            ) : (
                <div className="cw-grid">
                    {rows.map((row) => (
                        <article key={row.id} className="cw-card">
                            <div className={cx('cw-cover', `theme-${row.source === 'IMPORT' ? 'science' : 'paper'}`)}>
                                <span className="cw-cover-sub">
                                    {row.subjectName || '未分学科'}
                                    {row.grade ? ` · ${row.grade}` : ''}
                                </span>
                                <strong>{row.title}</strong>
                                <span className="cw-cover-mark">
                                    {row.source === 'IMPORT' ? '导入' : `讲义 · v${row.version}`}
                                </span>
                            </div>
                            <div className="cw-body">
                                {row.description && <p className="cw-desc">{row.description}</p>}
                                {row.knowledge.length > 0 && (
                                    <div className="cw-tags">
                                        {row.knowledge.slice(0, 3).map((tag) => (
                                            <Pill key={tag} tone="muted">
                                                {tag}
                                            </Pill>
                                        ))}
                                        {row.knowledge.length > 3 && (
                                            <Pill tone="muted">+{row.knowledge.length - 3}</Pill>
                                        )}
                                    </div>
                                )}
                                <div className="cw-stats">
                                    {row.source === 'LOCAL_BUILD' && (
                                        <>
                                            <span>
                                                <strong>{row.stepCount}</strong> 段
                                            </span>
                                            <span>
                                                <strong>{row.quizCount}</strong> 题
                                            </span>
                                        </>
                                    )}
                                    <span>
                                        <strong>{row.classes.length}</strong> 班布置
                                    </span>
                                    <span className="cw-updated">{formatTime(row.updatedAt)}</span>
                                </div>
                                {row.classes.length > 0 && (
                                    <div className="cw-assigned">
                                        {row.classes.map((klass) => (
                                            <span key={klass.id} className="cw-assigned-chip">
                                                {klass.name}
                                                <button
                                                    className="icon-btn tiny"
                                                    title="取消布置"
                                                    onClick={() => removeAssign(row.id, klass.id)}
                                                >
                                                    ✕
                                                </button>
                                            </span>
                                        ))}
                                    </div>
                                )}
                                <div className="cw-actions">
                                    <button className="btn small" onClick={() => openPreview(row)}>
                                        预览
                                    </button>
                                    <button className="btn small" onClick={() => setExportFor(row)}>
                                        导出
                                    </button>
                                    {row.hasSpec && (
                                        <button className="btn small" onClick={() => editSpec(row)}>
                                            编辑
                                        </button>
                                    )}
                                    <button
                                        className="btn small"
                                        onClick={() =>
                                            setAssignFor({
                                                id: row.id,
                                                title: row.title,
                                                classIds: [],
                                                dueAt: '',
                                                note: '',
                                            })
                                        }
                                    >
                                        布置
                                    </button>
                                    <button className="btn small ghost" onClick={() => openMeta(row)}>
                                        信息
                                    </button>
                                    <button className="btn small danger-text" onClick={() => remove(row)}>
                                        删除
                                    </button>
                                </div>
                            </div>
                        </article>
                    ))}
                </div>
            )}

            {/* 预览：隔离 iframe，不进主窗 CSP */}
            {preview && (
                <div className="cw-preview-mask" onMouseDown={() => setPreview(null)}>
                    <div className="cw-preview" onMouseDown={(event) => event.stopPropagation()}>
                        <div className="cw-preview-head">
                            <div>
                                <h3>{preview.title}</h3>
                                {preview.warnings?.length > 0 && (
                                    <p className="cw-preview-warn">{preview.warnings.join('；')}</p>
                                )}
                            </div>
                            <button className="icon-btn" onClick={() => setPreview(null)} aria-label="关闭">
                                ✕
                            </button>
                        </div>
                        <iframe
                            className="cw-frame"
                            title={preview.title}
                            src={preview.url}
                            sandbox="allow-scripts allow-same-origin"
                            allow="fullscreen"
                        />
                    </div>
                </div>
            )}

            {/* 布置到班级 */}
            {assignFor && <AssignModal state={assignFor} setState={setAssignFor} onSave={saveAssign} busy={busy}/>}

            {/* 元信息 */}
            {metaFor && (
                <Modal
                    title="课件信息"
                    subtitle="改名字、学科与知识点，方便检索"
                    onClose={() => setMetaFor(null)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={() => setMetaFor(null)}>
                                取消
                            </button>
                            <button className="btn primary" onClick={saveMeta} disabled={busy}>
                                保存
                            </button>
                        </>
                    }
                >
                    <Field label="标题">
                        <input
                            value={metaFor.title}
                            onChange={(event) => setMetaFor({...metaFor, title: event.target.value})}
                        />
                    </Field>
                    <div className="form-row">
                        <Field label="学科">
                            <select
                                value={metaFor.subjectId}
                                onChange={(event) => setMetaFor({...metaFor, subjectId: event.target.value})}
                            >
                                <option value="">不指定</option>
                                {subjects.map((item) => (
                                    <option key={item.id} value={item.id}>
                                        {item.name}
                                    </option>
                                ))}
                            </select>
                        </Field>
                        <Field label="年级">
                            <input
                                value={metaFor.grade}
                                onChange={(event) => setMetaFor({...metaFor, grade: event.target.value})}
                            />
                        </Field>
                    </div>
                    <Field label="简介">
                        <textarea
                            rows={3}
                            value={metaFor.description}
                            onChange={(event) => setMetaFor({...metaFor, description: event.target.value})}
                        />
                    </Field>
                    <Field label="知识点" hint="顿号或逗号分隔">
                        <input
                            value={metaFor.knowledge}
                            onChange={(event) => setMetaFor({...metaFor, knowledge: event.target.value})}
                        />
                    </Field>
                </Modal>
            )}

            {/* AI 提示词工作台：接受草稿后仍要进入编辑器校对 */}
            {aiOpen && (
                <CoursewareAi
                    baseSpec={emptySpec()}
                    subjects={subjects}
                    onClose={() => setAiOpen(false)}
                    onAccept={acceptAiSpec}
                />
            )}

            {/* 已有讲义批量导入 */}
            {importOpen && (
                <CoursewareImport
                    subjects={subjects}
                    onClose={() => setImportOpen(false)}
                    onReview={reviewImport}
                />
            )}

            {/* 导出 */}
            {exportFor && (
                <Modal
                    title={`导出「${exportFor.title}」`}
                    subtitle={exportFor.hasSpec ? '选择要保存的文件格式' : '旧 HTML 课件可导出网页或 PDF'}
                    onClose={() => setExportFor(null)}
                >
                    <div className="cw-export-options">
                        {EXPORT_FORMATS.filter((format) => exportFor.hasSpec || !format.spec).map((format) => (
                            <button
                                key={format.kind}
                                className="btn cw-export-option"
                                disabled={Boolean(exportBusy)}
                                onClick={() => exportFile(exportFor, format)}
                            >
                                <strong>{exportBusy === `${exportFor.id}:${format.kind}` ? '正在导出…' : format.label}</strong>
                                <small>{format.hint}</small>
                            </button>
                        ))}
                    </div>
                </Modal>
            )}

            {/* 讲义编辑器 */}
            {specForm && (
                <SpecEditor
                    form={specForm}
                    setForm={setSpecForm}
                    subjects={subjects}
                    gates={gates}
                    busy={busy}
                    onClose={closeSpec}
                    onCompile={compile}
                    onSaveDraft={importReview ? saveImportDraft : null}
                    onAiDraft={aiDraft}
                    patchItem={patchItem}
                    addRow={addRow}
                    dropRow={dropRow}
                    pickQuestion={pickQuestion}
                />
            )}

            {/* 题库挑题 */}
            {picker && (
                <Modal
                    title="从题库挑题"
                    subtitle="挑中后把题干/答案/解析带进讲义条目，编译时内联"
                    onClose={() => setPicker(null)}
                    wide
                >
                    <div className="teach-toolbar">
                        <input
                            className="teach-search"
                            placeholder="搜题干"
                            value={picker.keyword}
                            onChange={(event) => setPicker({...picker, keyword: event.target.value})}
                            onKeyDown={(event) => event.key === 'Enter' && searchPicker()}
                        />
                        <button className="btn" onClick={searchPicker}>
                            搜索
                        </button>
                    </div>
                    <div className="cw-pick-list">
                        {picker.loading ? (
                            <div className="loading">加载中…</div>
                        ) : picker.rows.length === 0 ? (
                            <div className="loading">没搜到题目</div>
                        ) : (
                            picker.rows.map((question) => (
                                <button
                                    key={question.id}
                                    className="cw-pick-row"
                                    onClick={() => chooseQuestion(question)}
                                >
                                    <span className="cw-pick-stem"><RichInline text={question.stem}/></span>
                                    <span className="cw-pick-meta">{question.typeLabel || question.type}</span>
                                </button>
                            ))
                        )}
                    </div>
                </Modal>
            )}
        </div>
    );
}

// ---------------------------------------------------------------------------

function AssignModal({state, setState, onSave, busy}) {
    const [classes, setClasses] = useState([]);
    useEffect(() => {
        classList()
            .then((rows) => setClasses(rows.filter((row) => row.status !== 'CLOSED')))
            .catch((error) => notify.error(error.message));
    }, []);
    return (
        <Modal
            title={`布置「${state.title}」`}
            subtitle="选班级，可设截止时间；学生在中心课件库看到"
            onClose={() => setState(null)}
            footer={
                <>
                    <button className="btn ghost" onClick={() => setState(null)}>
                        取消
                    </button>
                    <button className="btn primary" onClick={onSave} disabled={busy}>
                        布置
                    </button>
                </>
            }
        >
            <Field label="班级">
                <div className="cw-check-list">
                    {classes.length === 0 && <p className="field-hint">还没有班级，先去「班级」建一个。</p>}
                    {classes.map((klass) => {
                        const on = state.classIds.includes(klass.id);
                        return (
                            <label key={klass.id} className={cx('cw-check', on && 'on')}>
                                <input
                                    type="checkbox"
                                    checked={on}
                                    onChange={() =>
                                        setState({
                                            ...state,
                                            classIds: on
                                                ? state.classIds.filter((id) => id !== klass.id)
                                                : [...state.classIds, klass.id],
                                        })
                                    }
                                />
                                <span>{klass.name}</span>
                                <small>{klass.memberCount} 人</small>
                            </label>
                        );
                    })}
                </div>
            </Field>
            <div className="form-row">
                <Field label="截止时间" hint="留空表示不限时">
                    <input
                        type="datetime-local"
                        value={state.dueAt}
                        onChange={(event) => setState({...state, dueAt: event.target.value})}
                    />
                </Field>
            </div>
            <Field label="备注">
                <input
                    value={state.note}
                    placeholder="例如：课前预习第 1–3 段"
                    onChange={(event) => setState({...state, note: event.target.value})}
                />
            </Field>
        </Modal>
    );
}

function SpecEditor({
    form,
    setForm,
    subjects,
    gates,
    busy,
    onClose,
    onCompile,
    onSaveDraft,
    onAiDraft,
    patchItem,
    addRow,
    dropRow,
    pickQuestion,
}) {
    return (
        <div className="modal-mask" onMouseDown={onClose}>
            <div className="modal cw-editor" onMouseDown={(event) => event.stopPropagation()}>
                <div className="modal-head">
                    <div>
                        <h3>讲义生成器</h3>
                        <p>知识清单 → 例题 → 变式 → 拔高 → 思维训练 → 小结作业，一次编译成单文件讲义</p>
                    </div>
                    <button className="icon-btn" onClick={onClose} aria-label="关闭">
                        ✕
                    </button>
                </div>
                <div className="modal-body cw-editor-body">
                    <section className="cw-sec">
                        <h4>基本信息</h4>
                        <Field label="课题">
                            <input
                                value={form.title}
                                placeholder="例如：热力环流与海陆风"
                                onChange={(event) => setForm({...form, title: event.target.value})}
                            />
                        </Field>
                        <div className="form-row">
                            <Field label="学科">
                                <select
                                    value={form.subjectId}
                                    onChange={(event) => {
                                        const item = subjects.find((s) => s.id === event.target.value);
                                        setForm({
                                            ...form,
                                            subjectId: event.target.value,
                                            subject: item?.name || '',
                                        });
                                    }}
                                >
                                    <option value="">不指定</option>
                                    {subjects.map((item) => (
                                        <option key={item.id} value={item.id}>
                                            {item.name}
                                        </option>
                                    ))}
                                </select>
                            </Field>
                            <Field label="年级">
                                <input
                                    value={form.grade}
                                    onChange={(event) => setForm({...form, grade: event.target.value})}
                                />
                            </Field>
                            <Field label="章节">
                                <input
                                    value={form.chapter}
                                    onChange={(event) => setForm({...form, chapter: event.target.value})}
                                />
                            </Field>
                            <Field label="版式">
                                <select
                                    value={form.theme}
                                    onChange={(event) => setForm({...form, theme: event.target.value})}
                                >
                                    {THEMES.map((item) => (
                                        <option key={item.value} value={item.value}>
                                            {item.label}
                                        </option>
                                    ))}
                                </select>
                            </Field>
                        </div>
                        <div className="form-row">
                            <Field label="知识点" hint="顿号或逗号分隔">
                                <input
                                    value={form.knowledge.join('、')}
                                    onChange={(event) => setForm({...form, knowledge: splitList(event.target.value)})}
                                />
                            </Field>
                        </div>
                        <Field label="简介">
                            <textarea
                                rows={2}
                                value={form.description}
                                onChange={(event) => setForm({...form, description: event.target.value})}
                            />
                        </Field>
                        <Field label="教学目标" hint="一行一条">
                            <textarea
                                rows={3}
                                value={linesToText(form.objectives)}
                                onChange={(event) => setForm({...form, objectives: textToLines(event.target.value)})}
                            />
                        </Field>
                        <Field label="情境导入">
                            <textarea
                                rows={2}
                                value={form.warmup}
                                onChange={(event) => setForm({...form, warmup: event.target.value})}
                            />
                        </Field>
                    </section>

                    <section className="cw-sec">
                        <h4>教学要求</h4>
                        {(form.aiAdded.length > 0 || form.source.warnings.length > 0) && (
                            <p className="cw-source-note">
                                {form.aiAdded.length > 0 && <>AI 补写内容请重点核对：{form.aiAdded.join('；')}。</>}
                                {form.source.warnings.length > 0 && <> {form.source.warnings.join('；')}</>}
                            </p>
                        )}
                        {form.source.originalPath && (
                            <p className="cw-link-note cw-source-path">原稿：{form.source.originalPath}</p>
                        )}
                        <div className="form-row">
                            <Field label="内容结构">
                                <select value={form.contentMode} onChange={(event) => setForm({...form, contentMode: event.target.value})}>
                                    <option value="REGULAR">常规课程</option>
                                    <option value="TOPIC">专题课程</option>
                                </select>
                            </Field>
                            <Field label="课时"><input value={form.duration} onChange={(event) => setForm({...form, duration: event.target.value})}/></Field>
                            <Field label="难度"><input value={form.difficulty} onChange={(event) => setForm({...form, difficulty: event.target.value})}/></Field>
                            <Field label="教学风格"><input value={form.teachingStyle} onChange={(event) => setForm({...form, teachingStyle: event.target.value})}/></Field>
                        </div>
                        <Field label="学生情况"><input value={form.audience} onChange={(event) => setForm({...form, audience: event.target.value})}/></Field>
                        <div className="form-row">
                            <Field label="教学重点"><textarea rows={2} value={form.teachingFocus} onChange={(event) => setForm({...form, teachingFocus: event.target.value})}/></Field>
                            <Field label="教学难点"><textarea rows={2} value={form.teachingDifficulty} onChange={(event) => setForm({...form, teachingDifficulty: event.target.value})}/></Field>
                        </div>
                    </section>

                    {form.images.length > 0 && (
                        <section className="cw-sec">
                            <h4>课件图片 <small>确认图号、说明与使用位置</small></h4>
                            {form.images.map((image, index) => (
                                <div className="cw-item" key={`${image.id}-${index}`}>
                                    <div className="cw-item-head">
                                        <span className="cw-no">{image.id}</span>
                                        <input value={image.name} placeholder="图片名称" onChange={(event) => patchItem('images', index, {name: event.target.value})}/>
                                        <button className="icon-btn tiny" onClick={() => dropRow('images', index)}>✕</button>
                                    </div>
                                    <input value={image.caption} placeholder="图片说明" onChange={(event) => patchItem('images', index, {caption: event.target.value})}/>
                                    <div className="form-row">
                                        <input value={image.placement} placeholder="使用位置" onChange={(event) => patchItem('images', index, {placement: event.target.value})}/>
                                        <label className="cw-required"><input type="checkbox" checked={image.required} onChange={(event) => patchItem('images', index, {required: event.target.checked})}/>必须使用</label>
                                    </div>
                                </div>
                            ))}
                        </section>
                    )}

                    <section className="cw-sec">
                        <h4>
                            知识清单
                            <button className="btn small ghost" onClick={() => addRow('points', emptyPoint)}>
                                + 加一条
                            </button>
                        </h4>
                        {form.points.map((point, index) => (
                            <div key={index} className="cw-item">
                                <div className="cw-item-head">
                                    <span className="cw-no">{index + 1}</span>
                                    <input
                                        placeholder="概念名，例如：热力环流"
                                        value={point.name}
                                        onChange={(event) => patchItem('points', index, {name: event.target.value})}
                                    />
                                    <button className="icon-btn tiny" onClick={() => dropRow('points', index)}>
                                        ✕
                                    </button>
                                </div>
                                <input
                                    placeholder="一句话要点"
                                    value={point.summary}
                                    onChange={(event) => patchItem('points', index, {summary: event.target.value})}
                                />
                                <textarea
                                    rows={2}
                                    placeholder="展开讲解"
                                    value={point.detail}
                                    onChange={(event) => patchItem('points', index, {detail: event.target.value})}
                                />
                                <div className="form-row">
                                    <input
                                        placeholder="公式 / 口诀（可空）"
                                        value={point.formula}
                                        onChange={(event) => patchItem('points', index, {formula: event.target.value})}
                                    />
                                    <input
                                        placeholder="易错提醒（可空）"
                                        value={point.misconception}
                                        onChange={(event) =>
                                            patchItem('points', index, {misconception: event.target.value})
                                        }
                                    />
                                </div>
                            </div>
                        ))}
                    </section>

                    {[
                        {key: 'examples', label: '例题精讲', hint: '优先挂题库题，编译时内联题干与解析'},
                        {key: 'variants', label: '变式拓展', hint: '换条件 / 换问法，一题多变'},
                        {key: 'challenge', label: '拔高挑战', hint: '给学有余力的学生'},
                    ].map((section) => (
                        <section key={section.key} className="cw-sec">
                            <h4>
                                {section.label}
                                <small>{section.hint}</small>
                                <button
                                    className="btn small ghost"
                                    onClick={() => addRow(section.key, emptyItem)}
                                >
                                    + 加一题
                                </button>
                            </h4>
                            {form[section.key].length === 0 && (
                                <p className="field-hint">暂无。可手动加题，或用「AI 补题」从例题生成草稿。</p>
                            )}
                            {form[section.key].map((item, index) => (
                                <div key={index} className="cw-item">
                                    <div className="cw-item-head">
                                        <span className="cw-no">{index + 1}</span>
                                        <input
                                            placeholder="题干"
                                            value={item.stem}
                                            onChange={(event) =>
                                                patchItem(section.key, index, {stem: event.target.value})
                                            }
                                        />
                                        <button
                                            className="btn small ghost"
                                            onClick={() => pickQuestion(section.key, index)}
                                        >
                                            挑题
                                        </button>
                                        <button className="icon-btn tiny" onClick={() => dropRow(section.key, index)}>
                                            ✕
                                        </button>
                                    </div>
                                    {item.questionId && (
                                        <p className="cw-link-note">
                                            已挂题库题 <code>{item.questionId.slice(0, 8)}</code>，编译时把库里的题干/答案/解析内联进来
                                        </p>
                                    )}
                                    <div className="form-row">
                                        <input
                                            placeholder="答案"
                                            value={item.answer}
                                            onChange={(event) =>
                                                patchItem(section.key, index, {answer: event.target.value})
                                            }
                                        />
                                    </div>
                                    <textarea
                                        rows={2}
                                        placeholder="解析"
                                        value={item.analysis}
                                        onChange={(event) =>
                                            patchItem(section.key, index, {analysis: event.target.value})
                                        }
                                    />
                                </div>
                            ))}
                        </section>
                    ))}

                    <section className="cw-sec">
                        <h4>
                            思维训练
                            <small>开放设问，只给提示不给答案</small>
                            <button className="btn small ghost" onClick={() => addRow('thinking', emptyThink)}>
                                + 加一题
                            </button>
                        </h4>
                        {form.thinking.map((item, index) => (
                            <div key={index} className="cw-item">
                                <div className="cw-item-head">
                                    <span className="cw-no">{index + 1}</span>
                                    <input
                                        placeholder="开放设问"
                                        value={item.prompt}
                                        onChange={(event) => patchItem('thinking', index, {prompt: event.target.value})}
                                    />
                                    <button className="icon-btn tiny" onClick={() => dropRow('thinking', index)}>
                                        ✕
                                    </button>
                                </div>
                                <div className="form-row">
                                    <input
                                        placeholder="提示"
                                        value={item.hint}
                                        onChange={(event) => patchItem('thinking', index, {hint: event.target.value})}
                                    />
                                    <input
                                        placeholder="参考思路（可空）"
                                        value={item.answer}
                                        onChange={(event) => patchItem('thinking', index, {answer: event.target.value})}
                                    />
                                </div>
                            </div>
                        ))}
                    </section>

                    <section className="cw-sec">
                        <h4>小结与作业</h4>
                        <Field label="小结">
                            <textarea
                                rows={2}
                                value={form.summary}
                                onChange={(event) => setForm({...form, summary: event.target.value})}
                            />
                        </Field>
                        <Field label="作业" hint="一行一条">
                            <textarea
                                rows={2}
                                value={linesToText(form.homework)}
                                onChange={(event) => setForm({...form, homework: textToLines(event.target.value)})}
                            />
                        </Field>
                    </section>

                    {gates && gates.length > 0 && (
                        <section className="cw-sec">
                            <h4>五道闸</h4>
                            <ul className="cw-gates">
                                {gates.map((gate) => (
                                    <li key={gate.name} className={gate.ok ? 'ok' : 'bad'}>
                                        <span>{gate.ok ? '✓' : '✗'}</span>
                                        <strong>{gate.name}</strong>
                                        <em>{gate.detail}</em>
                                    </li>
                                ))}
                            </ul>
                        </section>
                    )}
                </div>
                <div className="modal-actions">
                    {onSaveDraft && <button className="btn" onClick={onSaveDraft} disabled={busy}>保存校对草稿</button>}
                    <button className="btn ghost" onClick={onAiDraft} disabled={busy}>
                        AI 补题
                    </button>
                    <button className="btn" onClick={onClose}>
                        关闭
                    </button>
                    <button className="btn primary" onClick={onCompile} disabled={busy}>
                        {busy ? '正在生成…' : '校对完成，生成课件'}
                    </button>
                </div>
            </div>
        </div>
    );
}
