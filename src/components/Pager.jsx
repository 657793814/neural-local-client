import {useEffect, useState} from 'react';

const SIZES = [20, 50, 100];

/** 页码条：页数少就全列，页数多就用首尾 + 当前页前后各一页。 */
function pageList(page, pageCount) {
    if (pageCount <= 7) {
        return Array.from({length: pageCount}, (_, index) => index + 1);
    }
    const out = [1];
    const start = Math.max(2, page - 1);
    const end = Math.min(pageCount - 1, page + 1);
    if (start > 2) out.push('…');
    for (let value = start; value <= end; value += 1) out.push(value);
    if (end < pageCount - 1) out.push('…');
    out.push(pageCount);
    return out;
}

/**
 * 通用翻页条：范围提示 + 页码 + 每页条数 + 跳页。
 * page 从 1 开始；onPage / onSize 由调用方负责重新取数（并自行回到列表顶部）。
 */
export function Pager({page, pageCount, total, size, onPage, onSize, unit = '道'}) {
    const [jump, setJump] = useState('');
    const safeCount = Math.max(1, pageCount);

    useEffect(() => {
        setJump('');
    }, [page, size]);

    if (!total) return null;

    const from = (page - 1) * size + 1;
    const to = Math.min(total, page * size);

    const submit = (event) => {
        if (event.key !== 'Enter') return;
        const value = Number(jump);
        if (Number.isFinite(value) && value > 0) {
            onPage(Math.min(safeCount, Math.max(1, Math.trunc(value))));
        }
        setJump('');
        event.currentTarget.blur();
    };

    return (
        <nav className="pager" aria-label="翻页">
            <div className="pager-info">
                第 <strong>{from}</strong>–<strong>{to}</strong> {unit} · 共 <strong>{total}</strong> {unit}
            </div>
            <div className="pager-pages">
                <button className="pg edge" disabled={page <= 1} onClick={() => onPage(1)} title="第一页">
                    «
                </button>
                <button className="pg" disabled={page <= 1} onClick={() => onPage(page - 1)} title="上一页">
                    ‹
                </button>
                {pageList(page, safeCount).map((item, index) =>
                    item === '…' ? (
                        <span key={`gap-${index}`} className="pg-gap">
                            …
                        </span>
                    ) : (
                        <button
                            key={item}
                            className={item === page ? 'pg active' : 'pg'}
                            aria-current={item === page ? 'page' : undefined}
                            onClick={() => onPage(item)}
                        >
                            {item}
                        </button>
                    ),
                )}
                <button className="pg" disabled={page >= safeCount} onClick={() => onPage(page + 1)} title="下一页">
                    ›
                </button>
                <button
                    className="pg edge"
                    disabled={page >= safeCount}
                    onClick={() => onPage(safeCount)}
                    title="最后一页"
                >
                    »
                </button>
            </div>
            <div className="pager-tools">
                <label className="pager-size">
                    每页
                    <select value={size} onChange={(event) => onSize(Number(event.target.value))}>
                        {SIZES.map((value) => (
                            <option key={value} value={value}>
                                {value}
                            </option>
                        ))}
                    </select>
                    {unit}
                </label>
                {safeCount > 1 && (
                    <label className="pager-jump">
                        跳至
                        <input
                            value={jump}
                            inputMode="numeric"
                            placeholder={String(page)}
                            onChange={(event) => setJump(event.target.value.replace(/[^\d]/g, ''))}
                            onKeyDown={submit}
                            aria-label="跳到第几页"
                        />
                        页
                    </label>
                )}
            </div>
        </nav>
    );
}
