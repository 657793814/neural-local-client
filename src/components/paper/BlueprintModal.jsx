import {useEffect, useMemo, useState} from 'react';
import {paperBlueprintFill, paperBlueprintSuggest} from '../../lib/api.js';
import {scoreText} from '../../lib/paper/score.js';
import {Modal, Pill, notify} from '../ui.jsx';

const KINDS = [
    {value: 'SINGLE', label: '单项选择题'},
    {value: 'MULTI', label: '多项选择题'},
    {value: 'FILL', label: '填空题'},
    {value: 'CALC', label: '解答题'},
];

const BANDS = [
    {key: 'easy', label: '易', hint: '难度 1-2'},
    {key: 'medium', label: '中', hint: '难度 3'},
    {key: 'hard', label: '难', hint: '难度 4-5'},
];

const emptyRow = () => ({
    kind: 'SINGLE',
    title: '',
    count: 0,
    unitScore: 5,
    bands: {easy: 0, medium: 0, hard: 0},
});

const rowTotal = (row) => (Number(row.count) || 0) * (Number(row.unitScore) || 0);

function knowledgeOptions(tree, subjectId) {
    const out = [];
    for (const subject of tree || []) {
        if (subjectId && subject.id !== subjectId) continue;
        for (const textbook of subject.textbooks || []) {
            for (const chapter of textbook.chapters || []) {
                for (const point of chapter.points || []) {
                    out.push({
                        id: point.id,
                        label: `${chapter.name} · ${point.name}`,
                    });
                }
            }
        }
    }
    return out;
}

/**
 * 蓝图组卷（双向细目表）：题数 + 每题分值 + 易中难配额 + 知识点配额。
 * 先算分再抽题——Σ(题数×分值) 必须等于卷面总分，否则后端直接拒绝，不抽也不落库。
 * 缺题时后端整次回滚，这里把缺口逐条列出来，可以一键放宽难度重试。
 */
export function BlueprintModal({paperId, paper, tree = [], onClose, onFilled}) {
    const [rows, setRows] = useState([]);
    const [quota, setQuota] = useState([]);
    const [replace, setReplace] = useState((paper?.questionCount || 0) > 0);
    const [loading, setLoading] = useState(true);
    const [busy, setBusy] = useState(false);
    const [report, setReport] = useState(null);

    const options = useMemo(() => knowledgeOptions(tree, paper?.subjectId), [tree, paper?.subjectId]);
    const total = Number(paper?.totalScore) || 0;
    const planned = rows.reduce((sum, row) => sum + rowTotal(row), 0);
    const delta = Math.round((total - planned) * 1000) / 1000;
    const bandsConsistent = rows.every((row) => {
        const sum = BANDS.reduce((acc, band) => acc + (Number(row.bands[band.key]) || 0), 0);
        return sum === 0 || sum === Number(row.count);
    });

    useEffect(() => {
        let alive = true;
        (async () => {
            try {
                const blueprint = await paperBlueprintSuggest(paperId);
                if (!alive) return;
                const list = (blueprint.sections || []).map((section) => ({
                    kind: section.kind || 'CALC',
                    title: section.title || '',
                    count: Number(section.count) || 0,
                    unitScore: Number(section.unitScore) || 5,
                    bands: {
                        easy: Number(section.bands?.easy) || 0,
                        medium: Number(section.bands?.medium) || 0,
                        hard: Number(section.bands?.hard) || 0,
                    },
                }));
                setRows(list.length ? list : [emptyRow()]);
            } catch (error) {
                notify.error(error.message);
            } finally {
                if (alive) setLoading(false);
            }
        })();
        return () => {
            alive = false;
        };
    }, [paperId]);

    const patchRow = (index, patch) =>
        setRows((current) =>
            current.map((row, at) => (at === index ? {...row, ...patch, bands: {...row.bands, ...(patch.bands || {})}} : row)),
        );

    const submit = async (relax = false) => {
        setBusy(true);
        setReport(null);
        try {
            const blueprint = {
                subjectId: paper?.subjectId || null,
                scoreScheme: paper?.scoreScheme || '',
                totalScore: total,
                relaxDifficulty: relax,
                knowledgeQuota: quota
                    .filter((item) => item.knowledgeId && Number(item.count) > 0)
                    .map((item) => ({knowledgeId: item.knowledgeId, count: Number(item.count)})),
                sections: rows.map((row) => ({
                    kind: row.kind,
                    title: row.title || '',
                    count: Number(row.count) || 0,
                    unitScore: Number(row.unitScore) || 0,
                    scoreMode: row.kind === 'CALC' ? 'PER_ITEM' : 'UNIFORM',
                    bands: Object.fromEntries(
                        BANDS.map((band) => [band.key, Number(row.bands[band.key]) || 0]).filter(
                            ([, value]) => value > 0,
                        ),
                    ),
                })),
            };
            const result = await paperBlueprintFill(paperId, blueprint, replace);
            setReport(result.report);
            if (result.report.filled) {
                notify.success(result.report.message);
                onFilled?.(result.paper);
            }
        } catch (error) {
            notify.error(error.message);
            setReport({filled: false, gaps: [], message: error.message});
        } finally {
            setBusy(false);
        }
    };

    const blocked = loading || planned <= 0 || Math.abs(delta) >= 0.001 || !bandsConsistent;

    return (
        <Modal
            title="按蓝图组卷"
            subtitle={`双向细目表：先定题数、分值、易中难，再由系统从已入库的题里抽。总分固定 ${scoreText(total)} 分。`}
            onClose={onClose}
            wide
            footer={
                <>
                    <span className={Math.abs(delta) < 0.001 ? 'muted' : 'delta-warn'}>
                        Σ = {scoreText(planned)} 分
                        {Math.abs(delta) < 0.001
                            ? ' · 与总分持平'
                            : delta > 0
                              ? ` · 还差 ${scoreText(delta)} 分`
                              : ` · 多了 ${scoreText(delta)} 分`}
                    </span>
                    <div className="drawer-actions-right">
                        <label className="bp-replace">
                            <input
                                type="checkbox"
                                checked={replace}
                                onChange={(event) => setReplace(event.target.checked)}
                            />
                            清空现有题目再组
                        </label>
                        <button className="btn ghost" onClick={onClose}>
                            取消
                        </button>
                        <button className="btn primary" onClick={() => submit(false)} disabled={busy || blocked}>
                            {busy ? '正在抽题…' : '按蓝图组卷'}
                        </button>
                    </div>
                </>
            }
        >
            <div className="bp-table">
                <div className="bp-row bp-head">
                    <span>大题</span>
                    <span>题数</span>
                    <span>每题分值</span>
                    <span>易 / 中 / 难</span>
                    <span>小计</span>
                    <span/>
                </div>
                {rows.map((row, index) => (
                    <div className="bp-row" key={index}>
                        <span className="bp-kind">
                            <select
                                value={row.kind}
                                onChange={(event) => patchRow(index, {kind: event.target.value})}
                            >
                                {KINDS.map((kind) => (
                                    <option key={kind.value} value={kind.value}>
                                        {kind.label}
                                    </option>
                                ))}
                            </select>
                        </span>
                        <input
                            type="number"
                            min="0"
                            step="1"
                            value={row.count}
                            onChange={(event) => patchRow(index, {count: Number(event.target.value) || 0})}
                        />
                        <input
                            type="number"
                            min="0"
                            step="0.5"
                            value={row.unitScore}
                            onChange={(event) =>
                                patchRow(index, {unitScore: Number(event.target.value) || 0})
                            }
                        />
                        <span className="bp-bands">
                            {BANDS.map((band) => (
                                <input
                                    key={band.key}
                                    type="number"
                                    min="0"
                                    step="1"
                                    title={`${band.label}（${band.hint}）`}
                                    placeholder={band.label}
                                    value={row.bands[band.key] || ''}
                                    onChange={(event) =>
                                        patchRow(index, {
                                            bands: {[band.key]: Number(event.target.value) || 0},
                                        })
                                    }
                                />
                            ))}
                        </span>
                        <span className="bp-sub">{scoreText(rowTotal(row))}</span>
                        <button
                            className="icon-btn"
                            title="删掉这一行"
                            onClick={() => setRows((current) => current.filter((_, at) => at !== index))}
                        >
                            ✕
                        </button>
                    </div>
                ))}
                <button className="btn ghost small" onClick={() => setRows((current) => [...current, emptyRow()])}>
                    + 加一个大题
                </button>
            </div>

            {!bandsConsistent && (
                <p className="bp-hint warn">
                    易/中/难 三格要么都留空（不限难度），要么加起来正好等于这一行的题数。
                </p>
            )}

            <div className="bp-quota">
                <div className="bp-quota-head">
                    <strong>知识点配额（可不填）</strong>
                    <button
                        className="btn ghost small"
                        onClick={() => setQuota((current) => [...current, {knowledgeId: '', count: 1}])}
                        disabled={options.length === 0}
                    >
                        + 指定知识点
                    </button>
                </div>
                {quota.length === 0 ? (
                    <p className="muted">
                        不指定就按学科题库分布抽；指定后优先进这些知识点，配额用完不再硬顶。
                    </p>
                ) : (
                    quota.map((item, index) => (
                        <div className="bp-quota-row" key={index}>
                            <select
                                value={item.knowledgeId}
                                onChange={(event) =>
                                    setQuota((current) =>
                                        current.map((entry, at) =>
                                            at === index ? {...entry, knowledgeId: event.target.value} : entry,
                                        ),
                                    )
                                }
                            >
                                <option value="">选知识点…</option>
                                {options.map((option) => (
                                    <option key={option.id} value={option.id}>
                                        {option.label}
                                    </option>
                                ))}
                            </select>
                            <input
                                type="number"
                                min="1"
                                step="1"
                                value={item.count}
                                onChange={(event) =>
                                    setQuota((current) =>
                                        current.map((entry, at) =>
                                            at === index
                                                ? {...entry, count: Number(event.target.value) || 1}
                                                : entry,
                                        ),
                                    )
                                }
                            />
                            <span className="muted">题</span>
                            <button
                                className="icon-btn"
                                onClick={() => setQuota((current) => current.filter((_, at) => at !== index))}
                            >
                                ✕
                            </button>
                        </div>
                    ))
                )}
            </div>

            {report && !report.filled && (
                <div className="bp-gaps">
                    <strong>没组成功，卷子没有改动</strong>
                    <p>{report.message}</p>
                    {report.gaps?.length > 0 && (
                        <ul>
                            {report.gaps.map((gap, index) => (
                                <li key={index}>
                                    {gap.sectionTitle} · {gap.label}：需要 {gap.need} 题，只找到 {gap.got} 题
                                </li>
                            ))}
                        </ul>
                    )}
                    <div className="bp-gaps-actions">
                        <button className="btn" onClick={() => submit(true)} disabled={busy}>
                            放宽难度再组一次
                        </button>
                        <span className="muted">放宽后不再按易中难挑，只保证题型与知识点。</span>
                    </div>
                </div>
            )}

            {loading && <p className="muted">正在读这份卷的大题结构…</p>}
            <p className="bp-hint">
                抽题规则：只取已入库（PUBLISHED）的题，本卷已用的不会重复；用过的少优先、新的优先。
                同一份蓝图 + 同一题库，抽出来的是同一套卷。
            </p>
        </Modal>
    );
}
