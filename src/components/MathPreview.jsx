import {looksLikeMath} from '../lib/latexMath.js';
import {Rich} from '../lib/Math.jsx';

/**
 * 编辑区里的公式实时预览。
 *
 * 文本框里存的必须是 LaTeX 源码（要能改、能回写），所以不把 textarea 本身变成富文本，
 * 而是在下面跟一块只读预览：改了源码，预览立刻跟着变。这样编辑一道几何/函数题时，
 * 不用靠脑补 `\frac{\sqrt{3}}{3}` 长什么样。
 *
 * 只在真的有公式时出现——普通题干没必要再重复一遍。
 */
const IMAGE_REF_RE = /!\[[^\]]*\]\([^)]+\)/g;

export function MathPreview({text, label = '公式预览', compact = false}) {
    const source = String(text ?? '');
    if (!looksLikeMath(source)) return null;
    // 配图引用在这里只是一个占位，别让预览去解 nbasset（草稿阶段还没有正式附件）。
    const body = source.replace(IMAGE_REF_RE, '［图］');
    return (
        <div className={`math-preview ${compact ? 'compact' : ''}`}>
            <span className="math-preview-tag">{label}</span>
            <div className="math-preview-body">
                <Rich text={body}/>
            </div>
        </div>
    );
}
