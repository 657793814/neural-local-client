import {useCallback, useEffect, useRef, useState} from 'react';

/**
 * 图片放大：点页面上任何一张图就能放大看，滚轮缩放、拖动平移、Esc 关闭。
 *
 * 做成全局监听而不是给每处 `<img>` 包一层组件，是因为图分散在题面、选项、
 * 解析、复核面板、课件、答题卡预览里，逐一改造既啰嗦又容易漏。
 * 判据放在 DOM 上：只要点到的是一张「够大的图」就放大，图标、logo、头像不受影响。
 *
 * 放大层挂在最后、层级高于弹窗，所以弹窗里的图（比如 AI 制图预览）也能放大。
 */

const MIN_SCALE = 0.4;
const MAX_SCALE = 10;
/** 太小的图放大没意义，还会把图标、头像误当正文图。 */
const MIN_SIZE = 64;

function clampScale(value) {
    return Math.min(MAX_SCALE, Math.max(MIN_SCALE, value));
}

export function ImageLightbox() {
    const [current, setCurrent] = useState(null); // {src, alt}
    const [scale, setScale] = useState(1);
    const [offset, setOffset] = useState({x: 0, y: 0});
    const [dragging, setDragging] = useState(false);
    const dragRef = useRef(null);
    const imageRef = useRef(null);

    const close = useCallback(() => {
        setCurrent(null);
        setScale(1);
        setOffset({x: 0, y: 0});
        dragRef.current = null;
    }, []);

    // 全局点击：用捕获阶段，图片在 <a> 或 <button> 里也能先接住。
    useEffect(() => {
        const onClick = (event) => {
            if (event.defaultPrevented || event.button !== 0) return;
            const target = event.target;
            if (!(target instanceof HTMLImageElement)) return;
            if (target.dataset.noZoom !== undefined) return;
            if (target.closest('[data-no-zoom]')) return;
            if (!target.src) return;
            const width = target.naturalWidth || target.width;
            const height = target.naturalHeight || target.height;
            if (width < MIN_SIZE || height < MIN_SIZE) return;
            event.preventDefault();
            setScale(1);
            setOffset({x: 0, y: 0});
            setCurrent({src: target.src, alt: target.alt || ''});
        };
        document.addEventListener('click', onClick, true);
        return () => document.removeEventListener('click', onClick, true);
    }, []);

    // 打开时锁住背景滚动，关闭/卸载时恢复。
    useEffect(() => {
        if (!current) return undefined;
        const previous = document.body.style.overflow;
        document.body.style.overflow = 'hidden';
        return () => {
            document.body.style.overflow = previous;
        };
    }, [current]);

    useEffect(() => {
        if (!current) return undefined;
        const onKey = (event) => {
            if (event.key === 'Escape') {
                event.stopPropagation();
                close();
            } else if (event.key === '=' || event.key === '+') {
                setScale((value) => clampScale(value * 1.25));
            } else if (event.key === '-' || event.key === '_') {
                setScale((value) => clampScale(value / 1.25));
            } else if (event.key === '0') {
                setScale(1);
                setOffset({x: 0, y: 0});
            }
        };
        window.addEventListener('keydown', onKey, true);
        return () => window.removeEventListener('keydown', onKey, true);
    }, [current, close]);

    if (!current) return null;

    const zoomAt = (delta, event) => {
        const next = clampScale(scale * delta);
        if (Math.abs(next - scale) < 0.0001) return;
        // 以鼠标位置为中心缩放，指针下的细节不会跑掉。
        if (event) {
            const rect = event.currentTarget.getBoundingClientRect();
            const cx = event.clientX - rect.left - rect.width / 2;
            const cy = event.clientY - rect.top - rect.height / 2;
            const ratio = next / scale;
            setOffset((current) => ({
                x: current.x - (cx - current.x) * (ratio - 1),
                y: current.y - (cy - current.y) * (ratio - 1),
            }));
        }
        setScale(next);
    };

    const actualSize = () => {
        const node = imageRef.current;
        if (!node) return;
        const shown = node.getBoundingClientRect().width / (scale || 1);
        if (!shown || !node.naturalWidth) return;
        setScale(clampScale(node.naturalWidth / shown));
        setOffset({x: 0, y: 0});
    };

    const onPointerDown = (event) => {
        if (event.button !== 0) return;
        setDragging(true);
        dragRef.current = {
            startX: event.clientX,
            startY: event.clientY,
            originX: offset.x,
            originY: offset.y,
        };
        event.currentTarget.setPointerCapture(event.pointerId);
    };

    const onPointerMove = (event) => {
        const drag = dragRef.current;
        if (!drag) return;
        setOffset({
            x: drag.originX + (event.clientX - drag.startX),
            y: drag.originY + (event.clientY - drag.startY),
        });
    };

    const onPointerUp = (event) => {
        dragRef.current = null;
        setDragging(false);
        if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
        }
    };

    return (
        <div
            className="img-zoom"
            role="dialog"
            aria-modal="true"
            aria-label="图片预览"
            onClick={(event) => {
                if (event.target === event.currentTarget) close();
            }}
        >
            <div className="img-zoom-bar" onClick={(event) => event.stopPropagation()}>
                <span className="img-zoom-percent">{Math.round(scale * 100)}%</span>
                <button type="button" className="img-zoom-btn" onClick={() => zoomAt(1 / 1.25)} title="缩小（-）">
                    −
                </button>
                <button type="button" className="img-zoom-btn" onClick={() => zoomAt(1.25)} title="放大（+）">
                    ＋
                </button>
                <button type="button" className="img-zoom-btn text" onClick={actualSize} title="实际大小（0）">
                    1:1
                </button>
                <button
                    type="button"
                    className="img-zoom-btn text"
                    onClick={() => {
                        setScale(1);
                        setOffset({x: 0, y: 0});
                    }}
                    title="适应窗口"
                >
                    适应
                </button>
                <button type="button" className="img-zoom-btn" onClick={close} title="关闭（Esc）">
                    ✕
                </button>
            </div>
            <div
                className={`img-zoom-stage ${dragging ? 'dragging' : ''}`}
                onWheel={(event) => {
                    event.preventDefault();
                    zoomAt(event.deltaY < 0 ? 1.12 : 1 / 1.12, event);
                }}
                onPointerDown={onPointerDown}
                onPointerMove={onPointerMove}
                onPointerUp={onPointerUp}
                onPointerCancel={onPointerUp}
                onDoubleClick={() => {
                    setScale((value) => (value > 1.05 ? 1 : 2));
                    setOffset({x: 0, y: 0});
                }}
            >
                <img
                    ref={imageRef}
                    src={current.src}
                    alt={current.alt}
                    draggable={false}
                    className="img-zoom-img"
                    style={{transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})`}}
                />
            </div>
            <p className="img-zoom-hint">滚轮缩放 · 拖动平移 · 双击放大 · Esc 关闭</p>
        </div>
    );
}
