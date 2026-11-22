/**
 * LaTeX → 纯文本数学写法转换。
 *
 * KaTeX 渲染失败时（模型吐出被截断的 \sqrt{、不认识的 \begin{...} 等）不能把
 * 一串裸反斜杠直接摆给用户。这里把常见 LaTeX 转成可读写法：分式 (a)/(b)、
 * 根号 sqrt()、上标 ^、希腊字母与常用符号转 Unicode，未知命令直接丢弃。
 *
 * 与中心端 front-src/neural/latexPlain.js 保持一致。
 */
const GREEK = {
    alpha: 'α', beta: 'β', gamma: 'γ', Gamma: 'Γ', delta: 'δ', Delta: 'Δ',
    epsilon: 'ε', zeta: 'ζ', eta: 'η', theta: 'θ', Theta: 'Θ', iota: 'ι',
    kappa: 'κ', lambda: 'λ', Lambda: 'Λ', mu: 'μ', nu: 'ν', xi: 'ξ', Xi: 'Ξ',
    pi: 'π', Pi: 'Π', rho: 'ρ', sigma: 'σ', Sigma: 'Σ', tau: 'τ',
    phi: 'φ', Phi: 'Φ', varphi: 'φ', chi: 'χ', psi: 'ψ', Psi: 'Ψ',
    omega: 'ω', Omega: 'Ω',
};

const SYMBOLS = {
    times: '×', div: '÷', cdot: '·', pm: '±', mp: '∓',
    leq: '≤', le: '≤', geq: '≥', ge: '≥', neq: '≠', ne: '≠',
    equiv: '≡', approx: '≈', sim: '∽', simeq: '≃', cong: '≅', propto: '∝',
    infty: '∞', angle: '∠', measuredangle: '∠', triangle: '△', perp: '⊥', parallel: '∥',
    because: '∵', therefore: '∴', in: '∈', notin: '∉', subset: '⊂', subseteq: '⊆',
    supset: '⊃', cup: '∪', cap: '∩', forall: '∀', exists: '∃', emptyset: '∅',
    prime: '′', circ: '°', degree: '°',
    to: '→', rightarrow: '→', Rightarrow: '⇒', leftarrow: '←', Leftarrow: '⇐',
    leftrightarrow: '↔', Leftrightarrow: '⇔', uparrow: '↑', downarrow: '↓',
    ldots: '…', vdots: '⋮', ddots: '⋱', cdots: '⋯', quad: ' ', qquad: '  ',
};

// 内容足够简单时不加括号，让 x^2、1/2 这类结果保持可读。
function isSimple(s) {
    return /^[\w.+\-°′]+$/.test(s);
}

function wrap(s) {
    return isSimple(s) ? s : `(${s})`;
}

export function latexToPlain(input) {
    let text = String(input ?? '');
    if (!text) return '';

    // 1. 去掉 $...$ / \(...\) / \[...\] 定界符
    text = text
        .replace(/\$\$([\s\S]+?)\$\$/g, '$1')
        .replace(/\$([^$\n]+?)\$/g, '$1')
        .replace(/\\\(([\s\S]+?)\\\)/g, '$1')
        .replace(/\\\[([\s\S]+?)\\\]/g, '$1');

    // 2. 结构化命令：内容不含花括号时才能匹配，反复展开以处理嵌套
    const structRules = [
        [/\^\s*\{\s*\\circ\s*\}/g, '°'],
        [/\\(?:d|t)?frac\s*\{([^{}]*)\}\s*\{([^{}]*)\}/g, (_m, a, b) => `${wrap(a)}/${wrap(b)}`],
        [/\\sqrt\s*\[([^\]]+)\]\s*\{([^{}]*)\}/g, (_m, n, b) => `${wrap(b)}^(1/${wrap(n)})`],
        [/\\sqrt\s*\{([^{}]*)\}/g, (_m, b) => `sqrt(${b})`],
        [/\^\s*\{([^{}]*)\}/g, (_m, b) => (isSimple(b) ? `^${b}` : `^(${b})`)],
        [/_\s*\{([^{}]*)\}/g, (_m, b) => (isSimple(b) ? `_${b}` : `_(${b})`)],
        [/\\(?:text|mathrm|mathbf|mathit|mathbb|mathsf|mbox|operatorname|boldsymbol|overline|underline|hat|vec|tilde|widehat|overrightarrow)\s*\{([^{}]*)\}/g, '$1'],
    ];
    for (let round = 0; round < 8; round++) {
        const before = text;
        for (const [re, rep] of structRules) text = text.replace(re, rep);
        if (text === before) break;
    }

    // 3. 符号与希腊字母。长名字先替换 + 负向前瞻，避免 \le 吃掉 \left 的前缀。
    const names = [...Object.keys(SYMBOLS), ...Object.keys(GREEK)].sort((a, b) => b.length - a.length);
    for (const name of names) {
        const rep = SYMBOLS[name] ?? GREEK[name];
        text = text.replace(new RegExp('\\\\' + name + '(?![a-zA-Z])', 'g'), rep);
    }

    // 4. 清理残余：换行命令、手动间距、转义字符、未知命令
    text = text
        .replace(/\\\\(\s*\n)?/g, ' ')
        .replace(/\\left|\\right/g, '')
        .replace(/\\[,;:!]/g, ' ')
        .replace(/\\ /g, ' ')
        .replace(/\\%/g, '%')
        .replace(/\\&/g, '&')
        .replace(/\\\$/g, '$')
        .replace(/\\\{/g, '{')
        .replace(/\\\}/g, '}')
        .replace(/\\_/g, '_')
        .replace(/\\#/g, '#')
        .replace(/\\[a-zA-Z]+/g, '')
        .replace(/[ \t]{2,}/g, ' ');

    // 模型输出被截断时（例如解析停在 \sqrt{ 一半）会留下没闭合的花括号，
    // 去掉这个孤零零的尾巴比把 "={" 摆给用户看更可读。
    text = text.replace(/\{(?=[^{}]*$)/g, '');

    return text.trim();
}
