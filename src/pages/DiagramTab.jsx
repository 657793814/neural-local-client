import {useState} from 'react';
import {assetAdd, diagramGenerate, listQuestions} from '../lib/api.js';
import {saveTextFile} from '../lib/format.js';
import {Card, Empty, Field, Modal, notify} from '../components/ui.jsx';
import {Rich} from '../lib/Math.jsx';
import {cx} from '../lib/format.js';

/**
 * 独立的「AI 制图」页：不用先出题，进来就能按题干/描述生成矢量示意图。
 * 生成后可以：保存为 SVG 文件、复制 SVG 代码、挂到题库里已有的某道题上。
 */
export default function DiagramTab({tree = []}) {
    const [subjectId, setSubjectId] = useState(tree[0]?.id || '');
    const [stem, setStem] = useState('');
    const [focus, setFocus] = useState('');
    const [mode, setMode] = useState('general');
    const [busy, setBusy] = useState(false);
    const [result, setResult] = useState(null);
    const [error, setError] = useState('');
    const [pickerOpen, setPickerOpen] = useState(false);

    const generate = async (nextMode = mode) => {
        if (!stem.trim()) {
            notify.warning('先写清楚题干或作图描述');
            return;
        }
        setMode(nextMode);
        setBusy(true);
        setResult(null);
        setError('');
        try {
            setResult(await diagramGenerate(subjectId, stem, focus, nextMode));
        } catch (failure) {
            setError(failure.message || '生成失败，请重试');
        } finally {
            setBusy(false);
        }
    };

    const saveSvg = async () => {
        if (!result) return;
        try {
            const saved = await saveTextFile(
                `ai-diagram-${Date.now()}.svg`,
                result.svg,
                [{name: 'SVG 矢量图', extensions: ['svg']}],
            );
            if (saved) notify.success(`已保存：${saved}`);
        } catch (error) {
            notify.error(error.message);
        }
    };

    const copySvg = async () => {
        if (!result) return;
        try {
            await navigator.clipboard.writeText(result.svg);
            notify.success('SVG 代码已复制');
        } catch (error) {
            notify.error('复制失败：' + error.message);
        }
    };

    return (
        <div className="generate-grid">
            <Card
                title="作图条件"
                subtitle="按题干生成矢量示意图；两种模式都走你配置的模型，离线渲染"
            >
                <div className="form-grid">
                    <label className="field">
                        <span className="field-label">学科</span>
                        <select value={subjectId} onChange={(event) => setSubjectId(event.target.value)}>
                            {tree.map((subject) => (
                                <option key={subject.id} value={subject.id}>{subject.name}</option>
                            ))}
                        </select>
                    </label>
                    <label className="field wide">
                        <span className="field-label">题干 / 作图描述</span>
                        <textarea
                            rows={5}
                            value={stem}
                            onChange={(event) => setStem(event.target.value)}
                            placeholder="把题干粘进来，或用一句话描述要画的图，例如：画出抛物线 y=x²-2x 与 x 轴的交点"
                        />
                    </label>
                    <label className="field wide">
                        <span className="field-label">作图重点（可留空）</span>
                        <input
                            value={focus}
                            onChange={(event) => setFocus(event.target.value)}
                            placeholder="例如：标出顶点与对称轴；画出受力方向"
                        />
                    </label>
                    <div className="field wide">
                        <span className="field-label">图形类型</span>
                        <div className="chip-row">
                            <button
                                type="button"
                                className={cx('chip', mode === 'general' && 'active')}
                                onClick={() => setMode('general')}
                            >
                                通用示意图
                            </button>
                            <button
                                type="button"
                                className={cx('chip', mode === 'math' && 'active')}
                                onClick={() => setMode('math')}
                            >
                                数学精确图形
                            </button>
                        </div>
                        <span className="field-hint">
                            数学模式强调坐标系、几何关系与关键点标注
                        </span>
                    </div>
                </div>
                <div className="diagram-actions">
                    <button className="btn primary" onClick={() => generate()} disabled={busy || !stem.trim()}>
                        {busy ? '作图中…' : result ? '重新生成' : '生成图形'}
                    </button>
                </div>

                {busy && (
                    <p className="diagram-progress">正在作图，通常 10～40 秒；失败会自动重画一次。</p>
                )}

                {error && !busy && (
                    <div className="diagram-error">
                        <div className="diagram-error-head">这次没画成</div>
                        <p className="diagram-error-body">{error}</p>
                        <div className="diagram-error-actions">
                            <button className="btn primary small" onClick={() => generate()} disabled={busy}>
                                重试
                            </button>
                            <button
                                className="btn ghost small"
                                onClick={() => generate(mode === 'math' ? 'general' : 'math')}
                                disabled={busy}
                            >
                                改用{mode === 'math' ? '通用示意图' : '数学精确图形'}
                            </button>
                        </div>
                    </div>
                )}
            </Card>

            <Card
                title="图形预览"
                subtitle={result ? '确认后可保存到本地，或挂到一道已有的题上' : undefined}
            >
                {!result ? (
                    <Empty title="还没有图形" hint="在左边填题干，点「生成图形」"/>
                ) : (
                    <div className="diagram-preview">
                        <div className="diagram-preview-canvas diagram-preview-large">
                            <img src={result.dataUrl} alt="AI 生成的示意图"/>
                        </div>
                        <div className="diagram-preview-actions">
                            <button className="btn primary" onClick={() => setPickerOpen(true)}>
                                挂到题目
                            </button>
                            <button className="btn ghost" onClick={saveSvg}>保存 SVG</button>
                            <button className="btn ghost" onClick={copySvg}>复制代码</button>
                        </div>
                    </div>
                )}
            </Card>

            {pickerOpen && result && (
                <AttachPicker
                    dataUrl={result.dataUrl}
                    onClose={() => setPickerOpen(false)}
                />
            )}
        </div>
    );
}

/** 选题并挂图：搜索题库/草稿箱，选中一道题后把 SVG 作为配图存进去。 */
function AttachPicker({dataUrl, onClose}) {
    const [scope, setScope] = useState('published');
    const [keyword, setKeyword] = useState('');
    const [rows, setRows] = useState([]);
    const [loading, setLoading] = useState(false);
    const [saving, setSaving] = useState('');

    const search = async () => {
        setLoading(true);
        try {
            const query = {page: 1, size: 30};
            if (keyword.trim()) query.keyword = keyword.trim();
            if (scope === 'draft') query.needsReview = true;
            const page = await listQuestions(query);
            setRows(page.items || []);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setLoading(false);
        }
    };

    const attach = async (row) => {
        setSaving(row.id);
        try {
            await assetAdd(row.id, dataUrl, 'DIAGRAM');
            notify.success('已挂到该题的配图');
            onClose();
        } catch (error) {
            notify.error(error.message);
        } finally {
            setSaving('');
        }
    };

    return (
        <Modal
            wide
            title="挂到题目"
            subtitle="搜到目标题目后点「挂上」；图形会作为该题的配图保存"
            onClose={onClose}
            footer={<button className="btn ghost" onClick={onClose}>关闭</button>}
        >
            <div className="attach-picker">
                <div className="classify-toolbar">
                    <select value={scope} onChange={(event) => { setScope(event.target.value); setRows([]); }}>
                        <option value="published">题库（正式题）</option>
                        <option value="draft">草稿箱</option>
                    </select>
                    <input
                        className="classify-search"
                        value={keyword}
                        onChange={(event) => setKeyword(event.target.value)}
                        onKeyDown={(event) => { if (event.key === 'Enter') search(); }}
                        placeholder="搜题干关键词…"
                    />
                    <button className="btn ghost small" onClick={search}>搜索</button>
                </div>
                {loading ? (
                    <div className="loading">加载中…</div>
                ) : rows.length === 0 ? (
                    <Empty title="没有匹配的题目" hint="换个关键词，或切到草稿箱看看"/>
                ) : (
                    <div className="attach-list">
                        {rows.map((row) => (
                            <div className="attach-row" key={row.id}>
                                <span className="attach-stem"><Rich text={row.stem} className="inline"/></span>
                                <button
                                    className="btn ghost small"
                                    disabled={saving === row.id}
                                    onClick={() => attach(row)}
                                >
                                    {saving === row.id ? '挂图中…' : '挂上'}
                                </button>
                            </div>
                        ))}
                    </div>
                )}
            </div>
        </Modal>
    );
}
