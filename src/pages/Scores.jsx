import {useCallback, useEffect, useMemo, useRef, useState} from 'react';
import {
    classList,
    scoreExamDelete,
    scoreExamSave,
    scoreExamSheet,
    scoreExams,
    scoreSave,
} from '../lib/api.js';
import {Card, Empty, Field, Modal, PageHead, Pill, confirmDialog, notify} from '../components/ui.jsx';
import {CountUp} from '../components/effects.jsx';
import {cx} from '../lib/format.js';
import {parseScoreImport} from '../lib/scoreImport.js';

const KINDS = [
    {key: 'EXAM', label: '考试'},
    {key: 'QUIZ', label: '小测'},
    {key: 'UNIT', label: '单元测'},
];

const KIND_LABEL = Object.fromEntries(KINDS.map((item) => [item.key, item.label]));

function kindLabel(kind) {
    return KIND_LABEL[kind] || '考试';
}

function today() {
    const now = new Date();
    const month = String(now.getMonth() + 1).padStart(2, '0');
    const day = String(now.getDate()).padStart(2, '0');
    return `${now.getFullYear()}-${month}-${day}`;
}

function fmt(value) {
    if (value == null) return '—';
    return Math.round(value * 10) / 10;
}

/**
 * 成绩单：一次考试一张表。
 *
 * 与「作业」的分工写在页头上——作业按题判分、系统自动汇总；这里录的是整卷总分，
 * 纸笔考完直接登分。所以这里不做逐题，只做「谁多少分、排第几、班均多少」。
 */
export default function Scores({onGo, onChanged}) {
    const [classes, setClasses] = useState([]);
    const [classId, setClassId] = useState('');
    const [exams, setExams] = useState([]);
    const [examId, setExamId] = useState('');
    const [sheet, setSheet] = useState(null);
    const [drafts, setDrafts] = useState({});
    const [loading, setLoading] = useState(true);
    const [saving, setSaving] = useState(false);
    const [error, setError] = useState('');
    const [form, setForm] = useState(null);
    const [importOpen, setImportOpen] = useState(false);
    const [importText, setImportText] = useState('');
    const [importPreview, setImportPreview] = useState(null);
    const [importSkipped, setImportSkipped] = useState(() => new Set());
    const [importFileName, setImportFileName] = useState('');
    const tableRef = useRef(null);

    const loadClasses = useCallback(async () => {
        try {
            const list = await classList();
            setClasses(list);
            setClassId((current) => {
                if (current && list.some((row) => row.id === current)) return current;
                return list[0]?.id || '';
            });
        } catch (err) {
            setError(err.message);
        } finally {
            setLoading(false);
        }
    }, []);

    const loadExams = useCallback(async (id) => {
        try {
            const list = await scoreExams(id);
            setExams(list);
            setExamId((current) => {
                if (current && list.some((row) => row.id === current)) return current;
                return list[0]?.id || '';
            });
        } catch (err) {
            setError(err.message);
            setLoading(false);
        }
    }, []);

    const loadSheet = useCallback(async (id) => {
        if (!id) {
            setSheet(null);
            setDrafts({});
            setLoading(false);
            return;
        }
        setLoading(true);
        try {
            const data = await scoreExamSheet(id);
            setSheet(data);
            setDrafts(
                Object.fromEntries(
                    data.rows.map((row) => [row.studentId, row.score == null ? '' : String(row.score)]),
                ),
            );
            setError('');
        } catch (err) {
            setError(err.message);
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => {
        loadClasses();
    }, [loadClasses]);

    useEffect(() => {
        if (classId) loadExams(classId);
    }, [classId, loadExams]);

    useEffect(() => {
        loadSheet(examId);
    }, [examId, loadSheet]);

    const dirty = useMemo(() => {
        if (!sheet) return [];
        return sheet.rows.filter((row) => {
            const draft = (drafts[row.studentId] ?? '').trim();
            const current = row.score == null ? '' : String(row.score);
            return draft !== current;
        });
    }, [sheet, drafts]);

    const openCreate = () => {
        const first = classes.find((row) => row.id === classId) || classes[0];
        if (!first) {
            notify.warning('先在「班级」里建一个班，成绩要挂在班级上');
            return;
        }
        setForm({
            id: '',
            classId: first?.id || '',
            title: '',
            kind: 'EXAM',
            fullScore: '100',
            passLine: '',
            goodLine: '',
            takenAt: today(),
            note: '',
        });
    };

    const openEdit = () => {
        if (!sheet) return;
        const exam = sheet.exam;
        setForm({
            id: exam.id,
            classId: exam.classId,
            title: exam.title,
            kind: exam.kind,
            fullScore: String(exam.fullScore),
            passLine: String(exam.passLine),
            goodLine: String(exam.goodLine),
            takenAt: exam.takenAt,
            note: exam.note,
        });
    };

    const saveExam = async () => {
        if (!form.classId) {
            notify.warning('先选一个班级');
            return;
        }
        if (!form.title.trim()) {
            notify.warning('考试名称不能为空');
            return;
        }
        const full = Number(form.fullScore);
        if (!Number.isFinite(full) || full <= 0) {
            notify.warning('满分要填一个大于 0 的数字');
            return;
        }
        try {
            const id = await scoreExamSave({
                id: form.id || null,
                classId: form.classId,
                title: form.title,
                kind: form.kind,
                fullScore: full,
                passLine: form.passLine.trim() === '' ? null : Number(form.passLine),
                goodLine: form.goodLine.trim() === '' ? null : Number(form.goodLine),
                takenAt: form.takenAt || null,
                note: form.note,
            });
            setForm(null);
            if (form.classId !== classId) setClassId(form.classId);
            await loadExams(form.classId);
            setExamId(id);
            notify.success('已保存');
            onChanged?.();
        } catch (err) {
            notify.error(err.message);
        }
    };

    const removeExam = async () => {
        if (!sheet) return;
        const ok = await confirmDialog({
            title: '删除这次考试？',
            message: `「${sheet.exam.title}」的分数会一起消失，学生学情里的考试记录也会去掉。`,
            confirmText: '删除',
            danger: true,
        });
        if (!ok) return;
        try {
            await scoreExamDelete(sheet.exam.id);
            setExamId('');
            await loadExams(classId);
            notify.success('已删除');
            onChanged?.();
        } catch (err) {
            notify.error(err.message);
        }
    };

    const saveScores = async () => {
        if (!sheet) return;
        const rows = [];
        for (const row of sheet.rows) {
            const raw = (drafts[row.studentId] ?? '').trim();
            if (raw === '') {
                rows.push({studentId: row.studentId, score: null, note: row.note || ''});
                continue;
            }
            const value = Number(raw);
            if (!Number.isFinite(value)) {
                notify.warning(`${row.name} 的分数不是数字：${raw}`);
                return;
            }
            if (value < 0 || value > sheet.exam.fullScore) {
                notify.warning(`${row.name} 的分数超出 0–${sheet.exam.fullScore}`);
                return;
            }
            rows.push({studentId: row.studentId, score: value, note: row.note || ''});
        }
        setSaving(true);
        try {
            await scoreSave(sheet.exam.id, rows);
            await loadSheet(sheet.exam.id);
            await loadExams(classId);
            notify.success('成绩已保存');
            onChanged?.();
        } catch (err) {
            notify.error(err.message);
        } finally {
            setSaving(false);
        }
    };

    const exportCsv = () => {
        if (!sheet) return;
        const head = ['学号', '姓名', '分数', '满分', '名次', '与班均', '备注'];
        const lines = sheet.rows.map((row) => [
            row.studentNo || '',
            row.name,
            row.score == null ? '' : row.score,
            sheet.exam.fullScore,
            row.rank ?? '',
            row.vsAverage == null ? '' : row.vsAverage,
            row.note || '',
        ]);
        const csv = [head, ...lines]
            .map((cells) => cells.map((cell) => `"${String(cell).replaceAll('"', '""')}"`).join(','))
            .join('\r\n');
        const blob = new Blob([`\ufeff${csv}`], {type: 'text/csv;charset=utf-8'});
        const url = URL.createObjectURL(blob);
        const link = document.createElement('a');
        link.href = url;
        link.download = `${sheet.exam.className}-${sheet.exam.title}-成绩.csv`;
        link.click();
        URL.revokeObjectURL(url);
    };

    const openImport = () => {
        setImportText('');
        setImportFileName('');
        setImportPreview(null);
        setImportSkipped(new Set());
        setImportOpen(true);
    };

    const pickImportFile = async (event) => {
        const file = event.target.files?.[0];
        if (!file) return;
        try {
            const text = await file.text();
            setImportText(text);
            setImportFileName(file.name);
            setImportPreview(null);
        } catch (err) {
            notify.error(`读文件失败：${err.message}`);
        }
        event.target.value = '';
    };

    const previewImport = () => {
        if (!sheet) return;
        const data = parseScoreImport(importText, sheet.rows, sheet.exam.fullScore);
        setImportPreview(data);
        setImportSkipped(new Set());
        if (data.total === 0) notify.warning('没读到数据行，检查一下粘贴的内容');
    };

    const applyImport = () => {
        if (!importPreview) return;
        const picked = importPreview.rows.filter(
            (row, at) => row.status === 'ok' && !importSkipped.has(at),
        );
        if (picked.length === 0) {
            notify.warning('没有可填入的分数');
            return;
        }
        setDrafts((current) => {
            const next = {...current};
            picked.forEach((row) => {
                next[row.studentId] = row.score == null ? '' : String(row.score);
            });
            return next;
        });
        setImportOpen(false);
        notify.success(`已填入 ${picked.length} 人，点「保存成绩」才生效`);
    };

    if (classes.length === 0 && !loading) {
        return (
            <div className="page">
                <PageHead title="成绩" subtitle="整卷总分单独登分；作业成绩在作业页与个人学情里" help="scores"/>
                <Card>
                    <Empty
                        title="还没有班级"
                        hint="成绩挂在班级上：先建班、加学生，再录第一次考试"
                        action={
                            <button className="btn primary" onClick={() => onGo?.('classes')}>
                                去班级
                            </button>
                        }
                    />
                </Card>
            </div>
        );
    }

    const stats = sheet?.stats;
    const distMax = Math.max(1, ...(sheet?.dist || []).map((bucket) => bucket.count));

    return (
        <div className="page">
            <PageHead
                title="成绩"
                subtitle="整卷总分单独登分；按题判分的作业成绩仍在作业页与个人学情里"
                help="scores"
                actions={
                    <div className="inline-actions">
                        {sheet && (
                            <>
                                <button className="btn ghost small" onClick={exportCsv}>
                                    导出 CSV
                                </button>
                                <button className="btn ghost small" onClick={openEdit}>
                                    改考试信息
                                </button>
                            </>
                        )}
                        <button className="btn primary small" onClick={openCreate}>
                            新建考试
                        </button>
                    </div>
                }
            />

            {error && <p className="error-text">{error}</p>}

            <div className="ins-bar">
                {classes.map((row) => (
                    <button
                        key={row.id}
                        className={cx('ins-chip', row.id === classId && 'on')}
                        onClick={() => setClassId(row.id)}
                    >
                        {row.name}
                        <span className="muted">{row.memberCount ? `${row.memberCount} 人` : ''}</span>
                    </button>
                ))}
                <span className="grow"/>
                <button className="btn ghost small" onClick={() => loadExams(classId)}>
                    刷新
                </button>
            </div>

            {exams.length === 0 ? (
                <Card>
                    <Empty
                        title="这个班还没有考试记录"
                        hint="录一次月考或单元测：填名称、满分、日期，然后按学号登分"
                        action={
                            <button className="btn primary" onClick={openCreate}>
                                新建考试
                            </button>
                        }
                    />
                </Card>
            ) : (
                <div className="sc-list">
                    {exams.map((exam) => (
                        <button
                            key={exam.id}
                            className={cx('sc-card', exam.id === examId && 'on')}
                            onClick={() => setExamId(exam.id)}
                        >
                            <div className="sc-card-top">
                                <span className="sc-card-title">{exam.title}</span>
                                <Pill tone={exam.kind === 'EXAM' ? 'info' : 'muted'}>{kindLabel(exam.kind)}</Pill>
                            </div>
                            <div className="sc-card-meta">
                                <span>{exam.takenAt}</span>
                                <span>满分 {fmt(exam.fullScore)}</span>
                                <span>
                                    已录 {exam.entered}/{exam.studentCount}
                                </span>
                            </div>
                            <div className="sc-card-num">
                                <b>{exam.average == null ? '—' : fmt(exam.average)}</b>
                                <span>班均</span>
                                {exam.goodRate != null && <em>优秀 {Math.round(exam.goodRate)}%</em>}
                            </div>
                        </button>
                    ))}
                </div>
            )}

            {sheet && (
                <>
                    <div className="sc-kpis">
                        <div className="sc-kpi">
                            <b><CountUp value={stats.average == null ? '—' : fmt(stats.average)}/></b>
                            <span>平均分</span>
                        </div>
                        <div className="sc-kpi">
                            <b><CountUp value={stats.median == null ? '—' : fmt(stats.median)}/></b>
                            <span>中位数</span>
                        </div>
                        <div className="sc-kpi">
                            <b><CountUp value={stats.highest == null ? '—' : fmt(stats.highest)}/></b>
                            <span>最高</span>
                        </div>
                        <div className="sc-kpi">
                            <b><CountUp value={stats.lowest == null ? '—' : fmt(stats.lowest)}/></b>
                            <span>最低</span>
                        </div>
                        <div className="sc-kpi">
                            <b><CountUp value={stats.passRate == null ? '—' : `${Math.round(stats.passRate)}%`}/></b>
                            <span>及格率</span>
                            <em>
                                {stats.pass} / {stats.entered}
                            </em>
                        </div>
                        <div className="sc-kpi">
                            <b><CountUp value={stats.goodRate == null ? '—' : `${Math.round(stats.goodRate)}%`}/></b>
                            <span>优秀率</span>
                            <em>
                                {stats.good} / {stats.entered}
                            </em>
                        </div>
                        <div className="sc-kpi">
                            <b><CountUp value={stats.missing}/></b>
                            <span>缺考 / 未录</span>
                        </div>
                    </div>

                    <div className="sc-grid">
                        <Card title="分数段" subtitle="按满分百分比切档，100 / 150 分制都能直接看">
                            <div className="sc-dist">
                                {sheet.dist.map((bucket) => (
                                    <div className="sc-dist-row" key={bucket.label}>
                                        <span className="sc-dist-label">{bucket.label}</span>
                                        <div className="sc-dist-bar">
                                            <i
                                                className={cx('sc-dist-fill', bucket.label === '不及格' && 'bad')}
                                                style={{width: `${Math.max((bucket.count / distMax) * 100, 2)}%`}}
                                            />
                                        </div>
                                        <b>{bucket.count}</b>
                                    </div>
                                ))}
                            </div>
                            <p className="sc-hint">
                                及格线 {fmt(sheet.exam.passLine)}，优秀线 {fmt(sheet.exam.goodLine)}；
                                留空时按满分的 60% / 85% 算。
                            </p>
                        </Card>

                        <Card
                            title="登分"
                            subtitle={
                                dirty.length > 0
                                    ? `${dirty.length} 人改动未保存`
                                    : '直接改分数，空着表示缺考；保存后才进统计与学情'
                            }
                            actions={
                                <div className="inline-actions">
                                    <button className="btn ghost small" onClick={openImport}>
                                        粘贴 / 导入
                                    </button>
                                    <button
                                        className="btn ghost small"
                                        onClick={() => loadSheet(examId)}
                                        disabled={dirty.length === 0}
                                    >
                                        放弃改动
                                    </button>
                                    <button className="btn primary small" onClick={saveScores} disabled={saving || dirty.length === 0}>
                                        {saving ? '保存中…' : '保存成绩'}
                                    </button>
                                </div>
                            }
                        >
                            <div className="sc-table" ref={tableRef}>
                                <div className="sc-row sc-head">
                                    <span>学号</span>
                                    <span>姓名</span>
                                    <span>分数</span>
                                    <span>名次</span>
                                    <span>与班均</span>
                                    <span>备注</span>
                                </div>
                                {sheet.rows.map((row) => {
                                    const draft = drafts[row.studentId] ?? '';
                                    const changed = (row.score == null ? '' : String(row.score)) !== draft.trim();
                                    return (
                                        <div className={cx('sc-row', changed && 'changed')} key={row.studentId}>
                                            <span className="sc-no">{row.studentNo || '—'}</span>
                                            <span className="sc-name">{row.name}</span>
                                            <span>
                                                <input
                                                    className="sc-input"
                                                    inputMode="decimal"
                                                    value={draft}
                                                    placeholder="缺考"
                                                    onChange={(event) =>
                                                        setDrafts((current) => ({
                                                            ...current,
                                                            [row.studentId]: event.target.value,
                                                        }))
                                                    }
                                                    onKeyDown={(event) => {
                                                        if (event.key !== 'Enter') return;
                                                        const inputs = tableRef.current?.querySelectorAll('.sc-input');
                                                        const list = Array.from(inputs || []);
                                                        const at = list.indexOf(event.currentTarget);
                                                        list[at + 1]?.focus();
                                                    }}
                                                />
                                                <em className="sc-full">/ {fmt(sheet.exam.fullScore)}</em>
                                            </span>
                                            <span className="sc-rank">{changed ? '待存' : row.rank ?? '—'}</span>
                                            <span className={cx('sc-vs', (row.vsAverage || 0) > 0 && 'up', (row.vsAverage || 0) < 0 && 'down')}>
                                                {changed || row.vsAverage == null
                                                    ? '—'
                                                    : row.vsAverage > 0
                                                      ? `+${fmt(row.vsAverage)}`
                                                      : fmt(row.vsAverage)}
                                            </span>
                                            <span className="sc-note">{row.note || ''}</span>
                                        </div>
                                    );
                                })}
                            </div>
                            <div className="sc-foot">
                                <span className="muted small">
                                    按 Enter 跳到下一个人；也可以「粘贴 / 导入」成绩单，按学号自动对上名单。
                                </span>
                                <span className="grow"/>
                                <button className="btn ghost small danger" onClick={removeExam}>
                                    删除这次考试
                                </button>
                            </div>
                        </Card>
                    </div>
                </>
            )}

            {form && (
                <Modal
                    title={form.id ? '修改考试信息' : '新建考试'}
                    subtitle="满分决定及格线 / 优秀线的默认值；改满分不会动已录的分数"
                    onClose={() => setForm(null)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={() => setForm(null)}>
                                取消
                            </button>
                            <button className="btn primary" onClick={saveExam}>
                                保存
                            </button>
                        </>
                    }
                >
                    <div className="form-grid">
                        <Field label="名称" wide>
                            <input
                                autoFocus
                                value={form.title}
                                placeholder="第一次月考"
                                onChange={(event) => setForm({...form, title: event.target.value})}
                            />
                        </Field>
                        <Field label="班级">
                            <select
                                value={form.classId}
                                onChange={(event) => setForm({...form, classId: event.target.value})}
                            >
                                {classes.map((row) => (
                                    <option key={row.id} value={row.id}>
                                        {row.name}
                                    </option>
                                ))}
                            </select>
                        </Field>
                        <Field label="类型">
                            <select value={form.kind} onChange={(event) => setForm({...form, kind: event.target.value})}>
                                {KINDS.map((item) => (
                                    <option key={item.key} value={item.key}>
                                        {item.label}
                                    </option>
                                ))}
                            </select>
                        </Field>
                        <Field label="满分">
                            <input
                                inputMode="decimal"
                                value={form.fullScore}
                                onChange={(event) => setForm({...form, fullScore: event.target.value})}
                            />
                        </Field>
                        <Field label="考试日期">
                            <input
                                value={form.takenAt}
                                placeholder="2026-11-18"
                                onChange={(event) => setForm({...form, takenAt: event.target.value})}
                            />
                        </Field>
                        <Field label="及格线" hint="留空 = 满分 × 60%">
                            <input
                                inputMode="decimal"
                                value={form.passLine}
                                placeholder="60"
                                onChange={(event) => setForm({...form, passLine: event.target.value})}
                            />
                        </Field>
                        <Field label="优秀线" hint="留空 = 满分 × 85%">
                            <input
                                inputMode="decimal"
                                value={form.goodLine}
                                placeholder="85"
                                onChange={(event) => setForm({...form, goodLine: event.target.value})}
                            />
                        </Field>
                        <Field label="备注" wide>
                            <input
                                value={form.note}
                                placeholder="范围 / 难度 / 是否计入期末…"
                                onChange={(event) => setForm({...form, note: event.target.value})}
                            />
                        </Field>
                    </div>
                </Modal>
            )}

            {importOpen && (
                <Modal
                    title="批量登分"
                    subtitle="按学号自动匹配本班名单，只填「学号 + 分数」两列也行"
                    wide
                    onClose={() => setImportOpen(false)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={() => setImportOpen(false)}>
                                取消
                            </button>
                            <button className="btn ghost" onClick={previewImport} disabled={!importText.trim()}>
                                预览匹配
                            </button>
                            <button
                                className="btn primary"
                                onClick={applyImport}
                                disabled={
                                    !importPreview ||
                                    importPreview.rows.every(
                                        (row, at) => row.status !== 'ok' || importSkipped.has(at),
                                    )
                                }
                            >
                                填入登分表
                            </button>
                        </>
                    }
                >
                    <div className="sc-import-tools">
                        <label className="btn ghost small sc-import-file">
                            选择 CSV / TXT
                            <input
                                type="file"
                                accept=".csv,.tsv,.txt,text/csv,text/plain"
                                onChange={pickImportFile}
                            />
                        </label>
                        {importFileName && <span className="muted small">{importFileName}</span>}
                        <span className="grow"/>
                        <span className="muted small">也可以从表格复制后直接粘到下面</span>
                    </div>
                    <textarea
                        className="teach-import"
                        rows={8}
                        placeholder={'学号,姓名,分数\n2026001,张三,92\n2026002,李四,85\n（支持 Excel 复制的 Tab 分隔、CSV 逗号、空格分隔；表头可省略）'}
                        value={importText}
                        onChange={(event) => {
                            setImportText(event.target.value);
                            setImportPreview(null);
                        }}
                    />
                    <p className="sc-hint">
                        留空 / 写「缺考」表示缺考；重复学号只取第一行，名单里没有的学号和非法分数会在预览里标出来，勾选可跳过。
                    </p>
                    {importPreview && (
                        <>
                            <p className="teach-import-note">
                                读到 {importPreview.total} 行：匹配 {importPreview.matched} 行（缺考 {importPreview.blank} 行）
                                {importPreview.missing > 0 && `，未匹配 ${importPreview.missing} 行`}
                                {importPreview.invalid > 0 && `，分数有问题 ${importPreview.invalid} 行`}
                                {importPreview.duplicate > 0 && `，重复 ${importPreview.duplicate} 行`}。
                                只有匹配的行会填入，取消勾选可跳过。
                            </p>
                            <div className="teach-import-list sc-import-list">
                                {importPreview.rows.map((row, at) => {
                                    const usable = row.status === 'ok';
                                    const skipped = importSkipped.has(at);
                                    return (
                                        <label
                                            key={`${row.line}-${at}`}
                                            className={cx(
                                                'teach-import-row',
                                                'sc-import-row',
                                                !usable && 'dup',
                                                skipped && 'off',
                                            )}
                                        >
                                            <input
                                                type="checkbox"
                                                checked={usable && !skipped}
                                                disabled={!usable}
                                                onChange={() =>
                                                    setImportSkipped((current) => {
                                                        const next = new Set(current);
                                                        if (next.has(at)) next.delete(at);
                                                        else next.add(at);
                                                        return next;
                                                    })
                                                }
                                            />
                                            <span className="sc-imp-no">{row.rawNo || row.rawName || '—'}</span>
                                            <span className="sc-imp-name">{row.name || '—'}</span>
                                            <span className="sc-imp-score">
                                                {row.blank ? '缺考' : row.score == null ? '—' : row.score}
                                            </span>
                                            <span className={cx('sc-imp-status', row.status)}>
                                                {usable
                                                    ? row.matchBy === 'name'
                                                        ? '按姓名匹配'
                                                        : '匹配'
                                                    : row.reason}
                                            </span>
                                        </label>
                                    );
                                })}
                            </div>
                        </>
                    )}
                </Modal>
            )}
        </div>
    );
}
