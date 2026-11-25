import {useMemo, useState} from 'react';
import {DecodeText} from '../components/effects.jsx';
import quickStart from '../help/quick-start.md?raw';
import account from '../help/account.md?raw';
import library from '../help/library.md?raw';
import papersDoc from '../help/papers.md?raw';
import capture from '../help/capture.md?raw';
import classes from '../help/classes.md?raw';
import insights from '../help/insights.md?raw';
import scoresDoc from '../help/scores.md?raw';
import coursewareDoc from '../help/courseware.md?raw';
import gradingDoc from '../help/grading.md?raw';
import dictationDoc from '../help/dictation.md?raw';
import importDoc from '../help/import.md?raw';
import packFormat from '../help/pack-format.md?raw';
import models from '../help/models.md?raw';
import syncDoc from '../help/sync.md?raw';
import backup from '../help/backup.md?raw';
import faq from '../help/faq.md?raw';

const GROUPS = [
    {key: 'start', label: '入门', desc: '第一次使用先看这里'},
    {key: 'record', label: '录题与题库', desc: '把资料变成能搜到的题'},
    {key: 'teach', label: '教学与评价', desc: '试卷、作业、成绩与课件'},
    {key: 'care', label: '维护与排障', desc: '同步、备份与常见问题'},
];

export const DOCS = [
    {slug: 'quick-start', title: '快速上手', desc: '五分钟录进第一道题', group: 'start', body: quickStart},
    {slug: 'account', title: '本地账号', desc: '登录、改密码、忘记密码', group: 'start', body: account},
    {slug: 'models', title: '模型配置', desc: 'Base URL、Key 与用途路由', group: 'start', body: models},
    {slug: 'library', title: '题库', desc: '搜索、筛选、编辑、回收站', group: 'record', body: library},
    {slug: 'capture', title: '截图识别', desc: '热键截图到可核对的题目', group: 'record', body: capture},
    {slug: 'import', title: '批量导入', desc: '题目包、粘贴与撤销', group: 'record', body: importDoc},
    {slug: 'pack-format', title: '题目包格式', desc: 'JSON 字段与图片约定', group: 'record', body: packFormat},
    {slug: 'papers', title: '试卷', desc: '组卷、配分、排版与打印', group: 'teach', body: papersDoc},
    {slug: 'classes', title: '班级与作业', desc: '学生名单、建班、组卷与批改', group: 'teach', body: classes},
    {slug: 'insights', title: '学情', desc: '掌握度矩阵、趋势与错题本', group: 'teach', body: insights},
    {slug: 'scores', title: '成绩', desc: '考试登分、名次与分数段', group: 'teach', body: scoresDoc},
    {slug: 'courseware', title: '课件', desc: '讲义生成、例题变式与布置', group: 'teach', body: coursewareDoc},
    {slug: 'grading', title: '批改台', desc: '拍照批改、切题与逐题给分', group: 'teach', body: gradingDoc},
    {slug: 'dictation', title: '听写', desc: '拼音定音、语音播报与人工核对', group: 'teach', body: dictationDoc},
    {slug: 'sync', title: '同步', desc: '连接中心题库与冲突处理', group: 'care', body: syncDoc},
    {slug: 'backup', title: '备份与换机', desc: '整库拷贝与恢复', group: 'care', body: backup},
    {slug: 'faq', title: '常见问题', desc: '报错怎么处理', group: 'care', body: faq},
];

const GROUP_OF = Object.fromEntries(GROUPS.map((group) => [group.key, group]));

/** 解码动画只吃纯文本，先把行内标记剥掉，避免把 ** 或反引号也当成字符去抖。 */
function stripInline(text) {
    return String(text).replace(/`([^`]+)`/g, '$1').replace(/\*\*([^*]+)\*\*/g, '$1');
}

function inline(text, keyPrefix) {
    const nodes = [];
    const pattern = /(`[^`]+`|\*\*[^*]+\*\*)/g;
    let last = 0;
    let match;
    let index = 0;
    while ((match = pattern.exec(text)) !== null) {
        if (match.index > last) nodes.push(text.slice(last, match.index));
        const token = match[0];
        if (token.startsWith('`')) {
            nodes.push(<code key={`${keyPrefix}-c${index++}`}>{token.slice(1, -1)}</code>);
        } else {
            nodes.push(<strong key={`${keyPrefix}-b${index++}`}>{token.slice(2, -2)}</strong>);
        }
        last = match.index + token.length;
    }
    if (last < text.length) nodes.push(text.slice(last));
    return nodes;
}

/** 极简 Markdown：标题、列表、代码块、段落。帮助正文都在这套语法里写。 */
function renderMarkdown(source) {
    const lines = String(source).split('\n');
    const blocks = [];
    let list = null;
    let code = null;
    let firstHeading = true;

    const flushList = () => {
        if (!list) return;
        const Tag = list.ordered ? 'ol' : 'ul';
        blocks.push(
            <Tag key={`list-${blocks.length}`} className="help-list">
                {list.items.map((item, index) => (
                    <li key={index}>{inline(item, `l${blocks.length}-${index}`)}</li>
                ))}
            </Tag>,
        );
        list = null;
    };

    for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
        const raw = lines[lineIndex];
        const line = raw.replace(/\s+$/, '');
        if (line.startsWith('|') && line.trim().endsWith('|')) {
            flushList();
            const rows = [];
            while (lineIndex < lines.length) {
                const candidate = lines[lineIndex].trim();
                if (!candidate.startsWith('|') || !candidate.endsWith('|')) break;
                rows.push(candidate);
                lineIndex += 1;
            }
            lineIndex -= 1; // 外层还会 +1，这里回退让下一行继续被处理
            const cells = rows.map((row) => row.slice(1, -1).split('|').map((cell) => cell.trim()));
            const hasDivider = cells.length > 1 && cells[1].every((cell) => /^:?-{3,}:?$/.test(cell));
            const head = cells[0];
            const body = hasDivider ? cells.slice(2) : cells.slice(1);
            blocks.push(
                <div className="help-table-wrap" key={`table-${blocks.length}`}>
                    <table className="help-table">
                        <thead>
                            <tr>
                                {head.map((cell, index) => (
                                    <th key={index}>{inline(cell, `th${blocks.length}-${index}`)}</th>
                                ))}
                            </tr>
                        </thead>
                        <tbody>
                            {body.map((row, rowIndex) => (
                                <tr key={rowIndex}>
                                    {row.map((cell, colIndex) => (
                                        <td key={colIndex}>{inline(cell, `td${blocks.length}-${rowIndex}-${colIndex}`)}</td>
                                    ))}
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>,
            );
            continue;
        }
        if (line.trim().startsWith('```')) {
            if (code === null) {
                flushList();
                code = [];
            } else {
                blocks.push(
                    <pre className="help-code" key={`code-${blocks.length}`}>
                        {code.join('\n')}
                    </pre>,
                );
                code = null;
            }
            continue;
        }
        if (code !== null) {
            code.push(raw);
            continue;
        }
        if (!line.trim()) {
            flushList();
            continue;
        }
        const heading = line.match(/^(#{1,3})\s+(.*)$/);
        if (heading) {
            flushList();
            const level = heading[1].length;
            const content = <DecodeText text={stripInline(heading[2])}/>;
            const key = `h-${blocks.length}`;
            if (level === 1 && firstHeading) {
                firstHeading = false;
                blocks.push(
                    <h2 className="help-doc-title" key={key}>{content}</h2>,
                );
            } else {
                blocks.push(
                    level === 2 ? (
                        <h3 className="help-h2" key={key}>{content}</h3>
                    ) : (
                        <h4 className="help-h3" key={key}>{content}</h4>
                    ),
                );
            }
            continue;
        }
        const ordered = line.match(/^(\d+)\.\s+(.*)$/);
        const bullet = line.match(/^[-*]\s+(.*)$/);
        if (ordered || bullet) {
            const isOrdered = !!ordered;
            if (!list || list.ordered !== isOrdered) {
                flushList();
                list = {ordered: isOrdered, items: []};
            }
            list.items.push(ordered ? ordered[2] : bullet[1]);
            continue;
        }
        if (line.startsWith('> ')) {
            flushList();
            blocks.push(
                <blockquote key={`q-${blocks.length}`}>{inline(line.slice(2), `q${blocks.length}`)}</blockquote>,
            );
            continue;
        }
        flushList();
        blocks.push(<p key={`p-${blocks.length}`}>{inline(line, `p${blocks.length}`)}</p>);
    }
    flushList();
    if (code !== null) {
        blocks.push(
            <pre className="help-code" key="code-last">
                {code.join('\n')}
            </pre>,
        );
    }
    return blocks;
}

function searchSnippet(body, query) {
    const hit = String(body).split('\n').find((line) => line.toLowerCase().includes(query));
    if (!hit) return '';
    return hit.replace(/^[#>\s]+/, '').replace(/[`*]/g, '').slice(0, 96);
}

function HelpHome() {
    return (
        <div className="help-home">
            <section className="help-hero">
                <div className="help-hero-copy">
                    <span className="help-hero-tag">离线文档 · 随安装包提供</span>
                    <h2><DecodeText text="从这里开始"/></h2>
                    <p>所有说明都打包在这台电脑上，断网也能查。选一个主题慢慢看，或用右上角的搜索框直接找关键词。</p>
                </div>
                <div className="help-hero-links">
                    <a className="btn primary" href="#/help/quick-start">快速上手</a>
                    <a className="btn" href="#/help/faq">常见问题</a>
                </div>
            </section>

            {GROUPS.map((group) => (
                <section className="help-group card" key={group.key}>
                    <header className="help-group-head">
                        <h2><DecodeText text={group.label}/></h2>
                        <span>{group.desc}</span>
                    </header>
                    <div className="help-topic-grid">
                        {DOCS.filter((item) => item.group === group.key).map((item) => (
                            <a className="help-topic" href={`#/help/${item.slug}`} key={item.slug}>
                                <span className="help-topic-title">{item.title}</span>
                                <span className="help-topic-desc">{item.desc}</span>
                                <svg viewBox="0 0 24 24" aria-hidden="true">
                                    <path d="M9 6l6 6-6 6"/>
                                </svg>
                            </a>
                        ))}
                    </div>
                </section>
            ))}
        </div>
    );
}

export default function Help({slug}) {
    const [query, setQuery] = useState('');
    const doc = DOCS.find((item) => item.slug === slug) || null;
    const blocks = useMemo(() => (doc ? renderMarkdown(doc.body) : []), [doc]);
    const q = query.trim().toLowerCase();
    const results = useMemo(
        () => (q ? DOCS.filter((item) => `${item.title} ${item.desc} ${item.body}`.toLowerCase().includes(q)) : []),
        [q],
    );
    const index = doc ? DOCS.findIndex((item) => item.slug === doc.slug) : -1;
    const prev = index > 0 ? DOCS[index - 1] : null;
    const next = index >= 0 && index < DOCS.length - 1 ? DOCS[index + 1] : null;

    return (
        <div className="page help-page">
            <header className="page-head">
                <div className="page-head-main">
                    <span className="page-eyebrow">
                        <i aria-hidden="true"/>
                        NeuralKB · Workspace
                    </span>
                    <h1><DecodeText text="帮助"/></h1>
                    <p>内容随安装包离线提供，不依赖外网文档</p>
                </div>
                <div className="page-actions">
                    <label className="help-search">
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <circle cx="11" cy="11" r="6.5"/>
                            <path d="M16 16l4.5 4.5"/>
                        </svg>
                        <input
                            value={query}
                            onChange={(event) => setQuery(event.target.value)}
                            placeholder="搜索帮助内容…"
                            aria-label="搜索帮助内容"
                        />
                        {query && (
                            <button type="button" className="help-search-clear" onClick={() => setQuery('')} title="清除">
                                ×
                            </button>
                        )}
                    </label>
                </div>
            </header>

            <div className="help-layout">
                <nav className="help-nav">
                    <a href="#/help" className={!doc && !q ? 'active' : ''}>
                        <span className="help-nav-title">总览</span>
                    </a>
                    {GROUPS.map((group) => (
                        <div className="help-nav-group" key={group.key}>
                            <div className="help-nav-label">{group.label}</div>
                            {DOCS.filter((item) => item.group === group.key).map((item) => (
                                <a
                                    key={item.slug}
                                    href={`#/help/${item.slug}`}
                                    className={item.slug === doc?.slug ? 'active' : ''}
                                    onClick={() => setQuery('')}
                                >
                                    <span className="help-nav-title">{item.title}</span>
                                </a>
                            ))}
                        </div>
                    ))}
                </nav>

                <div className="help-main">
                    {q ? (
                        <section className="help-results card">
                            <div className="help-results-head">
                                <h2>搜索结果</h2>
                                <span>{results.length} 篇文档匹配「{query.trim()}」</span>
                            </div>
                            {results.length ? (
                                <div className="help-topic-grid">
                                    {results.map((item) => (
                                        <a
                                            className="help-topic"
                                            href={`#/help/${item.slug}`}
                                            key={item.slug}
                                            onClick={() => setQuery('')}
                                        >
                                            <span className="help-topic-group">{GROUP_OF[item.group]?.label}</span>
                                            <span className="help-topic-title">{item.title}</span>
                                            <span className="help-topic-desc">
                                                {searchSnippet(item.body, q) || item.desc}
                                            </span>
                                        </a>
                                    ))}
                                </div>
                            ) : (
                                <p className="help-no-result">没找到相关内容。试试更短的关键词，或者直接翻左侧目录。</p>
                            )}
                        </section>
                    ) : !doc ? (
                        <HelpHome/>
                    ) : (
                        <article className="help-body">
                            <div className="help-crumb">
                                {GROUP_OF[doc.group]?.label}
                                <span>/</span>
                                {doc.title}
                            </div>
                            <p className="help-doc-sub">{doc.desc}</p>
                            {blocks}
                            <div className="help-pager">
                                {prev ? (
                                    <a href={`#/help/${prev.slug}`}>
                                        <small>← 上一篇</small>
                                        <strong><DecodeText text={prev.title}/></strong>
                                    </a>
                                ) : (
                                    <span/>
                                )}
                                {next && (
                                    <a className="next" href={`#/help/${next.slug}`}>
                                        <small>下一篇 →</small>
                                        <strong><DecodeText text={next.title}/></strong>
                                    </a>
                                )}
                            </div>
                            <footer className="help-foot">
                                还有问题？先看「常见问题」；报错原文可以直接复制给开发者。
                            </footer>
                        </article>
                    )}
                </div>
            </div>
        </div>
    );
}
