import {useEffect, useState} from 'react';
import {createPortal} from 'react-dom';
import {getQuestion} from '../lib/api.js';
import {Rich} from '../lib/Math.jsx';
import {NbImage} from '../lib/Math.jsx';
import {DIFFICULTIES, typeLabel} from '../lib/format.js';
import {Empty, Loading, Modal, Pill} from './ui.jsx';

const DIFF_LABEL = Object.fromEntries(DIFFICULTIES.map((item) => [item.value, item.label]));

/**
 * 只读的题目详情弹窗：选题 / 组卷时想先看完整题目再决定要不要，
 * 点「详情」就地打开这一层，不离开当前页面。
 * 渲染复用题库详情抽屉的链路（Rich → KaTeX）与样式类。
 */
export function QuestionDetailModal({questionId, onClose}) {
    const [detail, setDetail] = useState(null);
    const [error, setError] = useState('');

    useEffect(() => {
        let alive = true;
        setDetail(null);
        setError('');
        getQuestion(questionId)
            .then((data) => {
                if (alive) setDetail(data);
            })
            .catch((cause) => {
                if (alive) setError(cause.message || String(cause));
            });
        return () => {
            alive = false;
        };
    }, [questionId]);

    // portal 到 body：选题窗口本身就是弹窗，详情要盖在它上面，不受其滚动容器影响。
    return createPortal(
        <Modal
            wide
            maskClassName="modal-above"
            title="题目详情"
            subtitle="只读预览；要修改请到「题库」里编辑"
            onClose={onClose}
            footer={
                <button className="btn primary" onClick={onClose}>
                    关闭
                </button>
            }
        >
            {error ? (
                <Empty title="加载失败" hint={error}/>
            ) : !detail ? (
                <Loading text="正在读题…"/>
            ) : (
                <div className="detail-blocks">
                    <div className="detail-meta">
                        <Pill tone="line">{detail.typeLabel || typeLabel(detail.type)}</Pill>
                        <Pill tone="line">{DIFF_LABEL[detail.difficulty] || `${detail.difficulty} 星`}</Pill>
                        {detail.subjectName && <Pill tone="line">{detail.subjectName}</Pill>}
                        {detail.needsReview ? <Pill tone="warn">待复核</Pill> : null}
                        {detail.status === 'DRAFT' ? <Pill tone="warn">草稿</Pill> : null}
                    </div>

                    <section className="detail-block">
                        <h4>题干</h4>
                        <Rich text={detail.stem}/>
                    </section>

                    {detail.options.length > 0 && (
                        <section className="detail-block">
                            <h4>选项</h4>
                            <ul className="option-view">
                                {detail.options.map((option) => (
                                    <li key={option.key}>
                                        <span className="option-key">{option.key}</span>
                                        <Rich text={option.text} className="inline"/>
                                    </li>
                                ))}
                            </ul>
                        </section>
                    )}

                    {detail.assets.length > 0 && (
                        <section className="detail-block">
                            <h4>配图</h4>
                            <div className="asset-row">
                                {detail.assets.map((asset) => (
                                    <div className="asset-thumb" key={asset.id}>
                                        <NbImage id={asset.id} alt={asset.kind}/>
                                        <span className="asset-kind">{asset.kind}</span>
                                    </div>
                                ))}
                            </div>
                        </section>
                    )}

                    <section className="detail-block answer">
                        <h4>答案</h4>
                        <Rich text={detail.answer}/>
                    </section>

                    {detail.analysis && (
                        <section className="detail-block">
                            <h4>解析</h4>
                            <Rich text={detail.analysis}/>
                        </section>
                    )}

                    <section className="detail-block">
                        <h4>知识点</h4>
                        <div className="chip-row">
                            {detail.knowledge.map((point) => (
                                <span key={point.id} className="tag kp" title={point.path}>
                                    {point.name}
                                </span>
                            ))}
                            {detail.knowledge.length === 0 && <span className="muted">未挂知识点</span>}
                        </div>
                    </section>
                </div>
            )}
        </Modal>,
        document.body,
    );
}
