import {useEffect, useRef, useState} from 'react';
import {paperSize} from '../../lib/paper/model.js';
import {PREVIEW_PAGE_GAP_MM} from '../../lib/paper/render.js';
import {Loading, Pill} from '../ui.jsx';

export const MM_TO_PX = 96 / 25.4;

/** 预览纸堆的总高度：正文页 + 页间灰缝。编辑器与全屏预览共用，避免各算一套。 */
export function previewPaperHeight(layout, pageCount, scale = 1) {
    const size = paperSize(layout);
    const count = Math.max(1, pageCount || 1);
    const gap = PREVIEW_PAGE_GAP_MM * MM_TO_PX * (count - 1);
    return (size.height * MM_TO_PX * count + gap) * scale;
}

/** 一份 HTML、一个 iframe：预览与打印完全同源。 */
export function PrintPreviewFrame({html, layout, pageCount, scale = 1}) {
    const size = paperSize(layout);
    const width = size.width * MM_TO_PX;
    const height = previewPaperHeight(layout, pageCount, 1);
    return (
        <div className="ppf" style={{width: width * scale, height: height * scale}}>
            <iframe
                title="试卷预览"
                srcDoc={html}
                style={{
                    width,
                    height,
                    transform: `scale(${scale})`,
                    transformOrigin: 'top left',
                }}
            />
        </div>
    );
}

/** 全屏打印预览：学生版 / 教师版切换、缩放、打印、导出。 */
export default function PaperPreview({
    doc,
    html,
    pageCount,
    warnings = [],
    variant,
    onVariant,
    onClose,
    onPrint,
    onExport,
    rendering,
}) {
    const boxRef = useRef(null);
    const [fit, setFit] = useState(0.85);
    const [zoom, setZoom] = useState(null);

    useEffect(() => {
        const element = boxRef.current;
        if (!element) return undefined;
        const compute = () => {
            const size = paperSize(doc?.layout);
            const width = size.width * MM_TO_PX;
            const available = element.clientWidth - 64;
            setFit(Math.min(1.2, Math.max(0.3, available / width)));
        };
        compute();
        const observer = new ResizeObserver(compute);
        observer.observe(element);
        return () => observer.disconnect();
    }, [doc?.layout]);

    const scale = zoom || fit;

    return (
        <div className="paper-preview-full">
            <header className="pp-head">
                <div className="pp-title">
                    <button className="btn ghost small" onClick={onClose}>
                        ← 返回编辑
                    </button>
                    <strong>{doc?.title || '试卷预览'}</strong>
                    <Pill tone="line">{pageCount ? `${pageCount} 页` : '排版中'}</Pill>
                </div>
                <div className="pp-actions">
                    <div className="seg">
                        <button
                            className={variant === 'STUDENT' ? 'active' : ''}
                            onClick={() => onVariant('STUDENT')}
                        >
                            学生版
                        </button>
                        <button
                            className={variant === 'TEACHER' ? 'active' : ''}
                            onClick={() => onVariant('TEACHER')}
                        >
                            教师版（含答案）
                        </button>
                    </div>
                    <div className="seg">
                        <button onClick={() => setZoom(Math.max(0.3, scale - 0.1))}>−</button>
                        <span className="seg-label">{Math.round(scale * 100)}%</span>
                        <button onClick={() => setZoom(Math.min(1.6, scale + 0.1))}>＋</button>
                        <button onClick={() => setZoom(null)}>适应</button>
                    </div>
                    <button className="btn ghost" onClick={onExport} disabled={!html}>
                        导出 HTML
                    </button>
                    <button className="btn primary" onClick={onPrint} disabled={!html}>
                        打印 / 另存 PDF
                    </button>
                </div>
            </header>

            {warnings.length > 0 && (
                <div className="pp-warnings">
                    {warnings.map((warning) => (
                        <span key={warning}>{warning}</span>
                    ))}
                </div>
            )}

            <div className="pp-body" ref={boxRef}>
                {rendering && !html ? (
                    <Loading text="正在排版…"/>
                ) : html ? (
                    <PrintPreviewFrame
                        html={html}
                        layout={doc?.layout}
                        pageCount={pageCount}
                        scale={scale}
                    />
                ) : (
                    <div className="muted">还没有内容，先从题库加几道题。</div>
                )}
            </div>
        </div>
    );
}
