/**
 * 把混合文本切成「文字 / 公式」片段。
 *
 * OCR 出来的题目可能带 $...$、\(...\) 定界符，也可能是裸的 \frac{1}{2}、x^{2}。
 * 这里把两类都识别出来，交给 MathText 用 KaTeX 渲染。
 */
const EXPLICIT_RE = /\$\$([\s\S]+?)\$\$|\$([^$\n]+?)\$|\\\(([\s\S]+?)\\\)|\\\[([\s\S]+?)\\\]/g;

// 数学候选片段：ASCII 字母数字 + 反斜杠/花括号/运算符/括号 + 空格制表符。
// 中文、全角标点、换行都不在集合里，天然成为片段边界，避免跨行把整段吃成一个公式。
const RUN_RE = /[A-Za-z0-9\\{}^_+\-*/=<>().,[\]| \t]+/g;

// 至少含一个 LaTeX / 幂次记号才当公式，否则像 AB=5 这种纯 ASCII 保持文字。
const MARK_RE = /\\[A-Za-z]+|\^\s*[{(]|_\s*[{(]|\^[0-9A-Za-z]|_[0-9A-Za-z]/;

function pushPlain(parts, value) {
    if (!value) return;
    const last = parts[parts.length - 1];
    if (last && !last.math) last.value += value;
    else parts.push({ math: false, value });
}

function pushMath(parts, raw) {
    // 保留首尾空白，避免公式和汉字挤在一起。
    const lead = raw.match(/^[ \t]*/)[0];
    const trail = raw.match(/[ \t]*$/)[0];
    const tex = raw.trim();
    pushPlain(parts, lead);
    // % 在 TeX 里是注释符，含百分号的片段按文字处理更安全。
    if (tex && !tex.includes('%')) parts.push({ math: true, value: tex });
    else pushPlain(parts, tex);
    pushPlain(parts, trail);
}

function splitBare(parts, chunk) {
    RUN_RE.lastIndex = 0;
    let last = 0;
    let match;
    while ((match = RUN_RE.exec(chunk)) !== null) {
        const run = match[0];
        if (!MARK_RE.test(run)) continue;
        pushPlain(parts, chunk.slice(last, match.index));
        pushMath(parts, run);
        last = match.index + run.length;
    }
    pushPlain(parts, chunk.slice(last));
}

export function splitMath(text) {
    const source = String(text || '');
    const parts = [];
    let last = 0;
    let match;
    EXPLICIT_RE.lastIndex = 0;
    while ((match = EXPLICIT_RE.exec(source)) !== null) {
        splitBare(parts, source.slice(last, match.index));
        const tex = (match[1] ?? match[2] ?? match[3] ?? match[4] ?? '').trim();
        if (tex) parts.push({ math: true, value: tex });
        last = match.index + match[0].length;
    }
    splitBare(parts, source.slice(last));
    return parts;
}

/**
 * 粗判：这段文字里有没有需要渲染的公式。
 *
 * 编辑区的实时预览用它决定要不要露头——普通题干没必要在下面再重复一遍，
 * 只有真的写了 `$...$` / `\(...\)` / `\frac{1}{2}` / `x^{2}` 才显示。
 * 宁可漏判（多显示一块预览不碍事），不要误判到普通中文上。
 */
export function looksLikeMath(text) {
    const source = String(text || '');
    if (!source) return false;
    if (/\$[^$\n]+\$/.test(source)) return true;
    if (/\\\(|\\\[/.test(source)) return true;
    return /\\[A-Za-z]{2,}|\^\{|_\{/.test(source);
}

/**
 * 只切「显式定界符」公式：$...$ / $$...$$ / \(...\) / \[...\]。
 *
 * 与 splitMath 不同，这里不把裸的 \frac{1}{2} 之类 ASCII 片段当公式——HTML 正文里
 * 普通文字很多，误判会破坏排版。HTML 格式的题目不经过 markdown.js，KaTeX 占位
 * 元素需要由 MathHtml 依据本函数的结果补建。
 *
 * 返回 [{text}] 或 [{latex, display}]，是纯函数，可独立验证。
 */
export function splitMathDelimiters(text) {
    const source = String(text || '');
    const parts = [];
    let last = 0;
    let match;
    EXPLICIT_RE.lastIndex = 0;
    while ((match = EXPLICIT_RE.exec(source)) !== null) {
        if (match.index > last) parts.push({ text: source.slice(last, match.index) });
        const latex = (match[1] ?? match[2] ?? match[3] ?? match[4] ?? '').trim();
        // $$...$$ 与 \[...\] 是块级公式，其余是行内公式。
        const display = match[1] !== undefined || match[4] !== undefined;
        parts.push(latex ? { latex, display } : { text: match[0] });
        last = match.index + match[0].length;
    }
    if (last < source.length) parts.push({ text: source.slice(last) });
    return parts;
}

// ---------------------------------------------------------------------------
// 裸 LaTeX：没有 $ / \( / \[ 包裹的情况
//
// OCR 和视觉模型经常直接输出 \sqrt{2}a、(sinB - sinC)^2，题干里就露出一串反斜杠。
// 只在「认得的命令」或上下标记号出现时才当公式，而且命令名必须落在白名单里——
// 否则 C:\Users、\_private 这类普通文本会被当成公式画成红色报错。
// ---------------------------------------------------------------------------

const LATEX_COMMANDS = new Set([
    // 结构
    'frac', 'dfrac', 'tfrac', 'sqrt', 'overline', 'underline', 'hat', 'widehat', 'vec',
    'tilde', 'bar', 'overrightarrow', 'text', 'mathrm', 'mathbf', 'mathit', 'mathbb',
    'mathsf', 'mbox', 'operatorname', 'boldsymbol', 'left', 'right', 'limits', 'displaystyle',
    'begin', 'end', 'quad', 'qquad', 'prime', 'circ', 'degree', 'cdots', 'ldots', 'vdots',
    'ddots', 'dots',
    // 函数
    'sin', 'cos', 'tan', 'cot', 'sec', 'csc', 'arcsin', 'arccos', 'arctan', 'sinh', 'cosh',
    'tanh', 'log', 'ln', 'lg', 'exp', 'lim', 'max', 'min', 'sup', 'inf', 'det', 'gcd', 'mod',
    'bmod', 'pmod', 'sum', 'prod', 'int', 'iint', 'oint',
    // 运算符与关系
    'cdot', 'times', 'div', 'pm', 'mp', 'leq', 'le', 'geq', 'ge', 'neq', 'ne', 'equiv',
    'approx', 'sim', 'simeq', 'cong', 'propto', 'leqslant', 'geqslant', 'to', 'rightarrow',
    'Rightarrow', 'leftarrow', 'Leftarrow', 'leftrightarrow', 'Leftrightarrow', 'uparrow',
    'downarrow', 'in', 'notin', 'ni', 'subset', 'subseteq', 'supset', 'supseteq', 'cup',
    'cap', 'setminus', 'emptyset', 'varnothing', 'forall', 'exists', 'infty', 'angle',
    'measuredangle', 'triangle', 'perp', 'parallel', 'because', 'therefore', 'langle',
    'rangle', 'lfloor', 'rfloor', 'lceil', 'rceil', 'mid', 'nmid', 'ast', 'star', 'odot',
    'oplus', 'otimes', 'bigcup', 'bigcap', 'choose', 'binom', 'partial', 'nabla', 'ell',
    // 希腊字母
    'alpha', 'beta', 'gamma', 'Gamma', 'delta', 'Delta', 'epsilon', 'varepsilon', 'zeta',
    'eta', 'theta', 'Theta', 'iota', 'kappa', 'lambda', 'Lambda', 'mu', 'nu', 'xi', 'Xi',
    'pi', 'Pi', 'rho', 'sigma', 'Sigma', 'tau', 'upsilon', 'phi', 'Phi', 'varphi', 'chi',
    'psi', 'Psi', 'omega', 'Omega',
]);

// sin / cos / tan 这类函数名后面直接跟变量（sinB、cosC）仍然算数学，不算英文单词。
const FUNCTION_WORD_RE = /^(?:arcsin|arccos|arctan|sinh|cosh|tanh|sin|cos|tan|cot|sec|csc|log|ln|lg|exp|lim|max|min|sup|inf|det|gcd|deg|rad)[A-Za-z0-9]*$/;

const COMMAND_RE = /\\([A-Za-z]+)/g;
// 英文单词按「3 个字母以上」或「首字母大写的两字母词（If / In / As）」认，
// 这样 bc、ab 这类两个小写字母的乘积不会被当成单词。
const WORD_RE = /[A-Za-z]{3,}|[A-Z][a-z]/g;
// 公式 token 只允许这些字符：字母数字、反斜杠、花括号、上下标与常见运算符。
const MATH_TOKEN_RE = /^[A-Za-z0-9\\{}^_+\-*/=<>().,[\]|'!?]+$/;

function hasKnownCommand(text) {
    COMMAND_RE.lastIndex = 0;
    let match;
    while ((match = COMMAND_RE.exec(text)) !== null) {
        if (LATEX_COMMANDS.has(match[1])) return true;
    }
    return false;
}

/** 普通英文单词（find、If、Given…）出现时，公式片段必须在它前面断开。 */
function hasForeignWord(text) {
    // 先去掉 \sqrt、\frac 这类命令，否则命令名本身会被当成英文单词。
    const stripped = String(text || '').replace(/\\[A-Za-z]+/g, ' ');
    WORD_RE.lastIndex = 0;
    let match;
    while ((match = WORD_RE.exec(stripped)) !== null) {
        const word = match[0];
        if (/^[A-Z]+$/.test(word)) continue;      // AB、MN 这种图形顶点标记
        if (FUNCTION_WORD_RE.test(word)) continue; // sinB、cosC
        return true;
    }
    return false;
}

/**
 * 只有 ^ / _ 而没有 LaTeX 命令时，才需要更谨慎：URL 里的下划线不是下标。
 * 只排除链接（https://、//x.com），不排除除号——a (b^2+c^2)/(2bc) 这种式子很常见。
 */
function hasScriptMark(text) {
    return /[\^_]\s*[{0-9A-Za-z]/.test(text) && !/https?:\/\/|\/\//i.test(text);
}

function isMathToken(token) {
    if (!token) return false;
    if (!MATH_TOKEN_RE.test(token)) return false;
    // 认得的命令一定是数学：\text{find} 里的英文是公式内容，不是句子。
    if (hasKnownCommand(token)) return true;
    return !hasForeignWord(token);
}

function pushText(parts, value) {
    if (!value) return;
    const last = parts[parts.length - 1];
    if (last && last.text !== undefined) last.text += value;
    else parts.push({ text: value });
}

/**
 * 切一段裸 ASCII 片段。有普通英文单词时按 token 分组，避免把整句英文画成斜体乘积；
 * 全是数学 token 时整段交给 KaTeX（更接近原题的排版）。
 */
function pushBareRun(parts, run) {
    // 「…sinBsinC. (I)求A」这种把小题号连在句末的情况，先按句号断开，
    // 否则 (I) 会被并进公式里跟着斜体。
    const segments = run.split(/(?<=\.)\s+/);
    for (let index = 0; index < segments.length; index++) {
        if (index > 0) pushText(parts, ' ');
        pushBareSegment(parts, segments[index]);
    }
}

function pushBareSegment(parts, run) {
    if (!run) return;
    // 小题号「(II)」不是公式的一部分：(II) sinC = ... 里只有后半段该斜体。
    const label = run.match(/^\((?:\d{1,2}|[IVX]{1,4})\)/);
    if (label && label[0].length < run.length) {
        pushText(parts, label[0]);
        pushBareSegment(parts, run.slice(label[0].length));
        return;
    }
    if (!hasKnownCommand(run) && !hasScriptMark(run)) {
        pushText(parts, run);
        return;
    }
    if (!hasForeignWord(run)) {
        const tex = run.trim();
        // % 在 TeX 里是注释符，含百分号的片段按文字处理更安全。
        if (tex && !tex.includes('%')) parts.push({ latex: tex, display: false, bare: true });
        else pushText(parts, run);
        return;
    }
    const tokens = run.split(/(\s+)/);
    let buffer = '';
    let strong = false;
    const flush = () => {
        if (!buffer) return;
        const tex = buffer.trim();
        if (tex && strong && !tex.includes('%')) parts.push({ latex: tex, display: false, bare: true });
        else pushText(parts, buffer);
        buffer = '';
        strong = false;
    };
    for (const token of tokens) {
        if (!token) continue;
        if (/^\s+$/.test(token)) {
            if (buffer) buffer += token;
            else pushText(parts, token);
            continue;
        }
        if (isMathToken(token)) {
            buffer += token;
            if (hasKnownCommand(token) || hasScriptMark(token)) strong = true;
            continue;
        }
        flush();
        pushText(parts, token);
    }
    flush();
}

function pushBareText(parts, chunk) {
    RUN_RE.lastIndex = 0;
    let last = 0;
    let match;
    while ((match = RUN_RE.exec(chunk)) !== null) {
        pushText(parts, chunk.slice(last, match.index));
        pushBareRun(parts, match[0]);
        last = match.index + match[0].length;
    }
    pushText(parts, chunk.slice(last));
}

/**
 * 显式定界符 + 裸 LaTeX 一起切。
 *
 * 用于「展示」链路（题目富文本、列表摘要、tooltip、行内短文本）：既认 $...$，
 * 也认模型直出的 \sqrt{2}。返回值形状与 splitMathDelimiters 一致，可直接替换。
 */
export function splitLooseMath(text) {
    let source = String(text || '');
    // 模型输出被截断时可能留下落单的 $（有开头没结尾），它既配不成定界符、
    // 也不是正文，留在页面上就是一个孤零零的美元符号。只在它后面跟着没闭合的
    // LaTeX 片段时才当截断处理，正常货币符号（$5）不动。
    if ((source.match(/\$/g) || []).length % 2 === 1) {
        const at = source.lastIndexOf('$');
        const tail = source.slice(at + 1);
        const opens = (tail.match(/\{/g) || []).length;
        const closes = (tail.match(/\}/g) || []).length;
        if (opens > closes || /\\[A-Za-z]+/.test(tail)) {
            source = source.slice(0, at) + source.slice(at + 1);
        }
    }
    const parts = [];
    for (const part of splitMathDelimiters(source)) {
        if (part.latex) parts.push(part);
        else pushBareText(parts, part.text);
    }
    return parts;
}
