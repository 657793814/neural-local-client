import {useCallback, useEffect, useMemo, useState} from 'react';
import {open} from '@tauri-apps/plugin-dialog';
import {
    coursewareImportList,
    coursewareImportOriginal,
    coursewareImportRetry,
    coursewareImportStart,
    openPath,
} from '../lib/api.js';
import {cx, formatTime} from '../lib/format.js';
import {Empty, Field, Modal, Pill, confirmDialog, notify} from '../components/ui.jsx';

const STATUS = {
    QUEUED: {label: '等待处理', tone: 'muted'},
    EXTRACTING: {label: '正在整理', tone: 'warn'},
    REVIEW: {label: '待校对', tone: 'warn'},
    FAILED: {label: '失败', tone: 'danger'},
    DUPLICATE: {label: '重复', tone: 'muted'},
    COMMITTED: {label: '已入库', tone: 'ok'},
};

function draftText(draft) {
    if (!draft) return '尚未生成草稿';
    const blocks = [
        `课题：${draft.title || '未填写'}`,
        draft.description && `简介：${draft.description}`,
        draft.objectives?.length && `教学目标：\n${draft.objectives.map((item) => `- ${item}`).join('\n')}`,
        draft.warmup && `情境导入：\n${draft.warmup}`,
        draft.points?.length && `知识讲解：\n${draft.points.map((point, index) => `${index + 1}. ${point.name}\n${point.summary || ''}\n${point.detail || ''}`).join('\n\n')}`,
        ...[
            ['例题', draft.examples],
            ['变式', draft.variants],
            ['拔高', draft.challenge],
        ].filter(([, items]) => items?.length).map(([label, items]) => `${label}：\n${items.map((item, index) => `${index + 1}. ${item.stem}\n答案：${item.answer || '未提供'}\n解析：${item.analysis || '未提供'}`).join('\n\n')}`),
        draft.thinking?.length && `思维训练：\n${draft.thinking.map((item) => `- ${item.prompt}`).join('\n')}`,
        draft.summary && `小结：\n${draft.summary}`,
        draft.homework?.length && `作业：\n${draft.homework.map((item) => `- ${item}`).join('\n')}`,
    ];
    return blocks.filter(Boolean).join('\n\n');
}

export default function CoursewareImport({subjects, onClose, onReview}) {
    const [rows, setRows] = useState([]);
    const [selectedId, setSelectedId] = useState(null);
    const [loading, setLoading] = useState(true);
    const [busy, setBusy] = useState(false);
    const [retryId, setRetryId] = useState('');
    const [config, setConfig] = useState({
        mode: 'FORMAT_ONLY',
        useAi: true,
        subjectId: '',
        grade: '',
    });

    const load = useCallback(async (silent = false) => {
        try {
            const items = await coursewareImportList();
            setRows(items);
            setSelectedId((current) => items.some((item) => item.id === current) ? current : items[0]?.id || null);
        } catch (error) {
            if (!silent) notify.error(error.message);
        } finally {
            if (!silent) setLoading(false);
        }
    }, []);

    useEffect(() => { load(); }, [load]);

    const active = rows.some((item) => ['QUEUED', 'EXTRACTING'].includes(item.status));
    useEffect(() => {
        if (!active) return undefined;
        const timer = setInterval(() => load(true), 1200);
        return () => clearInterval(timer);
    }, [active, load]);

    const selected = useMemo(() => rows.find((item) => item.id === selectedId) || null, [rows, selectedId]);

    const start = async () => {
        if (config.mode === 'OPTIMIZE' && !config.useAi) {
            notify.warning('AI 优化需要启用 AI；也可以改回仅统一格式');
            return;
        }
        const picked = await open({
            multiple: true,
            filters: [{
                name: '讲义文件',
                extensions: ['docx', 'docm', 'pptx', 'pdf', 'md', 'markdown', 'txt', 'html', 'htm'],
            }],
        });
        if (!picked) return;
        const paths = Array.isArray(picked) ? picked : [picked];
        setBusy(true);
        try {
            const subject = subjects.find((item) => item.id === config.subjectId)?.name || '';
            const items = await coursewareImportStart({
                paths,
                mode: config.mode,
                useAi: config.useAi,
                subjectId: config.subjectId || null,
                subject,
                grade: config.grade,
            });
            setSelectedId(items[0]?.id || null);
            await load(true);
            notify.success(`已开始整理 ${items.length} 份讲义，每份可单独校对`);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const retry = async (item) => {
        const allowDuplicate = item.status === 'DUPLICATE';
        if (allowDuplicate) {
            const yes = await confirmDialog({
                title: '仍要导入重复讲义',
                message: `「${item.title}」与已有导入记录内容相同，确认再建一份待校对草稿？`,
                confirmText: '仍要导入',
            });
            if (!yes) return;
        }
        setRetryId(item.id);
        try {
            await coursewareImportRetry(item.id, item.useAi, allowDuplicate);
            await load(true);
            notify.info(`已重新整理「${item.title}」`);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setRetryId('');
        }
    };

    const openOriginal = async (item) => {
        try {
            await openPath(await coursewareImportOriginal(item.id));
        } catch (error) {
            notify.error(error.message);
        }
    };

    return (
        <Modal
            title="导入现有讲义"
            subtitle="批量统一格式，逐份校对后入库；默认不改写教学内容"
            onClose={onClose}
            wide
            footer={<button className="btn" onClick={onClose}>关闭</button>}
        >
            <div className="cw-import-config">
                <Field label="整理方式">
                    <select value={config.mode} onChange={(event) => setConfig({...config, mode: event.target.value})}>
                        <option value="FORMAT_ONLY">仅统一格式，不补答案</option>
                        <option value="OPTIMIZE">允许 AI 优化并标出新增内容</option>
                    </select>
                </Field>
                <Field label="学科">
                    <select value={config.subjectId} onChange={(event) => setConfig({...config, subjectId: event.target.value})}>
                        <option value="">不指定</option>
                        {subjects.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
                    </select>
                </Field>
                <Field label="年级"><input value={config.grade} onChange={(event) => setConfig({...config, grade: event.target.value})}/></Field>
                <label className="cw-required"><input type="checkbox" checked={config.useAi} onChange={(event) => setConfig({...config, useAi: event.target.checked})}/>用 AI 识别内容结构</label>
                <button className="btn primary" onClick={start} disabled={busy}>{busy ? '正在创建任务…' : '选择文件并导入'}</button>
            </div>
            <p className="field-hint">支持 DOCX、PPTX、PDF、Markdown、TXT、HTML；AI 不可用时仍保留本地解析草稿。</p>

            {loading ? <div className="loading">加载导入记录…</div> : rows.length === 0 ? (
                <Empty title="还没有导入记录" hint="选择一批讲义文件，每份都会单独保存原稿和处理状态。"/>
            ) : (
                <div className="cw-import-layout">
                    <div className="cw-import-list">
                        {rows.map((item) => {
                            const status = STATUS[item.status] || STATUS.QUEUED;
                            return (
                                <div key={item.id} className={cx('cw-import-row', selectedId === item.id && 'active')}>
                                    <button className="cw-import-select" onClick={() => setSelectedId(item.id)}>
                                        <strong>{item.title}</strong>
                                        <span><Pill tone={status.tone}>{status.label}</Pill>{item.mode === 'OPTIMIZE' ? 'AI 优化' : '仅统一格式'} · {formatTime(item.updatedAt)}</span>
                                    </button>
                                    <div className="cw-import-row-actions">
                                        {item.status === 'REVIEW' && <button className="btn small" onClick={() => onReview(item)}>校对</button>}
                                        {['FAILED', 'DUPLICATE'].includes(item.status) && (
                                            <button className="btn small" disabled={retryId === item.id} onClick={() => retry(item)}>
                                                {retryId === item.id ? '已排队' : item.status === 'DUPLICATE' ? '仍要导入' : '重试'}
                                            </button>
                                        )}
                                    </div>
                                </div>
                            );
                        })}
                    </div>

                    {selected && (
                        <div className="cw-import-detail">
                            <div className="cw-import-detail-head">
                                <div><h4>{selected.title}</h4><p>{selected.sourcePath}</p></div>
                                {selected.managedPath && <button className="btn small" onClick={() => openOriginal(selected)}>打开保留的原稿</button>}
                            </div>
                            {selected.error && <p className="cw-source-note">{selected.error}</p>}
                            {selected.warnings.length > 0 && <p className="cw-source-note">{selected.warnings.join('；')}</p>}
                            {selected.status === 'REVIEW' && selected.error.startsWith('AI') && (
                                <button className="btn small" disabled={retryId === selected.id} onClick={() => retry(selected)}>重试 AI 整理</button>
                            )}
                            <div className="cw-import-compare">
                                <section><h4>原稿抽取文字</h4><pre>{selected.extractedText || '尚未抽取到文字'}</pre></section>
                                <section><h4>待校对内容</h4><pre>{draftText(selected.draft)}</pre></section>
                            </div>
                            {selected.rawAiJson && <details className="cw-external"><summary>查看 AI 原始返回</summary><textarea rows={8} readOnly value={selected.rawAiJson}/></details>}
                            {selected.status === 'REVIEW' && <button className="btn primary" onClick={() => onReview(selected)}>校对这份课件</button>}
                        </div>
                    )}
                </div>
            )}
        </Modal>
    );
}
