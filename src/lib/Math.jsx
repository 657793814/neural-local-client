import {memo, useEffect, useMemo, useState} from 'react';
import katex from 'katex';
import {splitLooseMath} from './latexMath.js';
import {latexToPlain} from './latexPlain.js';
import {assetData} from './api.js';

const cache = new Map();

/** 图片附件按需加载：题库里引用的是 `nbasset:<id>`，不暴露文件路径。 */
export function useAsset(id) {
    const [url, setUrl] = useState(() => (id ? cache.get(id) : null));
    useEffect(() => {
        if (!id) return undefined;
        if (cache.has(id)) {
            setUrl(cache.get(id));
            return undefined;
        }
        let alive = true;
        assetData(id)
            .then((data) => {
                cache.set(id, data);
                if (alive) setUrl(data);
            })
            .catch(() => {
                if (alive) setUrl('');
            });
        return () => {
            alive = false;
        };
    }, [id]);
    return url;
}

export const NbImage = memo(function NbImage({id, alt = '', className = ''}) {
    const url = useAsset(id);
    if (url === null) return <span className="nb-image loading" aria-label={alt || '图片加载中'}/>;
    if (!url) return <span className="nb-image missing">图片丢失</span>;
    return <img className={`nb-image ${className}`} src={url} alt={alt}/>;
});

// KaTeX 输出缓存：翻页时同一批公式会被反复渲染，缓存后只剩 DOM 复用。
// throwOnError=true 才能在坏公式（截断的 \sqrt{ 等）上走可读文本兜底，
// 否则 KaTeX 会把一截裸反斜杠涂红摆在用户面前。
const mathCache = new Map();
const MATH_CACHE_LIMIT = 3000;

function renderMath(tex, display) {
    const key = `${display ? 'd' : 'i'}:${tex}`;
    if (mathCache.has(key)) return mathCache.get(key);
    let html = null;
    try {
        html = katex.renderToString(tex, {
            throwOnError: true,
            displayMode: display,
            strict: false,
            trust: false,
            output: 'html',
        });
    } catch {
        html = null;
    }
    if (mathCache.size >= MATH_CACHE_LIMIT) {
        mathCache.delete(mathCache.keys().next().value);
    }
    mathCache.set(key, html);
    return html;
}

/** 一段纯文本（可能含 $...$ 或裸 LaTeX），逐片交给 KaTeX。 */
export const MathText = memo(function MathText({text, className = ''}) {
    const parts = useMemo(() => splitLooseMath(String(text ?? '')), [text]);
    const rendered = useMemo(
        () =>
            parts.map((part, index) => {
                if (part.latex === undefined) return {key: index, text: part.text};
                const html = renderMath(part.latex, part.display);
                return html
                    ? {key: index, html, display: part.display}
                    : {key: index, text: latexToPlain(part.latex)};
            }),
        [parts],
    );
    return (
        <span className={`math-text ${className}`}>
            {rendered.map((part) => {
                if (part.html) {
                    return (
                        <span
                            key={part.key}
                            className={part.display ? 'math-block' : 'math-inline'}
                            dangerouslySetInnerHTML={{__html: part.html}}
                        />
                    );
                }
                return <span key={part.key}>{part.text}</span>;
            })}
        </span>
    );
});

const IMAGE_RE = /!\[([^\]]*)\]\(([^)]+)\)/g;

/**
 * 题目正文：把 `![](nbasset:id)` 换成真正的图片，其余段落走公式渲染。
 * 只认 `nbasset:` 和识别结果里带出来的图形引用；不认任意 URL——本地应用不该去加载外网图片。
 * `figures` 是 {引用名: dataUrl}，来自识别结果里裁出来的图形，未入库前用它预览。
 */
export const Rich = memo(function Rich({text, className = '', figures}) {
    const chunks = useMemo(() => {
        const source = String(text ?? '');
        const out = [];
        let last = 0;
        IMAGE_RE.lastIndex = 0;
        let match;
        while ((match = IMAGE_RE.exec(source)) !== null) {
            const target = match[2].trim();
            let chunk = null;
            if (target.startsWith('nbasset:')) {
                chunk = {type: 'image', id: target.slice(8), alt: match[1]};
            } else if (figures && figures[target]) {
                chunk = {type: 'figure', src: figures[target], alt: match[1]};
            }
            if (!chunk) continue;
            if (match.index > last) out.push({type: 'text', value: source.slice(last, match.index)});
            out.push(chunk);
            last = match.index + match[0].length;
        }
        if (last < source.length) out.push({type: 'text', value: source.slice(last)});
        return out;
    }, [text, figures]);

    return (
        <div className={`rich ${className}`}>
            {chunks.map((chunk, index) => {
                if (chunk.type === 'image') return <NbImage key={index} id={chunk.id} alt={chunk.alt}/>;
                if (chunk.type === 'figure') {
                    return <img key={index} className="nb-image" src={chunk.src} alt={chunk.alt}/>;
                }
                return <MathText key={index} text={chunk.value}/>;
            })}
        </div>
    );
});

/** 不带块级容器的行内版本，用于列表标题、选项。 */
export const RichInline = memo(function RichInline({text}) {
    const source = String(text ?? '');
    if (!source.includes('nbasset:')) return <MathText text={source}/>;
    return <Rich text={source} className="inline"/>;
});
