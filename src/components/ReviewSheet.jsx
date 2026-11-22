import {useCallback, useEffect, useState} from 'react';
import {reviewSheet} from '../lib/api.js';
import {Rich} from '../lib/Math.jsx';
import {buildReviewCsv, buildReviewHtml} from '../lib/review.js';
import {openInBrowser} from '../lib/paper/print.js';
import {saveTextFile} from '../lib/format.js';
import {Drawer, Empty, Pill, notify} from './ui.jsx';

function num(value) {
    const number = Number(value || 0);
    return Number.isInteger(number) ? String(number) : number.toFixed(1);
}

function rateText(rate) {
    if (rate == null) return '未批';
    return `${Math.round(rate * 100)}%`;
}

function rateTone(rate) {
    if (rate == null) return 'muted';
    if (rate < 0.5) return 'danger';
    if (rate < 0.8) return 'warn';
    return 'ok';
}

/**
 * 讲评清单：批完一份作业后，按题看「几个人错、错在哪、谁错了」。
 *
 * 这个页面是「批改备注」的第一个真实读者——以前备注写完就没人再看。
 * 排序用后端的 rank（错得多优先），这里不再自己算，免得打印稿和界面两种说法。
 */
export default function ReviewSheetView({assignmentId, title, onClose}) {
    const [data, setData] = useState(null);
    const [error, setError] = useState('');
    const [busy, setBusy] = useState(false);
    const [onlyWrong, setOnlyWrong] = useState(false);

    const load = useCallback(async () => {
        try {
            setData(await reviewSheet(assignmentId));
        } catch (issue) {
            setError(issue.message || String(issue));
        }
    }, [assignmentId]);

    useEffect(() => {
        load();
    }, [load]);

    const print = async () => {
        if (!data) return;
        setBusy(true);
        try {
            await openInBrowser(buildReviewHtml(data), `${data.className}-${data.title}-讲评清单`);
            notify.success('已在浏览器打开，⌘P 打印或另存为 PDF');
        } catch (issue) {
            notify.error(issue.message || String(issue));
        } finally {
            setBusy(false);
        }
    };

    const exportCsv = async () => {
        if (!data) return;
        try {
            await saveTextFile(`${data.title}-讲评清单.csv`, buildReviewCsv(data), [
                {name: '表格', extensions: ['csv']},
            ]);
            notify.success('已导出 CSV');
        } catch (issue) {
            notify.error(issue.message || String(issue));
        }
    };

    const questions = data?.questions || [];
    const shown = onlyWrong ? questions.filter((row) => row.wrong > 0) : questions;
    const missing = (data?.students || []).filter((row) => !row.submitted);
    const pendingStudents = (data?.students || []).filter((row) => row.submitted && row.pending > 0);
    const mustTeach = questions.filter((row) => row.wrong > 0).sort((left, right) => left.rank - right.rank).slice(0, 3);
    const gradedPct =
        data && data.expectedItems > 0 ? Math.round((data.gradedItems / data.expectedItems) * 100) : 0;

    return (
        <Drawer
            title={data ? `${data.title} · 讲评清单` : title ? `${title} · 讲评清单` : '讲评清单'}
            subtitle={data ? `${data.className} · ${data.studentCount} 人 · ${questions.length} 题` : '正在汇总批改结果…'}
            onClose={onClose}
            footer={
                <>
                    <button className="btn ghost" onClick={exportCsv} disabled={!data || busy}>
                        导出 CSV
                    </button>
                    <button className="btn primary" onClick={print} disabled={!data || busy || !questions.length}>
                        {busy ? '正在生成…' : '打印 / 存 PDF'}
                    </button>
                </>
            }
        >
            {error && <Empty title="读不到讲评清单" hint={error}/>}

            {data && (
                <>
                    <div className="rv-stats">
                        <div>
                            <strong>
                                {data.submittedCount}/{data.studentCount}
                            </strong>
                            <span>已交</span>
                        </div>
                        <div>
                            <strong>{num(data.avgScore)}</strong>
                            <span>均分（满分 {num(data.maxScore)}）</span>
                        </div>
                        <div>
                            <strong>{gradedPct}%</strong>
                            <span>
                                批改完成度 {data.gradedItems}/{data.expectedItems}
                            </span>
                        </div>
                    </div>

                    {!questions.length ? (
                        <Empty title="这份作业还没有题目" hint="先去「布置作业」里选题发布。"/>
                    ) : (
                        <>
                            {mustTeach.length > 0 && (
                                <section className="rv-block">
                                    <header>
                                        <strong>先讲这三道</strong>
                                        <span className="muted">错得最多的排前面，同错数看不准得分率</span>
                                    </header>
                                    <div className="rv-top">
                                        {mustTeach.map((row) => (
                                            <article key={row.questionId} className="rv-top-card">
                                                <div className="rv-top-head">
                                                    <span className="row-no">{row.ord + 1}</span>
                                                    <Pill tone="line">{row.typeLabel}</Pill>
                                                    <Pill tone={rateTone(row.rate)}>得分率 {rateText(row.rate)}</Pill>
                                                    <span className="muted">
                                                        错 {row.wrong} 人
                                                        {row.partial ? `（${row.partial} 人部分分）` : ''}
                                                    </span>
                                                </div>
                                                <Rich text={row.stem} className="rv-stem clamp-3"/>
                                                {row.wrongStudents.length > 0 && (
                                                    <p className="rv-names">
                                                        <span className="muted">错的人：</span>
                                                        {row.wrongStudents.join('、')}
                                                    </p>
                                                )}
                                            </article>
                                        ))}
                                    </div>
                                </section>
                            )}

                            <section className="rv-block">
                                <header>
                                    <strong>逐题记录</strong>
                                    <label className="rv-filter">
                                        <input
                                            type="checkbox"
                                            checked={onlyWrong}
                                            onChange={(event) => setOnlyWrong(event.target.checked)}
                                        />
                                        只看有人错的
                                    </label>
                                </header>
                                <div className="rv-list">
                                    {shown.map((row) => (
                                        <article key={row.questionId} className="rv-item">
                                            <div className="rv-item-head">
                                                <span className="row-no">{row.ord + 1}</span>
                                                <Pill tone="line">{row.typeLabel}</Pill>
                                                <span className="muted">难度 {row.difficulty}</span>
                                                <span className="muted">满分 {num(row.score)}</span>
                                                <Pill tone={rateTone(row.rate)}>
                                                    {row.wrong} 错 · {rateText(row.rate)}
                                                </Pill>
                                                {row.pending > 0 && <Pill tone="warn">待批 {row.pending}</Pill>}
                                                {row.blank > 0 && <Pill tone="muted">空答 {row.blank}</Pill>}
                                            </div>
                                            <Rich text={row.stem} className="rv-stem clamp-3"/>
                                            {row.answer && (
                                                <p className="rv-answer">
                                                    <span className="muted">参考答案：</span>
                                                    <Rich text={row.answer} className="inline"/>
                                                </p>
                                            )}
                                            {row.wrongStudents.length > 0 && (
                                                <p className="rv-names">
                                                    <span className="muted">错的人：</span>
                                                    {row.wrongStudents.join('、')}
                                                </p>
                                            )}
                                            {row.remarks.length > 0 && (
                                                <ul className="rv-remarks">
                                                    {row.remarks.map((item) => (
                                                        <li key={`${item.studentId}-${item.remark}`}>
                                                            <strong>{item.studentName}</strong>
                                                            <span className="muted">
                                                                （{num(item.score)}/{num(item.maxScore)}）
                                                            </span>
                                                            {item.remark}
                                                        </li>
                                                    ))}
                                                </ul>
                                            )}
                                            {row.knowledge.length > 0 && (
                                                <p className="rv-knowledge muted">{row.knowledge.join(' · ')}</p>
                                            )}
                                        </article>
                                    ))}
                                    {!shown.length && <p className="muted rv-empty">都批完了，没有错的题。</p>}
                                </div>
                            </section>

                            {(missing.length > 0 || pendingStudents.length > 0) && (
                                <section className="rv-block">
                                    <header>
                                        <strong>还没到齐</strong>
                                    </header>
                                    {missing.length > 0 && (
                                        <p className="rv-names">
                                            <span className="muted">未交（{missing.length} 人）：</span>
                                            {missing.map((row) => row.name).join('、')}
                                        </p>
                                    )}
                                    {pendingStudents.length > 0 && (
                                        <p className="rv-names">
                                            <span className="muted">待批（{pendingStudents.length} 人）：</span>
                                            {pendingStudents.map((row) => row.name).join('、')}
                                        </p>
                                    )}
                                </section>
                            )}
                        </>
                    )}
                </>
            )}
        </Drawer>
    );
}
