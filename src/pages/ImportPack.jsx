import {useEffect, useState} from 'react';
import {open} from '@tauri-apps/plugin-dialog';
import {
    importBatches,
    importCommit,
    importDiscard,
    importErrors,
    importScan,
    importScanText,
    importUndo,
    packPrompt,
} from '../lib/api.js';
import {Pill, Card, Empty, PageHead, Tabs, confirmDialog, notify} from '../components/ui.jsx';
import {Rich} from '../lib/Math.jsx';
import {cx, formatTime, typeLabel} from '../lib/format.js';

const STATUS = {
    READY: {label: '可导入', tone: 'ok'},
    DUPLICATE: {label: '重复', tone: 'warn'},
    SIMILAR: {label: '相似', tone: 'warn'},
    INVALID: {label: '无效', tone: 'danger'},
};

async function copyText(text) {
    try {
        await navigator.clipboard.writeText(text);
        return true;
    } catch {
        const area = document.createElement('textarea');
        area.value = text;
        document.body.appendChild(area);
        area.select();
        document.execCommand('copy');
        document.body.removeChild(area);
        return true;
    }
}

export default function ImportPack({onChanged}) {
    const [tab, setTab] = useState('file');
    const [preview, setPreview] = useState(null);
    const [selected, setSelected] = useState(() => new Set());
    const [filter, setFilter] = useState('ALL');
    const [busy, setBusy] = useState(false);
    const [result, setResult] = useState(null);
    const [pasteText, setPasteText] = useState('');
    const [batches, setBatches] = useState([]);
    const [batchErrors, setBatchErrors] = useState({});
    const [prompt, setPrompt] = useState('');

    const loadBatches = () => importBatches().then(setBatches).catch(() => {});

    useEffect(() => {
        loadBatches();
        packPrompt().then(setPrompt).catch(() => {});
    }, []);

    const discardPreview = async (previewId) => {
        if (!previewId) return;
        try {
            await importDiscard(previewId);
        } catch (error) {
            notify.error(error.message);
        }
    };

    const applyPreview = async (data) => {
        if (preview?.id && preview.id !== data.id) await discardPreview(preview.id);
        setPreview(data);
        setResult(null);
        setFilter('ALL');
        const picked = new Set(
            data.items
                .filter((item) => item.status === 'READY')
                .map((item) => item.rowNo),
        );
        setSelected(picked);
    };

    const cancelPreview = async () => {
        const previewId = preview?.id;
        setPreview(null);
        setSelected(new Set());
        await discardPreview(previewId);
    };

    const pickFile = async () => {
        const path = await open({
            multiple: false,
            filters: [{name: '题目包', extensions: ['json', 'jsonl', 'ndjson', 'zip']}],
        });
        if (!path) return;
        setBusy(true);
        try {
            await applyPreview(await importScan(path));
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const parsePaste = async () => {
        setBusy(true);
        try {
            await applyPreview(await importScanText(pasteText));
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const commit = async () => {
        if (!preview) return;
        const rows = [...selected];
        if (rows.length === 0) {
            notify.warning('还没有选任何题');
            return;
        }
        const risky = items.filter(
            (item) => selected.has(item.rowNo) && (item.status === 'DUPLICATE' || item.status === 'SIMILAR'),
        );
        if (risky.length > 0) {
            const ok = await confirmDialog({
                title: '仍然导入重复或相似题？',
                message: `已选 ${risky.length} 道重复或相似题。请确认题目包内容无误，再按所选题目导入。`,
                confirmText: '仍然导入',
            });
            if (!ok) return;
        }
        setBusy(true);
        try {
            const outcome = await importCommit(preview.id, rows, risky.length > 0);
            setResult(outcome);
            setPreview(null);
            setSelected(new Set());
            notify.success(
                `导入 ${outcome.imported} 道${outcome.skipped ? `，跳过 ${outcome.skipped} 道` : ''}${
                    outcome.failed ? `，失败 ${outcome.failed} 道` : ''
                }`,
            );
            await loadBatches();
            onChanged?.();
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const undo = async (batch) => {
        const ok = await confirmDialog({
            title: '撤销这一批？',
            message: `「${batch.filename}」导入的 ${batch.remaining} 道题会移到回收站，可以再恢复。`,
            confirmText: '撤销',
        });
        if (!ok) return;
        try {
            const count = await importUndo(batch.id);
            notify.success(`已撤回 ${count} 道题`);
            await loadBatches();
            onChanged?.();
        } catch (error) {
            notify.error(error.message);
        }
    };

    const showErrors = async (batch) => {
        if (batchErrors[batch.id]) {
            setBatchErrors((current) => {
                const next = {...current};
                delete next[batch.id];
                return next;
            });
            return;
        }
        try {
            const list = await importErrors(batch.id);
            setBatchErrors((current) => ({...current, [batch.id]: list}));
        } catch (error) {
            notify.error(error.message);
        }
    };

    const items = preview?.items || [];
    const visible = filter === 'ALL' ? items : items.filter((item) => item.status === filter);
    const counts = {
        ALL: items.length,
        READY: preview?.ready || 0,
        DUPLICATE: preview?.duplicate || 0,
        SIMILAR: preview?.similar || 0,
        INVALID: preview?.invalid || 0,
    };

    return (
        <div className="page">
            <PageHead
                title="导入"
                subtitle="题目包只有 JSON / JSONL / ZIP 三种；Excel 和 Word 先用 AI 转一次"
                help="import"
            />

            <Tabs
                value={tab}
                onChange={setTab}
                items={[
                    {value: 'file', label: '题目包文件'},
                    {value: 'paste', label: '粘贴 JSON'},
                    {value: 'ai', label: '用 AI 转换'},
                ]}
            />

            {tab === 'file' && (
                <Card>
                    <div className="dropzone" onClick={pickFile}>
                        <div className="dropzone-mark">⇪</div>
                        <strong>{busy ? '正在读取…' : '选择 questions.json / .jsonl / .zip'}</strong>
                        <p className="muted">
                            ZIP 里放 <code>questions.json</code> 和 <code>images/</code>；单题失败不影响整包
                        </p>
                    </div>
                </Card>
            )}

            {tab === 'paste' && (
                <Card title="粘贴题目包" subtitle="智能体直接给了 JSON？粘进来就行">
                    <textarea
                        className="paste-area"
                        rows={12}
                        value={pasteText}
                        placeholder={'{"format":"neural-question-pack","version":1,"questions":[…]}'}
                        onChange={(event) => setPasteText(event.target.value)}
                    />
                    <div className="row-end">
                        <button className="btn primary" onClick={parsePaste} disabled={busy || !pasteText.trim()}>
                            解析并预览
                        </button>
                    </div>
                </Card>
            )}

            {tab === 'ai' && (
                <Card
                    title="让 AI 把表格 / 试卷转成题目包"
                    subtitle="复制这段提示词，连同你的文件发给任意支持文件与视觉的智能体"
                    actions={
                        <button
                            className="btn primary small"
                            onClick={async () => {
                                await copyText(prompt);
                                notify.success('提示词已复制，粘给智能体即可');
                            }}
                        >
                            复制提示词
                        </button>
                    }
                >
                    <ol className="steps">
                        <li>复制提示词，连同 Excel / Word / 试卷图片一起发给智能体。</li>
                        <li>让它输出一个 JSON 文件；能打包 ZIP 更好（图片会一起带过来）。</li>
                        <li>回到「题目包文件」选那个文件，预览没问题再导入。</li>
                    </ol>
                    <pre className="prompt-block">{prompt || '加载中…'}</pre>
                    <p className="muted small">
                        智能体拿不到原图时会在 missingImages 里注明，导入后提示你补图——不会用文字描述糊过去。
                    </p>
                </Card>
            )}

            {result && (
                <Card title="导入完成" className="result-card">
                    <div className="result-grid">
                        <div>
                            <strong>{result.imported}</strong>
                            <span>已入库</span>
                        </div>
                        <div>
                            <strong>{result.skipped}</strong>
                            <span>跳过</span>
                        </div>
                        <div>
                            <strong>{result.failed}</strong>
                            <span>失败</span>
                        </div>
                    </div>
                    <p className="muted small">题目默认「草稿」，在题库里核对后再用；这一批可以整批撤销。</p>
                </Card>
            )}

            {preview && (
                <Card
                    title="预览"
                    subtitle={`${preview.filename} · 共 ${preview.total} 道`}
                    actions={
                        <div className="inline-actions">
                            <button className="btn ghost small" onClick={cancelPreview} disabled={busy}>
                                取消
                            </button>
                            <button className="btn primary small" onClick={commit} disabled={busy || selected.size === 0}>
                                导入 {selected.size} 道
                            </button>
                        </div>
                    }
                >
                    <div className="chip-row">
                        {Object.entries(counts).map(([key, count]) => (
                            <button
                                key={key}
                                className={cx('chip', filter === key && 'active')}
                                onClick={() => setFilter(key)}
                            >
                                {key === 'ALL' ? '全部' : STATUS[key].label} <span className="chip-count">{count}</span>
                            </button>
                        ))}
                        <span className="spacer"/>
                        <button
                            className="btn ghost small"
                            onClick={() =>
                                setSelected(
                                    new Set(
                                        items
                                            .filter((item) => item.status === 'READY' || item.status === 'SIMILAR')
                                            .map((item) => item.rowNo),
                                    ),
                                )
                            }
                        >
                            选中所有非重复
                        </button>
                    </div>

                    {visible.length === 0 ? (
                        <Empty title="这一类没有题目" hint="换一个筛选看看"/>
                    ) : (
                        <div className="preview-list">
                            {visible.map((item) => {
                                const status = STATUS[item.status] || STATUS.READY;
                                return (
                                    <article key={item.rowNo} className={cx('preview-item', item.status.toLowerCase())}>
                                        <label className="preview-check">
                                            <input
                                                type="checkbox"
                                                checked={selected.has(item.rowNo)}
                                                disabled={item.status === 'INVALID'}
                                                onChange={(event) => {
                                                    setSelected((current) => {
                                                        const next = new Set(current);
                                                        if (event.target.checked) next.add(item.rowNo);
                                                        else next.delete(item.rowNo);
                                                        return next;
                                                    });
                                                }}
                                            />
                                        </label>
                                        <div className="preview-body">
                                            <div className="preview-head">
                                                <span className="row-no">#{item.rowNo}</span>
                                                <Pill tone={status.tone}>{status.label}</Pill>
                                                <Pill tone="line">{typeLabel(item.draft.type)}</Pill>
                                                <Pill tone="line">难度 {item.draft.difficulty}</Pill>
                                                {item.draft.knowledgePath.length > 0 && (
                                                    <span className="muted small">{item.draft.knowledgePath.join(' / ')}</span>
                                                )}
                                            </div>
                                            <div className="preview-stem">
                                                <Rich text={item.draft.stem}/>
                                            </div>
                                            {item.draft.options.length > 0 && (
                                                <ul className="option-view compact">
                                                    {item.draft.options.map((option) => (
                                                        <li key={option.key}>
                                                            <span className="option-key">{option.key}</span>
                                                            <Rich text={option.text} className="inline"/>
                                                        </li>
                                                    ))}
                                                </ul>
                                            )}
                                            <div className="preview-answer">
                                                <span>答案</span>
                                                <Rich text={item.draft.answer} className="inline"/>
                                            </div>
                                            {item.images.length > 0 && (
                                                <div className="preview-images">
                                                    {item.images.map((image) => (
                                                        <figure key={image.file}>
                                                            {image.dataUrl ? (
                                                                <img src={image.dataUrl} alt={image.alt || image.file}/>
                                                            ) : (
                                                                <div className="image-missing">缺图</div>
                                                            )}
                                                            <figcaption>{image.file}</figcaption>
                                                        </figure>
                                                    ))}
                                                </div>
                                            )}
                                            {item.problems.map((problem) => (
                                                <p className="error-text" key={problem}>
                                                    {problem}
                                                </p>
                                            ))}
                                            {item.warnings.map((warning) => (
                                                <p className="warning-text" key={warning}>
                                                    {warning}
                                                </p>
                                            ))}
                                        </div>
                                    </article>
                                );
                            })}
                        </div>
                    )}
                </Card>
            )}

            <Card title="导入历史" subtitle="最近 20 批；导入错了可以整批撤回">
                {batches.length === 0 ? (
                    <Empty title="还没有导入过" hint="先选一个题目包试试"/>
                ) : (
                    <div className="batch-list">
                        {batches.map((batch) => (
                            <div className="batch" key={batch.id}>
                                <div className="batch-main">
                                    <strong>{batch.filename}</strong>
                                    <span className="muted small">
                                        {batch.sourceFormat.toUpperCase()} · {formatTime(batch.createdAt)} · 入库{' '}
                                        {batch.imported} / 跳过 {batch.skipped} / 失败 {batch.failed}
                                        {batch.status === 'UNDONE' && ' · 已撤销'}
                                    </span>
                                </div>
                                <div className="inline-actions">
                                    {batch.failed > 0 && (
                                        <button className="btn ghost small" onClick={() => showErrors(batch)}>
                                            看失败原因
                                        </button>
                                    )}
                                    <button
                                        className="btn ghost small"
                                        disabled={batch.remaining === 0 || batch.status === 'UNDONE'}
                                        onClick={() => undo(batch)}
                                    >
                                        撤销这一批
                                    </button>
                                </div>
                                {batchErrors[batch.id] && (
                                    <div className="batch-errors">
                                        {batchErrors[batch.id].length === 0 ? (
                                            <span className="muted small">没有逐行错误</span>
                                        ) : (
                                            batchErrors[batch.id].map((error) => (
                                                <div key={`${error.rowNo}-${error.code}`}>
                                                    第 {error.rowNo} 行（{error.code}）：{error.message}
                                                </div>
                                            ))
                                        )}
                                    </div>
                                )}
                            </div>
                        ))}
                    </div>
                )}
            </Card>
        </div>
    );
}
