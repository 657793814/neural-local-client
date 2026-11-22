import {useState} from 'react';
import {diagramGenerate} from '../lib/api.js';
import {Modal, notify} from './ui.jsx';

/**
 * AI 制图弹窗：按题干（或指定描述）生成一张 SVG 示意图，确认后把
 * `![](图N)` 这样的引用交给调用方插入题干 / 选项 / 解析。
 *
 * 图形本身先落在临时目录（后端 diagram_generate 完成），返回 path；
 * 草稿提交时 stage_figures 会把临时文件转成正式 question_asset，
 * 所以这里只需要把 {ref, path} 交给调用方写进 item.figures。
 *
 * 后端失败时不会静默：错误原文留在弹窗里，并给「重试」「换个模式」两个出口——
 * 制图失败大多是模型这一次输出不完整，换个模式或再试一次通常就成。
 *
 * @param {string} stem       预填的题干（可改）
 * @param {string} subjectId  学科 id
 * @param {string} refName    插入用的引用名，如「图1」
 * @param {function} onInsert 确认：(svg, dataUrl, path, refName) => void
 */
export function DiagramDialog({stem = '', subjectId, refName = '图1', onInsert, onClose}) {
    const [text, setText] = useState(stem);
    const [focus, setFocus] = useState('');
    const [mode, setMode] = useState('general');
    const [busy, setBusy] = useState(false);
    const [result, setResult] = useState(null);
    const [error, setError] = useState('');

    const generate = async (nextMode = mode) => {
        if (!text.trim()) {
            notify.warning('先写清楚题干或作图描述');
            return;
        }
        setMode(nextMode);
        setBusy(true);
        setResult(null);
        setError('');
        try {
            setResult(await diagramGenerate(subjectId, text, focus, nextMode));
        } catch (failure) {
            setError(failure.message || '生成失败，请重试');
        } finally {
            setBusy(false);
        }
    };

    const insert = () => {
        if (!result) return;
        onInsert?.(result.svg, result.dataUrl, result.path, refName);
        onClose?.();
    };

    const otherMode = mode === 'math' ? 'general' : 'math';

    return (
        <Modal
            wide
            title="AI 制图"
            subtitle="根据题干生成矢量示意图；失败会自动重画一次，仍然不成再手动重试"
            onClose={onClose}
            footer={
                <>
                    <button className="btn ghost" onClick={onClose}>取消</button>
                    {result && (
                        <button className="btn primary" onClick={insert}>插入图形</button>
                    )}
                </>
            }
        >
            <div className="diagram-form">
                <label className="field">
                    <span className="field-label">题干 / 作图描述</span>
                    <textarea
                        rows={4}
                        value={text}
                        onChange={(event) => setText(event.target.value)}
                        placeholder="把题干粘进来，或用一句话描述要画的图，例如：画出抛物线 y=x²-2x 与 x 轴的交点"
                    />
                </label>
                <label className="field">
                    <span className="field-label">作图重点（可留空）</span>
                    <input
                        value={focus}
                        onChange={(event) => setFocus(event.target.value)}
                        placeholder="例如：标出顶点与对称轴；P 在底面中心正上方"
                    />
                </label>
                <div className="field">
                    <span className="field-label">图形类型</span>
                    <div className="chip-row">
                        <button
                            type="button"
                            className={`chip ${mode === 'general' ? 'active' : ''}`}
                            onClick={() => setMode('general')}
                        >
                            通用示意图
                        </button>
                        <button
                            type="button"
                            className={`chip ${mode === 'math' ? 'active' : ''}`}
                            onClick={() => setMode('math')}
                        >
                            数学精确图形
                        </button>
                    </div>
                    <span className="field-hint">
                        数学模式强调坐标系、几何关系与关键点标注；两种模式都走你配置的模型，离线渲染
                    </span>
                </div>

                <div className="diagram-actions">
                    <button className="btn primary" onClick={() => generate()} disabled={busy || !text.trim()}>
                        {busy ? '作图中…' : result ? '重新生成' : '生成图形'}
                    </button>
                </div>

                {busy && (
                    <p className="diagram-progress">正在作图，通常 10～40 秒；复杂图形失败会自动重画一次。</p>
                )}

                {error && !busy && (
                    <div className="diagram-error">
                        <div className="diagram-error-head">这次没画成</div>
                        <p className="diagram-error-body">{error}</p>
                        <div className="diagram-error-actions">
                            <button className="btn primary small" onClick={() => generate()} disabled={busy}>
                                重试
                            </button>
                            <button className="btn ghost small" onClick={() => generate(otherMode)} disabled={busy}>
                                改用{otherMode === 'math' ? '数学精确图形' : '通用示意图'}
                            </button>
                        </div>
                        <p className="field-hint">
                            连续失败时，把点、线的位置关系写进「作图重点」比反复重试更有效。
                        </p>
                    </div>
                )}

                {result && (
                    <div className="diagram-preview">
                        <div className="diagram-preview-head">预览</div>
                        <div className="diagram-preview-canvas">
                            <img src={result.dataUrl} alt="AI 生成的示意图"/>
                        </div>
                    </div>
                )}
            </div>
        </Modal>
    );
}
