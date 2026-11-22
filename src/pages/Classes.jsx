import {useCallback, useEffect, useMemo, useState} from 'react';
import {
    classAddMembers,
    classClose,
    classDelete,
    classList,
    classMembers,
    classRegenerateCode,
    classRemoveMember,
    classSave,
    studentArchive,
    studentDelete,
    studentImportCommit,
    studentImportPreview,
    studentList,
    studentSave,
} from '../lib/api.js';
import {
    Card,
    Drawer,
    Empty,
    Field,
    Modal,
    PageHead,
    Pill,
    Tabs,
    confirmDialog,
    notify,
} from '../components/ui.jsx';

const EMPTY_CLASS = {name: '', subjectId: '', term: ''};
const EMPTY_STUDENT = {name: '', studentNo: '', grade: '', subjectIds: [], note: ''};

function copy(text) {
    if (navigator.clipboard?.writeText) {
        navigator.clipboard.writeText(text).then(
            () => notify.success('邀请码已复制'),
            () => notify.warning(`复制失败，手动记一下：${text}`),
        );
        return;
    }
    notify.info(`邀请码：${text}`);
}

/** 学生姓名首字当头像，和侧栏保持同一种视觉。 */
function Avatar({name}) {
    return <span className="teach-avatar">{(name || '?').slice(0, 1)}</span>;
}

export default function Classes({tree = [], onGo, onChanged}) {
    const [tab, setTab] = useState('classes');
    const [classes, setClasses] = useState([]);
    const [students, setStudents] = useState([]);
    const [keyword, setKeyword] = useState('');
    const [status, setStatus] = useState('ACTIVE');
    const [loading, setLoading] = useState(true);

    const [classForm, setClassForm] = useState(null);
    const [studentForm, setStudentForm] = useState(null);
    const [editingStudentId, setEditingStudentId] = useState('');
    const [importOpen, setImportOpen] = useState(false);
    const [importText, setImportText] = useState('');
    const [preview, setPreview] = useState(null);

    const [detail, setDetail] = useState(null);
    const [members, setMembers] = useState([]);
    const [allStudents, setAllStudents] = useState([]);
    const [memberKeyword, setMemberKeyword] = useState('');
    const [addOpen, setAddOpen] = useState(false);
    const [picked, setPicked] = useState([]);

    const subjects = useMemo(
        () => tree.map((subject) => ({id: subject.id, name: subject.name})),
        [tree],
    );

    const loadClasses = useCallback(async () => {
        try {
            setClasses(await classList());
        } catch (error) {
            notify.error(error.message);
        }
    }, []);

    const loadStudents = useCallback(async () => {
        try {
            setStudents(await studentList(keyword, status));
        } catch (error) {
            notify.error(error.message);
        }
    }, [keyword, status]);

    useEffect(() => {
        Promise.all([loadClasses(), loadStudents()]).finally(() => setLoading(false));
    }, [loadClasses, loadStudents]);

    const refreshAll = async () => {
        await Promise.all([loadClasses(), loadStudents()]);
        onChanged?.();
    };

    const openDetail = async (row) => {
        setDetail(row);
        setMemberKeyword('');
        try {
            setMembers(await classMembers(row.id));
        } catch (error) {
            notify.error(error.message);
        }
    };

    const reloadMembers = async (classId) => {
        const next = await classMembers(classId);
        setMembers(next);
        await loadClasses();
    };

    const saveClass = async () => {
        if (!classForm.name.trim()) {
            notify.warning('班级名称不能为空');
            return;
        }
        try {
            const saved = await classSave({
                id: classForm.id || null,
                name: classForm.name,
                subjectId: classForm.subjectId || null,
                term: classForm.term,
            });
            setClassForm(null);
            await refreshAll();
            if (detail && detail.id === saved.id) setDetail(saved);
            notify.success('班级已保存');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const toggleClassStatus = async (row) => {
        const closing = row.status === 'ACTIVE';
        const ok = await confirmDialog({
            title: closing ? '结课' : '重新开课',
            message: closing
                ? `结课后「${row.name}」会排到列表后面，已交作业与成绩都保留。`
                : `把「${row.name}」恢复为进行中？`,
            confirmText: closing ? '结课' : '重新开课',
        });
        if (!ok) return;
        try {
            await classClose(row.id, closing);
            await refreshAll();
            notify.success(closing ? '已结课' : '已重新开课');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const removeClass = async (row) => {
        const ok = await confirmDialog({
            title: '删除班级',
            message: `删除「${row.name}」后，班级、作业与作答都会从列表里消失（数据仍保留在本机，可追溯）。学生档案不受影响。`,
            confirmText: '删除',
            danger: true,
        });
        if (!ok) return;
        try {
            await classDelete(row.id);
            if (detail?.id === row.id) setDetail(null);
            await refreshAll();
            notify.success('班级已删除');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const regenerate = async (row) => {
        const ok = await confirmDialog({
            title: '重置邀请码',
            message: '旧邀请码立即失效，已经加入的学生不受影响。',
            confirmText: '重置',
        });
        if (!ok) return;
        try {
            const code = await classRegenerateCode(row.id);
            await loadClasses();
            if (detail?.id === row.id) setDetail({...detail, joinCode: code});
            notify.success(`新邀请码：${code}`);
        } catch (error) {
            notify.error(error.message);
        }
    };

    const saveStudent = async () => {
        if (!studentForm.name.trim()) {
            notify.warning('学生姓名不能为空');
            return;
        }
        try {
            await studentSave({
                id: editingStudentId || null,
                name: studentForm.name,
                studentNo: studentForm.studentNo || '',
                grade: studentForm.grade,
                subjectIds: studentForm.subjectIds,
                note: studentForm.note,
            });
            setStudentForm(null);
            setEditingStudentId('');
            await refreshAll();
            if (detail) await reloadMembers(detail.id);
            notify.success('学生已保存');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const archiveStudent = async (row, archived) => {
        try {
            await studentArchive(row.id, archived);
            await refreshAll();
            notify.success(archived ? '已归档' : '已恢复');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const deleteStudent = async (row) => {
        const ok = await confirmDialog({
            title: '删除学生',
            message: `「${row.name}」会从名单和所有班级里移除，历史作答仍保留在本机。`,
            confirmText: '删除',
            danger: true,
        });
        if (!ok) return;
        try {
            await studentDelete(row.id);
            await refreshAll();
            if (detail) await reloadMembers(detail.id);
            notify.success('已删除');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const previewImport = async () => {
        try {
            setPreview(await studentImportPreview(importText));
        } catch (error) {
            notify.error(error.message);
        }
    };

    const commitImport = async () => {
        try {
            const result = await studentImportCommit(importText);
            setImportOpen(false);
            setImportText('');
            setPreview(null);
            await refreshAll();
            notify.success(
                `导入完成：新增 ${result.created} 人${result.skipped ? `，跳过重复 ${result.skipped} 人` : ''}`,
            );
        } catch (error) {
            notify.error(error.message);
        }
    };

    // 添加成员用全量名单，不受「学生」标签页的搜索结果影响。
    const openAdd = async () => {
        setMemberKeyword('');
        setPicked([]);
        setAddOpen(true);
        try {
            setAllStudents(await studentList('', 'ACTIVE'));
        } catch (error) {
            notify.error(error.message);
        }
    };

    const addMembers = async () => {
        if (picked.length === 0) {
            notify.warning('先勾选学生');
            return;
        }
        try {
            const added = await classAddMembers(detail.id, picked);
            setAddOpen(false);
            setPicked([]);
            await reloadMembers(detail.id);
            notify.success(`已加入 ${added} 人`);
        } catch (error) {
            notify.error(error.message);
        }
    };

    const removeMember = async (student) => {
        try {
            await classRemoveMember(detail.id, student.id);
            await reloadMembers(detail.id);
            notify.success(`已把 ${student.name} 移出班级`);
        } catch (error) {
            notify.error(error.message);
        }
    };

    const memberIds = new Set(members.map((item) => item.id));
    const candidates = allStudents.filter(
        (item) =>
            item.status === 'ACTIVE' &&
            !memberIds.has(item.id) &&
            (!memberKeyword.trim() || item.name.includes(memberKeyword.trim())),
    );
    const visibleMembers = members.filter(
        (item) => !memberKeyword.trim() || item.name.includes(memberKeyword.trim()),
    );

    return (
        <div className="page">
            <PageHead
                title="班级"
                subtitle="本地建班、导入名单、布置作业与批改；断网也能用，联网后再同步到中心"
                help="classes"
                actions={
                    tab === 'classes' ? (
                        <button className="btn primary" onClick={() => setClassForm({...EMPTY_CLASS})}>
                            新建班级
                        </button>
                    ) : (
                        <>
                            <button className="btn ghost" onClick={() => setImportOpen(true)}>
                                批量导入
                            </button>
                            <button
                                className="btn primary"
                                onClick={() => {
                                    setEditingStudentId('');
                                    setStudentForm({...EMPTY_STUDENT});
                                }}
                            >
                                新建学生
                            </button>
                        </>
                    )
                }
            />

            <Tabs
                items={[
                    {value: 'classes', label: '班级', count: classes.length},
                    {value: 'students', label: '学生', count: students.length},
                ]}
                value={tab}
                onChange={setTab}
            />

            {tab === 'classes' && (
                <div className="teach-grid">
                    {classes.map((row) => (
                        <article key={row.id} className={`teach-card ${row.status === 'CLOSED' ? 'closed' : ''}`}>
                            <div className="teach-card-head">
                                <h3>{row.name}</h3>
                                <Pill tone={row.status === 'ACTIVE' ? 'ok' : 'muted'}>
                                    {row.status === 'ACTIVE' ? '进行中' : '已结课'}
                                </Pill>
                            </div>
                            <p className="teach-card-sub">
                                {row.subjectName || '未设学科'} · {row.term || '未填学期'}
                            </p>
                            <div className="teach-card-stats">
                                <span>
                                    <strong>{row.memberCount}</strong> 人
                                </span>
                                <span>
                                    <strong>{row.assignmentCount}</strong> 份作业
                                </span>
                                {row.pendingGrade > 0 ? (
                                    <span className="hot">
                                        <strong>{row.pendingGrade}</strong> 待批
                                    </span>
                                ) : (
                                    <span className="muted">无待批</span>
                                )}
                            </div>
                            <button
                                className="teach-code"
                                title="点击复制邀请码"
                                onClick={() => copy(row.joinCode)}
                            >
                                邀请码 <code>{row.joinCode}</code>
                                <span className="teach-code-hint">复制</span>
                            </button>
                            <div className="teach-card-actions">
                                <button className="btn ghost small" onClick={() => openDetail(row)}>
                                    成员
                                </button>
                                <button
                                    className="btn primary small"
                                    onClick={() => onGo(`assignments/${row.id}`)}
                                >
                                    作业
                                </button>
                                <button
                                    className="btn ghost small"
                                    onClick={() => setClassForm({
                                        id: row.id,
                                        name: row.name,
                                        subjectId: row.subjectId || '',
                                        term: row.term,
                                    })}
                                >
                                    编辑
                                </button>
                                <button className="btn ghost small" onClick={() => toggleClassStatus(row)}>
                                    {row.status === 'ACTIVE' ? '结课' : '开课'}
                                </button>
                                <button className="btn ghost small danger-text" onClick={() => removeClass(row)}>
                                    删除
                                </button>
                            </div>
                        </article>
                    ))}
                    {!loading && classes.length === 0 && (
                        <div className="teach-blank">
                            <Empty
                                title="还没有班级"
                                hint="先建一个班，再把学生加进来；作业和学情都挂在班级下面。"
                                action={
                                    <button className="btn primary" onClick={() => setClassForm({...EMPTY_CLASS})}>
                                        新建班级
                                    </button>
                                }
                            />
                        </div>
                    )}
                </div>
            )}

            {tab === 'students' && (
                <Card>
                    <div className="teach-toolbar">
                        <input
                            className="teach-search"
                            placeholder="搜索姓名 / 学号 / 年级 / 备注"
                            value={keyword}
                            onChange={(event) => setKeyword(event.target.value)}
                        />
                        <div className="chip-row">
                            {[
                                {value: 'ACTIVE', label: '在读'},
                                {value: 'ARCHIVED', label: '已归档'},
                                {value: 'ALL', label: '全部'},
                            ].map((item) => (
                                <button
                                    key={item.value}
                                    className={`chip ${status === item.value ? 'active' : ''}`}
                                    onClick={() => setStatus(item.value)}
                                >
                                    {item.label}
                                </button>
                            ))}
                        </div>
                    </div>

                    {students.length === 0 ? (
                        <Empty
                            title={students.length === 0 && !keyword ? '还没有学生' : '没有匹配的学生'}
                            hint="可以一个个加，也可以粘贴一份名单批量导入（学号, 姓名, 年级, 备注）。"
                            action={
                                <button className="btn ghost" onClick={() => setImportOpen(true)}>
                                    批量导入
                                </button>
                            }
                        />
                    ) : (
                        <div className="teach-table">
                            <div className="teach-tr teach-th">
                                <span>学号</span>
                                <span>姓名</span>
                                <span>年级</span>
                                <span>科目</span>
                                <span>班级</span>
                                <span>作答</span>
                                <span>操作</span>
                            </div>
                            {students.map((row) => (
                                <div key={row.id} className="teach-tr">
                                    <span className="teach-no">{row.studentNo || '—'}</span>
                                    <span className="teach-name">
                                        <Avatar name={row.name}/>
                                        <span>
                                            {row.name}
                                            {row.note && <small>{row.note}</small>}
                                        </span>
                                        {row.status === 'ARCHIVED' && <Pill tone="muted">已归档</Pill>}
                                    </span>
                                    <span>{row.grade || '—'}</span>
                                    <span>{row.subjectNames.join('、') || '—'}</span>
                                    <span>{row.classCount || '—'}</span>
                                    <span>{row.attemptCount || '—'}</span>
                                    <span className="teach-ops">
                                        <button
                                            className="btn ghost small"
                                            onClick={() => {
                                                setEditingStudentId(row.id);
                                                setStudentForm({
                                                    name: row.name,
                                                    studentNo: row.studentNo || '',
                                                    grade: row.grade,
                                                    subjectIds: row.subjectIds,
                                                    note: row.note,
                                                });
                                            }}
                                        >
                                            编辑
                                        </button>
                                        <button
                                            className="btn ghost small"
                                            onClick={() => archiveStudent(row, row.status !== 'ARCHIVED')}
                                        >
                                            {row.status === 'ARCHIVED' ? '恢复' : '归档'}
                                        </button>
                                        <button
                                            className="btn ghost small danger-text"
                                            onClick={() => deleteStudent(row)}
                                        >
                                            删除
                                        </button>
                                    </span>
                                </div>
                            ))}
                        </div>
                    )}
                </Card>
            )}

            {classForm && (
                <Modal
                    title={classForm.id ? '编辑班级' : '新建班级'}
                    subtitle="名字自己看得懂就行，比如「七年级一班 · 数学」"
                    onClose={() => setClassForm(null)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={() => setClassForm(null)}>
                                取消
                            </button>
                            <button className="btn primary" onClick={saveClass}>
                                保存
                            </button>
                        </>
                    }
                >
                    <div className="form-grid">
                        <Field label="班级名称" wide>
                            <input
                                autoFocus
                                value={classForm.name}
                                placeholder="七年级一班"
                                onChange={(event) => setClassForm({...classForm, name: event.target.value})}
                            />
                        </Field>
                        <Field label="学科">
                            <select
                                value={classForm.subjectId}
                                onChange={(event) => setClassForm({...classForm, subjectId: event.target.value})}
                            >
                                <option value="">未指定</option>
                                {subjects.map((subject) => (
                                    <option key={subject.id} value={subject.id}>
                                        {subject.name}
                                    </option>
                                ))}
                            </select>
                        </Field>
                        <Field label="学期">
                            <input
                                value={classForm.term}
                                placeholder="2026 秋"
                                onChange={(event) => setClassForm({...classForm, term: event.target.value})}
                            />
                        </Field>
                    </div>
                </Modal>
            )}

            {studentForm && (
                <Modal
                    title={editingStudentId ? '编辑学生' : '新建学生'}
                    subtitle="本地档案，不要求中心账号；之后可以再绑定"
                    onClose={() => {
                        setStudentForm(null);
                        setEditingStudentId('');
                    }}
                    footer={
                        <>
                            <button
                                className="btn ghost"
                                onClick={() => {
                                    setStudentForm(null);
                                    setEditingStudentId('');
                                }}
                            >
                                取消
                            </button>
                            <button className="btn primary" onClick={saveStudent}>
                                保存
                            </button>
                        </>
                    }
                >
                    <div className="form-grid">
                        <Field label="姓名" wide>
                            <input
                                autoFocus
                                value={studentForm.name}
                                placeholder="张三"
                                onChange={(event) => setStudentForm({...studentForm, name: event.target.value})}
                            />
                        </Field>
                        <Field label="学号" hint="留空自动编号（1001 起）">
                            <input
                                value={studentForm.studentNo}
                                placeholder="2026001"
                                onChange={(event) => setStudentForm({...studentForm, studentNo: event.target.value})}
                            />
                        </Field>
                        <Field label="年级">
                            <input
                                value={studentForm.grade}
                                placeholder="七年级"
                                onChange={(event) => setStudentForm({...studentForm, grade: event.target.value})}
                            />
                        </Field>
                        <Field label="备注">
                            <input
                                value={studentForm.note}
                                placeholder="课代表 / 需要关注…"
                                onChange={(event) => setStudentForm({...studentForm, note: event.target.value})}
                            />
                        </Field>
                        <Field label="关注科目" wide hint="只影响筛选，不影响作业">
                            <div className="chip-row">
                                {subjects.map((subject) => {
                                    const active = studentForm.subjectIds.includes(subject.id);
                                    return (
                                        <button
                                            key={subject.id}
                                            className={`chip ${active ? 'active' : ''}`}
                                            onClick={() =>
                                                setStudentForm({
                                                    ...studentForm,
                                                    subjectIds: active
                                                        ? studentForm.subjectIds.filter((id) => id !== subject.id)
                                                        : [...studentForm.subjectIds, subject.id],
                                                })
                                            }
                                        >
                                            {subject.name}
                                        </button>
                                    );
                                })}
                            </div>
                        </Field>
                    </div>
                </Modal>
            )}

            {importOpen && (
                <Modal
                    title="批量导入学生"
                    subtitle="一行一个，有表头就按表头认列（学号 / 姓名 / 年级 / 备注）；也可以直接从表格复制粘贴"
                    wide
                    onClose={() => {
                        setImportOpen(false);
                        setPreview(null);
                    }}
                    footer={
                        <>
                            <button
                                className="btn ghost"
                                onClick={() => {
                                    setImportOpen(false);
                                    setPreview(null);
                                }}
                            >
                                取消
                            </button>
                            <button className="btn ghost" onClick={previewImport} disabled={!importText.trim()}>
                                预览
                            </button>
                            <button
                                className="btn primary"
                                onClick={commitImport}
                                disabled={!preview || preview.rows.length === 0}
                            >
                                导入 {preview ? `${preview.total - preview.duplicates} 人` : ''}
                            </button>
                        </>
                    }
                >
                    <textarea
                        className="teach-import"
                        rows={6}
                        placeholder={'学号,姓名,年级,备注\n2026001,张三,七年级,数学课代表\n2026002,李四,七年级'}
                        value={importText}
                        onChange={(event) => {
                            setImportText(event.target.value);
                            setPreview(null);
                        }}
                    />
                    {preview && (
                        <>
                            <p className="teach-import-note">
                                读到 {preview.total} 行，其中 {preview.duplicates} 行与已有名单重复
                                {preview.conflicts > 0 && `，${preview.conflicts} 行学号已被占用`}
                                （都会跳过，其余照常导入）。
                            </p>
                            <div className="teach-import-list">
                                {preview.rows.map((row, index) => (
                                    <div
                                        key={`${row.name}-${index}`}
                                        className={`teach-import-row ${row.duplicate || row.conflict ? 'dup' : ''}`}
                                    >
                                        <span>{row.studentNo || '自动'}</span>
                                        <span>{row.name}</span>
                                        <span>{row.grade || '—'}</span>
                                        <span>{row.note || '—'}</span>
                                        <span>
                                            {row.duplicate ? '已存在，跳过' : row.conflict ? '学号冲突' : '新增'}
                                        </span>
                                    </div>
                                ))}
                            </div>
                        </>
                    )}
                </Modal>
            )}

            {detail && (
                <Drawer
                    title={detail.name}
                    subtitle={`${detail.subjectName || '未设学科'} · ${detail.term || '未填学期'} · ${members.length} 人`}
                    onClose={() => setDetail(null)}
                    footer={
                        <>
                            <button className="btn ghost" onClick={() => setImportOpen(true)}>
                                导入学生
                            </button>
                            <button className="btn ghost" onClick={openAdd}>
                                从名单添加
                            </button>
                            <button className="btn primary" onClick={() => onGo(`assignments/${detail.id}`)}>
                                去布置作业
                            </button>
                        </>
                    }
                >
                    <div className="teach-detail-bar">
                        <button className="teach-code inline" onClick={() => copy(detail.joinCode)}>
                            邀请码 <code>{detail.joinCode}</code>
                        </button>
                        <button className="btn ghost small" onClick={() => regenerate(detail)}>
                            重置
                        </button>
                    </div>
                    <input
                        className="teach-search"
                        placeholder="在班里搜学生"
                        value={memberKeyword}
                        onChange={(event) => setMemberKeyword(event.target.value)}
                    />
                    {visibleMembers.length === 0 ? (
                        <Empty
                            title="班里还没有学生"
                            hint="从名单里挑，或者先批量导入一份名单。"
                            action={
                                <button className="btn ghost" onClick={openAdd}>
                                    添加学生
                                </button>
                            }
                        />
                    ) : (
                        <div className="teach-members">
                            {visibleMembers.map((row) => (
                                <div key={row.id} className="teach-member">
                                    <Avatar name={row.name}/>
                                    <span className="teach-member-main">
                                        <strong>{row.name}</strong>
                                        <small>
                                            {row.grade || '未填年级'}
                                            {row.subjectNames.length > 0 && ` · ${row.subjectNames.join('、')}`}
                                        </small>
                                    </span>
                                    <span className="teach-member-count">{row.attemptCount} 次作答</span>
                                    <button className="btn ghost small" onClick={() => removeMember(row)}>
                                        移出
                                    </button>
                                </div>
                            ))}
                        </div>
                    )}
                </Drawer>
            )}

            {addOpen && detail && (
                <Modal
                    title="把学生加进班级"
                    subtitle="只列出还没在班里的学生"
                    onClose={() => {
                        setAddOpen(false);
                        setPicked([]);
                    }}
                    footer={
                        <>
                            <button
                                className="btn ghost"
                                onClick={() => {
                                    setAddOpen(false);
                                    setPicked([]);
                                }}
                            >
                                取消
                            </button>
                            <button className="btn primary" onClick={addMembers} disabled={picked.length === 0}>
                                加入 {picked.length || ''} 人
                            </button>
                        </>
                    }
                >
                    <input
                        className="teach-search"
                        placeholder="搜姓名"
                        value={memberKeyword}
                        onChange={(event) => setMemberKeyword(event.target.value)}
                    />
                    {candidates.length === 0 ? (
                        <p className="muted teach-add-empty">
                            名单里没有可加的学生了。先到「学生」标签页批量导入一份名单。
                        </p>
                    ) : (
                        <div className="teach-members">
                            {candidates.map((row) => {
                                const active = picked.includes(row.id);
                                return (
                                    <label key={row.id} className={`teach-member pick ${active ? 'on' : ''}`}>
                                        <input
                                            type="checkbox"
                                            checked={active}
                                            onChange={() =>
                                                setPicked(
                                                    active
                                                        ? picked.filter((id) => id !== row.id)
                                                        : [...picked, row.id],
                                                )
                                            }
                                        />
                                        <Avatar name={row.name}/>
                                        <span className="teach-member-main">
                                            <strong>{row.name}</strong>
                                            <small>{row.grade || '未填年级'}</small>
                                        </span>
                                    </label>
                                );
                            })}
                        </div>
                    )}
                </Modal>
            )}
        </div>
    );
}
