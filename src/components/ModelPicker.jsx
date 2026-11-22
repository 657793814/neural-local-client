import {useEffect, useRef, useState} from 'react';
import {listRemoteModels} from '../lib/api.js';
import {notify} from './ui.jsx';
import {cx} from '../lib/format.js';

/**
 * 模型名输入框：可直接手填，也可以「拉取列表」从服务商读回全部模型 ID 再选。
 *
 * 弹层用 position:fixed 挂在输入行下面——modal-body 是 overflow:auto，
 * absolute 会被裁掉一半。滚动或缩放窗口时直接收起，不做跟随重算。
 */
export default function ModelPicker({form, setForm, savedId}) {
    const [options, setOptions] = useState([]);
    const [loading, setLoading] = useState(false);
    const [openList, setOpenList] = useState(false);
    const [filtering, setFiltering] = useState(false);
    const [active, setActive] = useState(-1);
    const [sourceHost, setSourceHost] = useState('');
    const [anchor, setAnchor] = useState(null);
    const boxRef = useRef(null);
    const rowRef = useRef(null);
    const menuRef = useRef(null);

    const typed = form.model.trim();
    const list = filtering && typed
        ? options.filter((id) => id.toLowerCase().includes(typed.toLowerCase()))
        : options;

    // 弹层用 fixed 定位：modal-body 有 overflow，absolute 会被裁掉。
    const openListAt = () => {
        const rect = rowRef.current?.getBoundingClientRect();
        if (!rect) return;
        const below = window.innerHeight - rect.bottom;
        const up = below < 300 && rect.top > below;
        setAnchor({
            left: rect.left,
            width: rect.width,
            maxHeight: Math.max(150, Math.min(264, (up ? rect.top : below) - 22)),
            ...(up ? {bottom: window.innerHeight - rect.top + 8} : {top: rect.bottom + 8}),
        });
        setOpenList(true);
    };

    const closeList = () => {
        setOpenList(false);
        setActive(-1);
    };

    useEffect(() => {
        if (!openList) return;
        const onDown = (event) => {
            if (!boxRef.current?.contains(event.target)) setOpenList(false);
        };
        const onMove = () => setOpenList(false);
        document.addEventListener('mousedown', onDown);
        window.addEventListener('scroll', onMove, true);
        window.addEventListener('resize', onMove);
        return () => {
            document.removeEventListener('mousedown', onDown);
            window.removeEventListener('scroll', onMove, true);
            window.removeEventListener('resize', onMove);
        };
    }, [openList]);

    useEffect(() => {
        if (!openList || active < 0) return;
        menuRef.current?.querySelector('.model-option.active')?.scrollIntoView({block: 'nearest'});
    }, [active, openList]);

    const pick = (id) => {
        setForm((prev) => ({...prev, model: id, name: prev.name.trim() ? prev.name : id}));
        closeList();
        setFiltering(false);
    };

    const fetchList = async () => {
        const base = form.baseUrl.trim();
        if (!base) {
            notify('先填 Base URL，例如 https://api.example.com/v1');
            return;
        }
        setLoading(true);
        try {
            const ids = await listRemoteModels(base, form.apiKey.trim() || null, savedId || null);
            setOptions(ids);
            setSourceHost(base.replace(/^https?:\/\//, '').split('/')[0]);
            setFiltering(false);
            setActive(-1);
            openListAt();
            if (ids.length === 1) pick(ids[0]);
        } catch (error) {
            setOptions([]);
            notify.error(error.message);
        } finally {
            setLoading(false);
        }
    };

    const onKeyDown = (event) => {
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            if (!options.length) return;
            event.preventDefault();
            if (!openList) {
                openListAt();
                setFiltering(false);
                setActive(0);
                return;
            }
            const step = event.key === 'ArrowDown' ? 1 : -1;
            setActive((index) => (index + step + list.length) % list.length);
            return;
        }
        if (event.key === 'Enter' && openList && list[active]) {
            event.preventDefault();
            pick(list[active]);
            return;
        }
        if (event.key === 'Escape' && openList) {
            event.preventDefault();
            closeList();
        }
    };

    const hint = options.length
        ? `已从 ${sourceHost} 拉到 ${options.length} 个模型${filtering && typed ? `，${list.length} 个匹配` : ''}；也可以直接手填`
        : '填完 Base URL 与 Key，点「拉取列表」自动获取；也可以直接手填服务商给的 ID';

    return (
        <div className="field wide">
            <span className="field-label">模型名</span>
            <div className="model-picker" ref={boxRef}>
                <div className="model-picker-row" ref={rowRef}>
                    <input
                        value={form.model}
                        placeholder="gpt-4o / deepseek-chat / qwen2.5:7b"
                        onChange={(event) => {
                            setForm({...form, model: event.target.value});
                            setFiltering(true);
                            setActive(-1);
                            if (options.length) openListAt();
                        }}
                        onKeyDown={onKeyDown}
                        onFocus={() => options.length && openListAt()}
                    />
                    <button type="button" className="btn small" onClick={fetchList} disabled={loading}>
                        {loading ? '拉取中…' : options.length ? '重新拉取' : '拉取列表'}
                    </button>
                </div>
                {openList && anchor && (
                    <div className="model-menu" ref={menuRef} role="listbox" style={anchor}>
                        <div className="model-menu-head">
                            {filtering && typed ? `${list.length} / ${options.length} 个匹配` : `${options.length} 个模型`}
                        </div>
                        {list.length === 0 ? (
                            <div className="model-menu-empty">没有匹配的模型，可以直接用当前输入</div>
                        ) : (
                            list.map((id, index) => (
                                <button
                                    type="button"
                                    key={id}
                                    role="option"
                                    aria-selected={id === form.model}
                                    className={cx(
                                        'model-option',
                                        index === active && 'active',
                                        id === form.model && 'picked'
                                    )}
                                    onMouseEnter={() => setActive(index)}
                                    onClick={() => pick(id)}
                                >
                                    {id}
                                </button>
                            ))
                        )}
                    </div>
                )}
            </div>
            <span className="field-hint">{hint}</span>
        </div>
    );
}
