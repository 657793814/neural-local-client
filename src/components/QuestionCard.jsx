import {memo} from 'react';
import {Rich, NbImage} from '../lib/Math.jsx';
import {Pill} from './ui.jsx';
import {cx, statusLabel, syncLabel, typeLabel} from '../lib/format.js';

/** 题库列表里的题目卡：只读，点击进详情。memo 避免翻页/勾选时重算整页公式。 */
export const QuestionCard = memo(function QuestionCard({question, selected, onSelect, onOpen}) {
    const status = statusLabel(question);
    const sync = syncLabel(question.syncState);
    return (
        <article className={cx('q-card', selected && 'selected')}>
            <label className="q-check" onClick={(event) => event.stopPropagation()}>
                <input
                    type="checkbox"
                    checked={!!selected}
                    onChange={(event) => onSelect?.(question.id, event.target.checked)}
                    aria-label="选择这道题"
                />
            </label>
            <button className="q-main" onClick={() => onOpen(question.id)}>
                <div className="q-meta">
                    <Pill tone="line">{typeLabel(question.type)}</Pill>
                    <Pill tone="line">难度 {question.difficulty}</Pill>
                    {question.subjectName && question.subjectName !== '未分类' && (
                        <span className="q-subject">{question.subjectName}</span>
                    )}
                    <Pill tone={status.tone}>{status.text}</Pill>
                    {sync && <Pill tone={sync.tone}>{sync.text}</Pill>}
                    {question.assets.length > 0 && (
                        <span className="q-badge" title="带图">图 {question.assets.length}</span>
                    )}
                    <span className="q-id" title={`本地题目编号：${question.id}`}>
                        #{question.id.slice(0, 8)}
                    </span>
                </div>
                <div className="q-stem">
                    <Rich text={question.stem} className="clamp-3"/>
                </div>
                {question.issues?.length > 0 && (
                    <div className="q-issues">
                        {question.issues.map((issue) => (
                            <span key={issue} className="q-issue">
                                {issue}
                            </span>
                        ))}
                    </div>
                )}
                <div className="q-foot">
                    <div className="q-tags">
                        {question.knowledge.slice(0, 3).map((point) => (
                            <span key={point.id} className="tag kp" title={point.path}>
                                {point.name}
                            </span>
                        ))}
                    </div>
                    <span className="q-time">{question.updatedAt?.slice(0, 10)}</span>
                </div>
            </button>
            {question.thumb && (
                <button className="q-thumb" onClick={() => onOpen(question.id)} tabIndex={-1}>
                    <NbImage id={question.thumb} alt="题图"/>
                </button>
            )}
        </article>
    );
});
