import {useEffect, useRef, useState} from 'react';
import {imageFromPath, smartCrop} from '../lib/api.js';
import {Modal, notify} from './ui.jsx';

/**
 * 图形框校对：模型裁出来的图经常圈多了（带进相邻文字、题号、整页留白）。
 * 这里给一个可以拖的框——左边是截图原图加框，右边是按当前框实时裁出来的效果，
 * 所见即所得；确认后重新裁一张，把旧的临时图换掉。
 *
 * 坐标一律用 0–1 的归一化值（和 items.figures[].box 一致），裁的时候再乘原图像素，
 * 这样窗口大小、图片缩放都不影响结果。
 */

const MIN = 0.05;
const FULL = {x: 0, y: 0, w: 1, h: 1};
const PREVIEW_W = 260;

const clamp01 = (value) => Math.max(0, Math.min(1, value));

function sanitize(box) {
    const x = clamp01(Number(box?.x) || 0);
    const y = clamp01(Number(box?.y) || 0);
    const w = Math.min(clamp01(Number(box?.w) || 0), 1 - x);
    const h = Math.min(clamp01(Number(box?.h) || 0), 1 - y);
    if (w < MIN || h < MIN) return {...FULL};
    return {x, y, w, h};
}

/** 拖动 / 缩放框。mode：move 或 nw / ne / sw / se。 */
function applyDrag(box, mode, dx, dy) {
    let left = box.x;
    let top = box.y;
    let right = box.x + box.w;
    let bottom = box.y + box.h;
    if (mode === 'move') {
        return {
            x: clamp01(Math.min(Math.max(box.x + dx, 0), 1 - box.w)),
            y: clamp01(Math.min(Math.max(box.y + dy, 0), 1 - box.h)),
            w: box.w,
            h: box.h,
        };
    }
    if (mode.includes('w')) left = Math.min(Math.max(box.x + dx, 0), right - MIN);
    if (mode.includes('e')) right = Math.max(Math.min(box.x + box.w + dx, 1), left + MIN);
    if (mode.includes('n')) top = Math.min(Math.max(box.y + dy, 0), bottom - MIN);
    if (mode.includes('s')) bottom = Math.max(Math.min(box.y + box.h + dy, 1), top + MIN);
    return {x: left, y: top, w: right - left, h: bottom - top};
}

export function FigureCropDialog({sourcePath, box, auto, refName = '图1', onApply, onClose}) {
    const [src, setSrc] = useState('');
    const [natural, setNatural] = useState(null);
    const [current, setCurrent] = useState(() => sanitize(box || auto));
    const [drag, setDrag] = useState(null);
    const [busy, setBusy] = useState(false);
    const frameRef = useRef(null);

    const autoBox = sanitize(auto || FULL);

    useEffect(() => {
        let alive = true;
        if (!sourcePath) return () => {};
        imageFromPath(sourcePath)
            .then((result) => {
                if (alive) setSrc(result.dataUrl || '');
            })
            .catch((error) => {
                if (alive) notify.error(error.message || '读不了原图');
            });
        return () => {
            alive = false;
        };
    }, [sourcePath]);

    useEffect(() => {
        if (!drag) return undefined;
        const move = (event) => {
            const rect = frameRef.current?.getBoundingClientRect();
            if (!rect) return;
            const dx = (event.clientX - drag.startX) / Math.max(1, rect.width);
            const dy = (event.clientY - drag.startY) / Math.max(1, rect.height);
            setCurrent(applyDrag(drag.box, drag.mode, dx, dy));
        };
        const stop = () => setDrag(null);
        window.addEventListener('pointermove', move);
        window.addEventListener('pointerup', stop);
        window.addEventListener('pointercancel', stop);
        return () => {
            window.removeEventListener('pointermove', move);
            window.removeEventListener('pointerup', stop);
            window.removeEventListener('pointercancel', stop);
        };
    }, [drag]);

    const begin = (event, mode) => {
        const rect = frameRef.current?.getBoundingClientRect();
        if (!rect) return;
        event.preventDefault();
        event.stopPropagation();
        setDrag({mode, startX: event.clientX, startY: event.clientY, box: current});
    };

    const apply = async () => {
        if (!sourcePath) return;
        if (!natural) {
            notify.warning('原图还没读出来，稍等一下再点');
            return;
        }
        const region = {
            x: Math.round(current.x * natural.width),
            y: Math.round(current.y * natural.height),
            w: Math.max(8, Math.round(current.w * natural.width)),
            h: Math.max(8, Math.round(current.h * natural.height)),
        };
        setBusy(true);
        try {
            const cropped = await smartCrop(sourcePath, region);
            onApply?.(cropped.path, current);
            onClose?.();
        } catch (error) {
            notify.error(error.message || '重新裁图失败');
        } finally {
            setBusy(false);
        }
    };

    const preview = natural
        ? (() => {
              const scale = PREVIEW_W / Math.max(1, current.w * natural.width);
              return {
                  width: PREVIEW_W,
                  height: Math.max(56, current.h * natural.height * scale),
                  backgroundImage: `url(${src})`,
                  backgroundSize: `${natural.width * scale}px ${natural.height * scale}px`,
                  backgroundPosition: `${-current.x * natural.width * scale}px ${-current.y * natural.height * scale}px`,
              };
          })()
        : {width: PREVIEW_W, height: 96};

    return (
        <Modal
            wide
            title={refName ? `调整${refName}的图框` : '从截图里补一张图'}
            subtitle="拖框或拖四角，只留图形本身；右边是裁出来的样子"
            onClose={onClose}
            footer={
                <>
                    <button className="btn ghost" onClick={onClose}>
                        取消
                    </button>
                    <button className="btn primary" onClick={apply} disabled={busy || !src}>
                        {busy ? '正在裁…' : '用这个框'}
                    </button>
                </>
            }
        >
            <div className="figure-crop">
                <div className="figure-crop-head">
                    <span className="muted small">
                        框 {Math.round(current.w * 100)}% × {Math.round(current.h * 100)}%
                        {natural ? ` · 原图 ${natural.width}×${natural.height}` : ''}
                    </span>
                    <span className="inline-actions">
                        <button
                            className="btn ghost small"
                            onClick={() => setCurrent(autoBox)}
                            disabled={!auto}
                        >
                            恢复自动框
                        </button>
                        <button className="btn ghost small" onClick={() => setCurrent({...FULL})}>
                            用整张
                        </button>
                    </span>
                </div>
                <div className="figure-crop-body">
                    <div className="figure-crop-canvas">
                        {src ? (
                            <div className="figure-crop-frame" ref={frameRef}>
                                <img
                                    src={src}
                                    alt="截图原图"
                                    draggable={false}
                                    onLoad={(event) =>
                                        setNatural({
                                            width: event.target.naturalWidth || 1,
                                            height: event.target.naturalHeight || 1,
                                        })
                                    }
                                />
                                <div
                                    className="figure-crop-box"
                                    role="presentation"
                                    style={{
                                        left: `${current.x * 100}%`,
                                        top: `${current.y * 100}%`,
                                        width: `${current.w * 100}%`,
                                        height: `${current.h * 100}%`,
                                    }}
                                    onPointerDown={(event) => begin(event, 'move')}
                                >
                                    {['nw', 'ne', 'sw', 'se'].map((corner) => (
                                        <i
                                            key={corner}
                                            className={`figure-crop-handle ${corner}`}
                                            onPointerDown={(event) => begin(event, corner)}
                                        />
                                    ))}
                                </div>
                            </div>
                        ) : (
                            <div className="figure-crop-loading muted small">正在读原图…</div>
                        )}
                    </div>
                    <div className="figure-crop-side">
                        <span className="muted small">裁切结果</span>
                        <div className="figure-crop-preview" style={preview}/>
                        <span className="muted small">
                            只框图形：坐标轴、图上标注留着，题干文字和相邻题目别圈进来
                        </span>
                    </div>
                </div>
            </div>
        </Modal>
    );
}
