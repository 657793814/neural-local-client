import {useCallback, useEffect, useReducer, useRef, useState} from 'react';
import {cx} from '../lib/format.js';
import {DecodeText, CountUp, useSlidingThumb} from './effects.jsx';

// ---------------------------------------------------------------------------
// 提示与确认：与中心系统一样统一入口，页面不再各写一套。
// ---------------------------------------------------------------------------

let toastSeq = 0;
const toastListeners = new Set();

export function notify(message, tone = 'info') {
    const item = {id: ++toastSeq, message: String(message), tone};
    toastListeners.forEach((listener) => listener(item));
}

notify.success = (message) => notify(message, 'success');
notify.error = (message) => notify(message, 'danger');
notify.warning = (message) => notify(message, 'warn');
notify.info = (message) => notify(message, 'info');

let confirmResolver = null;
const confirmListeners = new Set();

export function confirmDialog(options) {
    const config = typeof options === 'string' ? {message: options} : options;
    return new Promise((resolve) => {
        confirmResolver = resolve;
        confirmListeners.forEach((listener) => listener({...config, open: true}));
    });
}

/** 毛玻璃提示：停留 5.2s，底部进度条显示剩余时间，鼠标悬停暂停。 */
function ToastItem({item, onDone}) {
    const duration = item.duration || 5200;
    const [paused, setPaused] = useState(false);
    const remaining = useRef(duration);

    useEffect(() => {
        if (paused) return undefined;
        const begin = Date.now();
        const timer = setTimeout(() => onDone(item.id), remaining.current);
        return () => {
            clearTimeout(timer);
            remaining.current = Math.max(200, remaining.current - (Date.now() - begin));
        };
    }, [paused, item.id, onDone]);

    return (
        <div
            className={cx('toast', item.tone, paused && 'paused')}
            onMouseEnter={() => setPaused(true)}
            onMouseLeave={() => setPaused(false)}
            onClick={() => onDone(item.id)}
            title="悬停暂停，点击关闭"
        >
            <span className="toast-dot" aria-hidden="true"/>
            <span className="toast-text">{item.message}</span>
            <span
                className="toast-timer"
                style={{'--toast-duration': `${duration}ms`}}
                aria-hidden="true"
            />
        </div>
    );
}

export function FeedbackHost() {
    const [items, setItems] = useState([]);
    const [confirm, setConfirm] = useState(null);
    const removeToast = useCallback((id) => {
        setItems((current) => current.filter((each) => each.id !== id));
    }, []);

    useEffect(() => {
        const onToast = (item) => {
            setItems((current) => [...current, item]);
        };
        const onConfirm = (config) => setConfirm(config);
        toastListeners.add(onToast);
        confirmListeners.add(onConfirm);
        return () => {
            toastListeners.delete(onToast);
            confirmListeners.delete(onConfirm);
        };
    }, []);

    const close = (answer) => {
        confirmResolver?.(answer);
        confirmResolver = null;
        setConfirm(null);
    };

    return (
        <>
            <div className="toasts" role="status" aria-live="polite">
                {items.map((item) => (
                    <ToastItem key={item.id} item={item} onDone={removeToast}/>
                ))}
            </div>
            {confirm && (
                <div className="modal-mask" onMouseDown={() => close(false)}>
                    <div className="modal small" onMouseDown={(event) => event.stopPropagation()}>
                        <h3>{confirm.title || '确认一下'}</h3>
                        <p className="modal-text">{confirm.message}</p>
                        <div className="modal-actions">
                            <button className="btn ghost" onClick={() => close(false)}>
                                取消
                            </button>
                            <button
                                className={cx('btn', confirm.danger ? 'danger' : 'primary')}
                                autoFocus
                                onClick={() => close(true)}
                            >
                                {confirm.confirmText || '确定'}
                            </button>
                        </div>
                    </div>
                </div>
            )}
        </>
    );
}

// ---------------------------------------------------------------------------
// 基础件
// ---------------------------------------------------------------------------

export function PageHead({title, subtitle, actions, help}) {
    return (
        <header className="page-head">
            <div className="page-head-main">
                <span className="page-eyebrow">
                    <i aria-hidden="true"/>
                    NeuralKB · Workspace
                </span>
                <h1><DecodeText text={title}/></h1>
                {subtitle && <p>{subtitle}</p>}
            </div>
            <div className="page-actions">
                {actions}
                {help && (
                    <a className="help-link" href={`#/help/${help}`} title="看这一页的说明">
                        ?
                    </a>
                )}
            </div>
        </header>
    );
}

export function Card({title, subtitle, actions, children, className, tilt}) {
    return (
        <section className={cx('card', className)} {...(tilt ? {'data-tilt': ''} : {})}>
            {(title || actions) && (
                <div className="card-head">
                    <div>
                        {title && <h2>{title}</h2>}
                        {subtitle && <p>{subtitle}</p>}
                    </div>
                    {actions && <div className="card-actions">{actions}</div>}
                </div>
            )}
            {children}
        </section>
    );
}

export function Empty({title, hint, action}) {
    return (
        <div className="empty">
            <div className="empty-mark">◇</div>
            <h3>{title}</h3>
            {hint && <p>{hint}</p>}
            {action}
        </div>
    );
}

export function Loading({text = '加载中…'}) {
    return <div className="loading">{text}</div>;
}

export function Pill({children, tone = 'muted'}) {
    return <span className={`pill ${tone}`}>{children}</span>;
}

export function Field({label, hint, children, wide}) {
    return (
        <label className={cx('field', wide && 'wide')}>
            <span className="field-label">{label}</span>
            {children}
            {hint && <span className="field-hint">{hint}</span>}
        </label>
    );
}

export function Modal({
    title,
    subtitle,
    onClose,
    children,
    footer,
    wide,
    small,
    xwide,
    bodyClassName,
    // 弹窗里再开弹窗（如选题 → 题目详情）时抬高一层，盖住下面的窗口。
    maskClassName = '',
}) {
    return (
        <div className={cx('modal-mask', maskClassName)} onMouseDown={onClose}>
            <div
                className={cx('modal liquid', wide && 'wide', small && 'small', xwide && 'xwide')}
                onMouseDown={(event) => event.stopPropagation()}
            >
                <span className="liquid-caustics" aria-hidden="true"/>
                <span className="liquid-sheen" aria-hidden="true"/>
                <span className="liquid-specular" aria-hidden="true"/>
                <span className="liquid-edge" aria-hidden="true"/>
                <div className="modal-head">
                    <div>
                        <h3>{title}</h3>
                        {subtitle && <p>{subtitle}</p>}
                    </div>
                    <button className="icon-btn" onClick={onClose} aria-label="关闭">
                        ✕
                    </button>
                </div>
                <div className={cx('modal-body', bodyClassName)}>{children}</div>
                {footer && <div className="modal-actions">{footer}</div>}
            </div>
        </div>
    );
}

export function Drawer({title, subtitle, onClose, children, footer}) {
    return (
        <div className="drawer-mask" onMouseDown={onClose}>
            <aside
                className="drawer liquid"
                onMouseDown={(event) => event.stopPropagation()}
            >
                <span className="liquid-caustics" aria-hidden="true"/>
                <span className="liquid-sheen" aria-hidden="true"/>
                <span className="liquid-specular" aria-hidden="true"/>
                <span className="liquid-edge" aria-hidden="true"/>
                <div className="modal-head">
                    <div>
                        <h3>{title}</h3>
                        {subtitle && <p>{subtitle}</p>}
                    </div>
                    <button className="icon-btn" onClick={onClose} aria-label="关闭">
                        ✕
                    </button>
                </div>
                <div className="drawer-body">{children}</div>
                {footer && <div className="drawer-actions">{footer}</div>}
            </aside>
        </div>
    );
}

export function Progress({value, label}) {
    return (
        <div className="progress-wrap">
            <div className="progress">
                <div className="progress-bar" style={{width: `${Math.max(3, value)}%`}}/>
            </div>
            {label && <span className="progress-label">{label}</span>}
        </div>
    );
}

export function Tabs({items, value, onChange}) {
    const [ref, thumb] = useSlidingThumb(value);
    return (
        <div className="tabs" role="tablist" ref={ref}>
            {thumb && (
                <span
                    className="tabs-thumb"
                    aria-hidden="true"
                    style={{
                        transform: `translate3d(${thumb.x}px, ${thumb.y}px, 0)`,
                        width: thumb.width,
                        height: thumb.height,
                    }}
                />
            )}
            {items.map((item) => (
                <button
                    key={item.value}
                    role="tab"
                    aria-selected={value === item.value}
                    className={cx('tab', value === item.value && 'active')}
                    onClick={() => onChange(item.value)}
                >
                    {item.label}
                    {item.count > 0 && (
                        <span className="tab-count">
                            <CountUp value={item.count} duration={720}/>
                        </span>
                    )}
                </button>
            ))}
        </div>
    );
}

/** 点击外部关闭的下拉，用于筛选与操作菜单。 */
export function Dropdown({label, children, className}) {
    const [open, setOpen] = useState(false);
    const ref = useRef(null);
    useEffect(() => {
        if (!open) return undefined;
        const onDown = (event) => {
            if (ref.current && !ref.current.contains(event.target)) setOpen(false);
        };
        document.addEventListener('mousedown', onDown);
        return () => document.removeEventListener('mousedown', onDown);
    }, [open]);
    return (
        <div className={cx('dropdown', className)} ref={ref}>
            <button className="btn ghost" onClick={() => setOpen((value) => !value)}>
                {label} <span className="caret">▾</span>
            </button>
            {open && <div className="dropdown-menu" onClick={() => setOpen(false)}>{children}</div>}
        </div>
    );
}

export function useAsync(work, deps = []) {
    const [state, setState] = useState({loading: true, data: null, error: null});
    const [, force] = useReducer((value) => value + 1, 0);
    useEffect(() => {
        let alive = true;
        setState((current) => ({...current, loading: true}));
        work()
            .then((data) => alive && setState({loading: false, data, error: null}))
            .catch((error) => alive && setState({loading: false, data: null, error: error.message}));
        return () => {
            alive = false;
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, deps);
    return {...state, reload: force};
}

/**
 * 可点击复制的错误块：报错时把完整错误信息一键复制到剪贴板，
 * 方便用户贴到反馈里。点击后短暂显示「已复制」。
 */
export function CopyableError({text, className}) {
    const [copied, setCopied] = useState(false);
    if (!text) return null;

    const copy = async () => {
        try {
            await navigator.clipboard.writeText(text);
            setCopied(true);
            setTimeout(() => setCopied(false), 1600);
        } catch {
            // clipboard API 不可用时降级：选中文本让用户手动 ⌘C
            const range = document.createRange();
            const node = document.getElementById('copyable-error-text');
            if (node) {
                range.selectNodeContents(node);
                const sel = window.getSelection();
                sel.removeAllRanges();
                sel.addRange(range);
            }
        }
    };

    return (
        <div
            className={`error-copy-block ${className || ''}`}
            onClick={copy}
            role="button"
            tabIndex={0}
            title="点击复制错误信息"
            onKeyDown={(e) => e.key === 'Enter' && copy()}
        >
            <p className="error-text" id="copyable-error-text">{text}</p>
            <span className="error-copy-hint">{copied ? '✓ 已复制' : '点击复制'}</span>
        </div>
    );
}
