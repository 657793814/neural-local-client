import {useEffect, useLayoutEffect, useRef, useState} from 'react';
import {cx} from '../lib/format.js';

/** 跟随系统的「减少动态效果」偏好，组件级动画据此降级为直接呈现。 */
export function usePrefersReducedMotion() {
    const [reduced, setReduced] = useState(
        () => typeof window !== 'undefined'
            && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches,
    );
    useEffect(() => {
        const query = window.matchMedia?.('(prefers-reduced-motion: reduce)');
        if (!query) return;
        const onChange = () => setReduced(query.matches);
        query.addEventListener?.('change', onChange);
        return () => query.removeEventListener?.('change', onChange);
    }, []);
    return reduced;
}

const easeOutExpo = (t) => (t >= 1 ? 1 : 1 - 2 ** (-10 * t));

/**
 * 数字递增：保留原字符串里的前缀 / 后缀，只让中间的数字滚动。
 * 例如 "50%"、"3/5"、"128 道" 都能直接传，更新时从当前值接着滚到新值。
 */
export function CountUp({value, duration = 1000, className}) {
    const reduced = usePrefersReducedMotion();
    const raw = String(value ?? '');
    const match = raw.match(/-?\d+(?:\.\d+)?/);
    const numeric = match ? Number(match[0]) : 0;
    const prefix = match ? raw.slice(0, match.index) : raw;
    const suffix = match ? raw.slice(match.index + match[0].length) : '';
    const decimals = match?.[0].includes('.') ? match[0].split('.')[1].length : 0;
    const [display, setDisplay] = useState(reduced ? numeric : 0);
    const fromRef = useRef(0);

    useEffect(() => {
        if (!match) return;
        if (reduced) {
            fromRef.current = numeric;
            setDisplay(numeric);
            return;
        }
        const from = fromRef.current;
        const delta = numeric - from;
        const begin = performance.now();
        let raf = 0;
        const tick = (now) => {
            const t = Math.min(1, (now - begin) / duration);
            setDisplay(from + delta * easeOutExpo(t));
            if (t < 1) raf = requestAnimationFrame(tick);
            else fromRef.current = numeric;
        };
        raf = requestAnimationFrame(tick);
        return () => cancelAnimationFrame(raf);
        // match 每次渲染都是新数组，用它的存在性与数值本身做依赖。
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [numeric, duration, reduced, Boolean(match)]);

    if (!match) return <span className={className}>{raw}</span>;

    const text = display.toLocaleString('zh-CN', {
        minimumFractionDigits: decimals,
        maximumFractionDigits: decimals,
    });
    return (
        <span className={cx('count-up', className)} aria-label={raw}>
            {prefix}{text}{suffix}
        </span>
    );
}

const DECODE_CHARS = '※◈◇◌⌗⍟⎔⌘⍜◍';

/**
 * 文字解码：标题进入时先显示随机符号，再逐字落定。
 * 时长刻意压得比较短，只在切页时出现一次，不抢内容注意力。
 */
export function DecodeText({text, duration = 620}) {
    const reduced = usePrefersReducedMotion();
    const [display, setDisplay] = useState(text);

    useEffect(() => {
        if (reduced) {
            setDisplay(text);
            return;
        }
        const glyphs = Array.from(text);
        const begin = performance.now();
        let raf = 0;
        const tick = (now) => {
            const t = Math.min(1, (now - begin) / duration);
            const resolved = Math.floor(t * glyphs.length);
            setDisplay(
                glyphs
                    .map((char, index) => {
                        if (index < resolved || char === ' ') return char;
                        return DECODE_CHARS[Math.floor(Math.random() * DECODE_CHARS.length)];
                    })
                    .join(''),
            );
            if (t < 1) raf = requestAnimationFrame(tick);
            else setDisplay(text);
        };
        raf = requestAnimationFrame(tick);
        return () => cancelAnimationFrame(raf);
    }, [text, duration, reduced]);

    return (
        <span className="decode-text" aria-label={text}>
            {display}
        </span>
    );
}

/**
 * 打字机：逐字显示，保留一份透明原文占位，避免文字一行行长出来把后面的内容顶下去。
 * 打完后留一根闪烁光标，减少动态效果偏好下直接整段呈现。
 */
export function Typewriter({text, speed = 26, startDelay = 180}) {
    const reduced = usePrefersReducedMotion();
    const [shown, setShown] = useState(reduced ? text : '');

    useEffect(() => {
        if (reduced) {
            setShown(text);
            return undefined;
        }
        const chars = Array.from(text);
        setShown('');
        let index = 0;
        let tick = 0;
        const begin = setTimeout(() => {
            tick = setInterval(() => {
                index += 1;
                setShown(chars.slice(0, index).join(''));
                if (index >= chars.length) clearInterval(tick);
            }, speed);
        }, startDelay);
        return () => {
            clearTimeout(begin);
            clearInterval(tick);
        };
    }, [text, speed, startDelay, reduced]);

    return (
        <span className="typewriter" aria-label={text}>
            <span className="typewriter-ghost" aria-hidden="true">{text}</span>
            <span className="typewriter-live" aria-hidden="true">
                {shown}<i className="typewriter-caret"/>
            </span>
        </span>
    );
}

/**
 * 分段控件的液态滑块：量出 aria-selected 那一项的位置，交给一个绝对定位的
 * 玻璃指示器滑过去。切换时是位移而不是重新淡入，视觉上更像同一块玻璃在移动。
 *
 * 返回值：[容器 ref, 滑块样式]。还没量到（首帧、被隐藏）时样式为 null，
 * 调用方据此不渲染滑块，避免左上角闪一下再跳过去。
 */
export function useSlidingThumb(activeKey) {
    const ref = useRef(null);
    const [thumb, setThumb] = useState(null);

    useLayoutEffect(() => {
        const box = ref.current;
        if (!box) return undefined;

        const measure = () => {
            const active = box.querySelector('[aria-selected="true"]');
            if (!active) {
                setThumb(null);
                return;
            }
            setThumb({
                x: active.offsetLeft,
                y: active.offsetTop,
                width: active.offsetWidth,
                height: active.offsetHeight,
            });
        };

        measure();
        const observer = new ResizeObserver(measure);
        observer.observe(box);
        // 字体加载完、窗口缩放后按钮宽度会变，跟着重量一次。
        window.addEventListener('resize', measure);
        return () => {
            observer.disconnect();
            window.removeEventListener('resize', measure);
        };
    }, [activeKey]);

    return [ref, thumb];
}
