import {useCallback, useEffect, useMemo, useRef, useState} from 'react';
import {open} from '@tauri-apps/plugin-dialog';
import {listen} from '@tauri-apps/api/event';
import {
    assignmentGet,
    assignmentList,
    attemptBatchGrade,
    attemptSheet,
    classList,
    gradingPaperSubmit,
    gradingPhotoAdd,
    gradingPhotoAttach,
    gradingPhotoCrop,
    gradingPhotoData,
    gradingPhotoList,
    gradingPhotoRegions,
    gradingPhotoRemove,
    gradingTranscribe,
    photoUploadStart,
    photoUploadStatus,
    photoUploadStop,
    fileOpen,
} from '../lib/api.js';
import {Rich, RichInline} from '../lib/Math.jsx';
import {typeLabel} from '../lib/format.js';
import {
    Card,
    Empty,
    Field,
    Modal,
    PageHead,
    Pill,
    confirmDialog,
    notify,
} from '../components/ui.jsx';
import ReviewSheetView from '../components/ReviewSheet.jsx';

const IMAGE_EXTS = ['jpg', 'jpeg', 'png', 'webp', 'gif'];

function scoreText(value) {
    const number = Number(value || 0);
    return Number.isInteger(number) ? String(number) : number.toFixed(1);
}

/** 一张照片的懒加载预览（后端给 data URL）。 */
function PhotoThumb({asset, onClick, onRemove, label, active}) {
    const [url, setUrl] = useState('');
    useEffect(() => {
        let alive = true;
        setUrl('');
        gradingPhotoData(asset.id)
            .then((data) => {
                if (alive) setUrl(data);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, [asset.id]);

    return (
        <figure className={`gp-thumb ${active ? 'on' : ''}`}>
            <button className="gp-thumb-img" onClick={onClick} title={label || '看大图'}>
                {url ? <img src={url} alt={label || '作答照片'}/> : <span className="gp-thumb-loading">读图…</span>}
            </button>
            <figcaption>
                <span>{label || (asset.kind === 'PAGE' ? '整页' : '题块')}</span>
                {onRemove && (
                    <button className="icon-btn tiny" onClick={onRemove} aria-label="删掉这张">
                        ✕
                    </button>
                )}
            </figcaption>
        </figure>
    );
}

/** 大图查看。 */
function PhotoViewer({asset, onClose}) {
    const [url, setUrl] = useState('');
    useEffect(() => {
        let alive = true;
        gradingPhotoData(asset.id)
            .then((data) => {
                if (alive) setUrl(data);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, [asset.id]);
    return (
        <Modal title="作答照片" onClose={onClose} wide>
            {url ? <img className="gp-viewer" src={url} alt="作答照片"/> : <p className="muted">读图…</p>}
        </Modal>
    );
}

/**
 * 切题对话框：在一张整页照片上找出题块。
 * 自动检测只给候选，最终挂哪道题由老师定。
 */
function CropDialog({asset, attemptIds, onClose, onDone}) {
    const [regions, setRegions] = useState(null);
    const [picked, setPicked] = useState(null);
    const [target, setTarget] = useState(attemptIds[0] || '');
    const [busy, setBusy] = useState(false);
    const [pageUrl, setPageUrl] = useState('');
    const [pageSize, setPageSize] = useState({width: 0, height: 0});
    const [drag, setDrag] = useState(null);

    useEffect(() => {
        let alive = true;
        gradingPhotoRegions(asset.id)
            .then((list) => {
                if (alive) setRegions(list);
            })
            .catch((error) => {
                notify.error(error.message);
            });
        gradingPhotoData(asset.id)
            .then((data) => {
                if (alive) setPageUrl(data);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, [asset.id]);

    const cut = async () => {
        if (!picked) return;
        setBusy(true);
        try {
            const block = await gradingPhotoCrop({
                pageAssetId: asset.id,
                region: picked,
                attemptId: target || null,
            });
            notify.success('切出一块，已挂到选中的题');
            onDone(block);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    // 手动框选：在整页图上拖一个矩形。自动检测只给候选，手写卷子常常要自己框。
    const startDrag = (event) => {
        const box = event.currentTarget.getBoundingClientRect();
        setDrag({
            box,
            x0: event.clientX,
            y0: event.clientY,
            x1: event.clientX,
            y1: event.clientY,
        });
    };
    const moveDrag = (event) => {
        if (!drag) return;
        setDrag({...drag, x1: event.clientX, y1: event.clientY});
    };
    const endDrag = () => {
        if (!drag || !pageSize.width) return;
        const left = Math.min(drag.x0, drag.x1);
        const top = Math.min(drag.y0, drag.y1);
        const width = Math.abs(drag.x1 - drag.x0);
        const height = Math.abs(drag.y1 - drag.y0);
        setDrag(null);
        if (width < 12 || height < 12) return;
        setPicked({
            x: Math.max(0, Math.round(((left - drag.box.left) / drag.box.width) * pageSize.width)),
            y: Math.max(0, Math.round(((top - drag.box.top) / drag.box.height) * pageSize.height)),
            w: Math.max(8, Math.round((width / drag.box.width) * pageSize.width)),
            h: Math.max(8, Math.round((height / drag.box.height) * pageSize.height)),
        });
    };
    const boxStyle = (region) => ({
        left: `${(region.x / pageSize.width) * 100}%`,
        top: `${(region.y / pageSize.height) * 100}%`,
        width: `${(region.w / pageSize.width) * 100}%`,
        height: `${(region.h / pageSize.height) * 100}%`,
    });
    const dragStyle = drag && pageSize.width
        ? {
            left: `${((Math.min(drag.x0, drag.x1) - drag.box.left) / drag.box.width) * 100}%`,
            top: `${((Math.min(drag.y0, drag.y1) - drag.box.top) / drag.box.height) * 100}%`,
            width: `${(Math.abs(drag.x1 - drag.x0) / drag.box.width) * 100}%`,
            height: `${(Math.abs(drag.y1 - drag.y0) / drag.box.height) * 100}%`,
        }
        : null;

    return (
        <Modal
            title="切题"
            subtitle="点蓝框选中一块，或者直接在图上拖一个框；切完可以再挂到别的题"
            onClose={onClose}
            wide
            footer={
                <>
                    <button className="btn ghost" onClick={onClose}>
                        取消
                    </button>
                    <button className="btn primary" onClick={cut} disabled={!picked || busy}>
                        切出这一块
                    </button>
                </>
            }
        >
            {regions === null ? (
                <p className="muted">正在找题块…</p>
            ) : !pageUrl ? (
                <p className="muted">读图…</p>
            ) : (
                <>
                    <p className="muted gp-hint">
                        自动找出了 {regions.length} 块。点蓝框选中——切错地方就自己拖一个框。
                    </p>
                    <div
                        className="gp-canvas"
                        onMouseDown={startDrag}
                        onMouseMove={moveDrag}
                        onMouseUp={endDrag}
                        onMouseLeave={endDrag}
                    >
                        <img
                            src={pageUrl}
                            alt="整页作答照片"
                            onLoad={(event) =>
                                setPageSize({
                                    width: event.target.naturalWidth || 1,
                                    height: event.target.naturalHeight || 1,
                                })
                            }
                        />
                        {regions.map((region, index) => (
                            <button
                                key={`${region.x}-${region.y}-${index}`}
                                className={`gp-box ${picked === region ? 'on' : ''}`}
                                style={boxStyle(region)}
                                hidden={
                                    !pageSize.width ||
                                    region.x + region.w > pageSize.width + 4 ||
                                    region.y + region.h > pageSize.height + 4
                                }
                                onMouseDown={(event) => event.stopPropagation()}
                                onClick={() => setPicked(region)}
                                title={`第 ${index + 1} 块`}
                            >
                                <span>{index + 1}</span>
                            </button>
                        ))}
                        {dragStyle && <div className="gp-box dragging" style={dragStyle}/>}
                    </div>
                    <Field label="挂到第几题" hint="留空就是先切下来，等会儿再挂">
                        <select value={target} onChange={(event) => setTarget(event.target.value)}>
                            <option value="">先不挂</option>
                            {attemptIds.map((id, index) => (
                                <option key={id} value={id}>
                                    第 {index + 1} 题
                                </option>
                            ))}
                        </select>
                    </Field>
                </>
            )}
        </Modal>
    );
}

/** 抄录建议卡片：模型给的建议，要老师点了才算数。 */
function SuggestionCard({suggestion, onApply, onDismiss}) {
    if (!suggestion) return null;
    return (
        <div className="gp-suggest">
            <div className="gp-suggest-head">
                <Pill tone={suggestion.confidence === 'high' ? 'ok' : 'muted'}>
                    {suggestion.confidence === 'high' ? '看得清' : '看不太清'}
                </Pill>
                <span className="muted">AI 抄录建议 · 确认后才算数</span>
            </div>
            {suggestion.answer ? (
                <div className="teach-answer">
                    <span className="teach-answer-label">抄到的答案</span>
                    <span><RichInline text={suggestion.answer}/></span>
                </div>
            ) : (
                <p className="muted">没抄到内容。</p>
            )}
            {suggestion.warning && <p className="gp-warn">{suggestion.warning}</p>}
            {suggestion.remark && (
                <div className="teach-answer ref">
                    <span className="teach-answer-label">建议评语</span>
                    <span><RichInline text={suggestion.remark}/></span>
                </div>
            )}
            <div className="gp-suggest-actions">
                <button className="btn small primary" onClick={onApply}>
                    用这个建议
                </button>
                <button className="btn small ghost" onClick={onDismiss}>
                    不用
                </button>
            </div>
        </div>
    );
}

/** 手机扫码上传弹窗：二维码 + 地址 + 已收张数轮询 + 停止。二维码 SVG 由后端生成。 */
function PhoneUploadModal({info, studentName, onClose, onStop}) {
    const [status, setStatus] = useState(null);
    useEffect(() => {
        let alive = true;
        const tick = async () => {
            try {
                const next = await photoUploadStatus();
                if (alive) setStatus(next);
            } catch {
                /* 服务已关就停在此刻的状态 */
            }
        };
        tick();
        const timer = setInterval(tick, 2000);
        return () => {
            alive = false;
            clearInterval(timer);
        };
    }, []);

    // 后端收到新照片时主组件会刷新照片墙，这里同步刷新计数。
    useEffect(() => {
        const refresh = () => photoUploadStatus().then(setStatus).catch(() => {});
        let cancelled = false;
        let unlisten;
        listen('nb://grading-photo-arrived', refresh).then((fn) => {
            if (cancelled) fn();
            else unlisten = fn;
        });
        return () => {
            cancelled = true;
            if (unlisten) unlisten();
        };
    }, []);

    const qrSrc = useMemo(
        () => `data:image/svg+xml;charset=utf-8,${encodeURIComponent(info.qrSvg)}`,
        [info.qrSvg],
    );
    const count = status?.count ?? 0;
    const max = info.maxPhotos;
    const left = status?.secondsLeft ?? info.expiresInSecs;
    const minutes = Math.max(0, Math.floor(left / 60));
    const seconds = String(Math.max(0, left % 60)).padStart(2, '0');

    const copyUrl = async () => {
        try {
            await navigator.clipboard.writeText(info.url);
            notify.success('上传地址已复制');
        } catch {
            notify.error('复制失败，请手动选中地址复制');
        }
    };

    return (
        <Modal
            title="手机扫码上传"
            subtitle={`给「${studentName || '当前学生'}」上传作答照片 · 手机需和这台电脑连同一个 WiFi`}
            onClose={onClose}
            footer={
                <>
                    <button className="btn ghost" onClick={copyUrl}>
                        复制地址
                    </button>
                    <button
                        className="btn primary"
                        onClick={() => {
                            onStop();
                            onClose();
                        }}
                    >
                        停止上传
                    </button>
                </>
            }
        >
            <div className="pu-wrap">
                <div className="pu-qr">
                    <img src={qrSrc} alt="扫码打开上传页" />
                </div>
                <div className="pu-side">
                    <p className="pu-step"><b>1</b> 手机相机扫上面的二维码</p>
                    <p className="pu-step"><b>2</b> 拍照或从相册选照片，可以一次传多张</p>
                    <p className="pu-step"><b>3</b> 传完点「停止上传」，照片已自动进批改台</p>
                    <div className="pu-url" title={info.url}>
                        {info.url}
                    </div>
                    <div className="pu-status">
                        <span>已收 <b>{count}</b> / {max} 张</span>
                        <span className={left < 60 ? 'pu-time warn' : 'pu-time'}>
                            剩余 {minutes}:{seconds}
                        </span>
                    </div>
                    <p className="pu-note">
                        入口 15 分钟后自动失效；照片只发到这台电脑，不经过外网。扫不出来可点「复制地址」用手机浏览器打开。
                    </p>
                </div>
            </div>
        </Modal>
    );
}

/**
 * 批改台：把「拍照 → 切题 → 逐题给分」串成一条线。
 *
 * 顶部是待批队列（上一份 / 下一份 / 待批 N），左边是照片，右边是逐题给分。
 */
export default function Grading({onGo}) {
    const [classes, setClasses] = useState([]);
    const [classId, setClassId] = useState('');
    const [assignments, setAssignments] = useState([]);
    const [assignmentId, setAssignmentId] = useState('');
    const [detail, setDetail] = useState(null);
    const [sheet, setSheet] = useState(null);
    const [photos, setPhotos] = useState([]);
    const [drafts, setDrafts] = useState({});
    const [suggestions, setSuggestions] = useState({});
    const [busy, setBusy] = useState(false);
    const [viewer, setViewer] = useState(null);
    const [cropping, setCropping] = useState(null);
    const [phoneUpload, setPhoneUpload] = useState(null);
    const [review, setReview] = useState(null);
    const listRef = useRef(null);

    const loadClasses = useCallback(async () => {
        try {
            const rows = await classList();
            setClasses(rows);
            if (rows.length && !classId) setClassId(rows[0].id);
        } catch (error) {
            notify.error(error.message);
        }
    }, [classId]);

    useEffect(() => {
        loadClasses();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    useEffect(() => {
        if (!classId) {
            setAssignments([]);
            return;
        }
        assignmentList(classId)
            .then((rows) => {
                setAssignments(rows);
                if (rows.length) setAssignmentId(rows[0].id);
                else setAssignmentId('');
            })
            .catch((error) => notify.error(error.message));
    }, [classId]);

    const loadDetail = useCallback(async (id) => {
        if (!id) {
            setDetail(null);
            return;
        }
        try {
            setDetail(await assignmentGet(id));
        } catch (error) {
            notify.error(error.message);
        }
    }, []);

    useEffect(() => {
        loadDetail(assignmentId);
    }, [assignmentId, loadDetail]);

    const openSheet = useCallback(
        async (studentId) => {
            if (!assignmentId) return;
            try {
                let data = await attemptSheet(assignmentId, studentId);
                // 纸质作业：学生没在线交卷时先建一次空提交，照片才有地方挂。
                if (!data.items.length) {
                    await gradingPaperSubmit(assignmentId, studentId);
                    data = await attemptSheet(assignmentId, studentId);
                }
                setSheet(data);
                const next = {};
                data.items.forEach((item) => {
                    next[item.attemptId] = {
                        score: item.score ?? 0,
                        remark: item.remark || '',
                    };
                });
                setDrafts(next);
                setSuggestions({});
                const list = await gradingPhotoList(studentId, assignmentId);
                setPhotos(list);
            } catch (error) {
                notify.error(error.message);
            }
        },
        [assignmentId],
    );

    const reloadPhotos = useCallback(async () => {
        if (!sheet || !assignmentId) return;
        try {
            setPhotos(await gradingPhotoList(sheet.student.id, assignmentId));
        } catch (error) {
            notify.error(error.message);
        }
    }, [sheet, assignmentId]);

    // 手机上传的照片一到，自动刷新当前学生的照片墙。
    useEffect(() => {
        let cancelled = false;
        let unlisten;
        listen('nb://grading-photo-arrived', (event) => {
            const payload = event.payload || {};
            if (!sheet || payload.studentId !== sheet.student.id) return;
            gradingPhotoList(sheet.student.id, assignmentId)
                .then((list) => {
                    if (!cancelled) setPhotos(list);
                })
                .catch(() => {});
        }).then((fn) => {
            if (cancelled) fn();
            else unlisten = fn;
        });
        return () => {
            cancelled = true;
            if (unlisten) unlisten();
        };
    }, [sheet, assignmentId]);

    // 离开批改台就关掉临时上传服务，不留后台端口。
    useEffect(() => () => {
        photoUploadStop().catch(() => {});
    }, []);

    const startPhoneUpload = async () => {
        if (!sheet) return;
        try {
            setBusy(true);
            const info = await photoUploadStart(sheet.student.id, assignmentId || null);
            setPhoneUpload(info);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const stopPhoneUpload = () => {
        photoUploadStop().catch(() => {});
    };

    const importPhotos = async () => {
        if (!sheet) return;
        try {
            const picked = await open({
                multiple: true,
                filters: [{name: '照片', extensions: IMAGE_EXTS}],
            });
            if (!picked) return;
            const paths = Array.isArray(picked) ? picked : [picked];
            setBusy(true);
            let ok = 0;
            for (const path of paths) {
                // 本地文件走采集台那条读取链路（顺带把 PDF/Word 挡在外面）。
                // eslint-disable-next-line no-await-in-loop
                const file = await fileOpen(path);
                if (!file || file.kind !== 'image') {
                    notify.error(`${path.split('/').pop()} 不是图片`);
                    continue;
                }
                // eslint-disable-next-line no-await-in-loop
                await gradingPhotoAdd({
                    studentId: sheet.student.id,
                    assignmentId,
                    dataUrl: file.image.dataUrl,
                });
                ok += 1;
            }
            await reloadPhotos();
            notify.success(`导入 ${ok} 张整页照片`);
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const removePhoto = async (asset) => {
        const ok = await confirmDialog({
            title: '删掉照片',
            message: '删掉这张作答照片？原图还在磁盘上，但界面上看不到了。',
            confirmText: '删掉',
            danger: true,
        });
        if (!ok) return;
        try {
            await gradingPhotoRemove(asset.id);
            await reloadPhotos();
        } catch (error) {
            notify.error(error.message);
        }
    };

    const attachTo = async (asset, attemptId) => {
        try {
            await gradingPhotoAttach(asset.id, attemptId);
            await reloadPhotos();
            await openSheet(sheet.student.id);
            notify.success('已挂到这道题');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const transcribe = async (asset, attemptId) => {
        setBusy(true);
        try {
            const suggestion = await gradingTranscribe(asset.id, attemptId);
            setSuggestions((current) => ({...current, [attemptId]: suggestion}));
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const applySuggestion = (attemptId) => {
        const suggestion = suggestions[attemptId];
        if (!suggestion) return;
        setDrafts((current) => ({
            ...current,
            [attemptId]: {
                score: suggestion.score ?? current[attemptId]?.score ?? 0,
                remark: suggestion.remark || current[attemptId]?.remark || '',
            },
        }));
        setSuggestions((current) => {
            const next = {...current};
            delete next[attemptId];
            return next;
        });
        notify.success('已用上建议，可以再改');
    };

    const saveGrades = async () => {
        if (!sheet) return;
        setBusy(true);
        try {
            const items = sheet.items.map((item) => ({
                attemptId: item.attemptId,
                score: Number(drafts[item.attemptId]?.score ?? 0),
                remark: drafts[item.attemptId]?.remark || '',
            }));
            await attemptBatchGrade(items);
            await loadDetail(assignmentId);
            notify.success('批改已保存，学情同步更新');
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    // 待批队列：没交卷的排前面，已批的排后面。
    const queue = useMemo(() => {
        if (!detail) return [];
        const rows = detail.submissions || [];
        return [...rows].sort((a, b) => {
            const ap = a.pending > 0 ? 0 : 1;
            const bp = b.pending > 0 ? 0 : 1;
            if (ap !== bp) return ap - bp;
            return (a.studentName || '').localeCompare(b.studentName || '', 'zh');
        });
    }, [detail]);

    const pendingCount = useMemo(
        () => (detail?.submissions || []).filter((row) => row.pending > 0).length,
        [detail],
    );

    const currentIndex = queue.findIndex((row) => row.studentId === sheet?.student?.id);
    const goStep = (delta) => {
        if (!queue.length) return;
        const next = (currentIndex + delta + queue.length) % queue.length;
        const target = queue[next];
        if (target) openSheet(target.studentId);
    };

    const pagePhotos = photos.filter((photo) => photo.kind === 'PAGE');
    const blockPhotos = photos.filter((photo) => photo.kind === 'QUESTION');
    const attemptIds = sheet ? sheet.items.map((item) => item.attemptId) : [];

    if (!classes.length) {
        return (
            <div className="page">
                <PageHead title="批改台" subtitle="拍照批改：纸质卷子拍成照片，逐题给分"/>
                <Empty title="还没有班级" hint="批改要先有班级和作业。">
                    <button className="btn primary" onClick={() => onGo('classes')}>
                        去班级
                    </button>
                </Empty>
            </div>
        );
    }

    return (
        <div className="page">
            <PageHead
                title="批改台"
                subtitle="纸质作业拍照 → 切题 → 逐题给分。AI 只给抄录建议，分数由你定。"
                help="grading"
                actions={
                    <button className="btn ghost" onClick={() => onGo('classes')}>
                        返回班级
                    </button>
                }
            />

            <div className="gp-toolbar">
                <Field label="班级">
                    <select value={classId} onChange={(event) => setClassId(event.target.value)}>
                        {classes.map((row) => (
                            <option key={row.id} value={row.id}>
                                {row.name}
                            </option>
                        ))}
                    </select>
                </Field>
                <Field label="作业">
                    <select
                        value={assignmentId}
                        onChange={(event) => setAssignmentId(event.target.value)}
                    >
                        <option value="">选择作业</option>
                        {assignments.map((row) => (
                            <option key={row.id} value={row.id}>
                                {row.title}
                            </option>
                        ))}
                    </select>
                </Field>
                {detail && (
                    <div className="gp-queue-stat">
                        <strong>{pendingCount}</strong>
                        <span>份待批</span>
                    </div>
                )}
                {detail && (
                    <button
                        className="btn ghost small gp-review-btn"
                        onClick={() => setReview(detail.assignment)}
                    >
                        讲评清单
                    </button>
                )}
            </div>

            {!assignmentId ? (
                <Empty title="先选一份作业" hint="批改台按作业组织，纸质卷子也要对应到作业里。"/>
            ) : (
                <div className="gp-layout">
                    {/* 待批队列 */}
                    <aside className="gp-queue">
                        <div className="gp-queue-head">
                            <span>待批队列</span>
                            <span className="muted">{queue.length} 人</span>
                        </div>
                        <div className="gp-queue-list" ref={listRef}>
                            {queue.map((row) => (
                                <button
                                    key={row.studentId}
                                    className={`gp-queue-item ${sheet?.student?.id === row.studentId ? 'on' : ''}`}
                                    onClick={() => openSheet(row.studentId)}
                                >
                                    <span className="gp-queue-name">{row.studentName}</span>
                                    <span className="gp-queue-meta">
                                        {row.pending > 0 ? (
                                            <Pill tone="warn">待批 {row.pending}</Pill>
                                        ) : row.submissionId ? (
                                            <Pill tone="ok">已批</Pill>
                                        ) : (
                                            <Pill tone="muted">未交</Pill>
                                        )}
                                        {row.photoCount > 0 && (
                                            <span className="gp-queue-photos">📷 {row.photoCount}</span>
                                        )}
                                    </span>
                                </button>
                            ))}
                            {queue.length === 0 && <p className="muted gp-empty-line">这个班还没有学生。</p>}
                        </div>
                    </aside>

                    {/* 批改主区 */}
                    <section className="gp-main">
                        {!sheet ? (
                            <Empty title="选一个学生开始批" hint="左边队列点名字，照片和逐题给分都在这里。"/>
                        ) : (
                            <>
                                <div className="gp-sheet-head">
                                    <div>
                                        <h3>{sheet.student.name}</h3>
                                        <p className="muted">
                                            {sheet.submittedAt
                                                ? `提交于 ${sheet.submittedAt.replace('T', ' ').slice(0, 16)}${sheet.late ? '（迟交）' : ''}`
                                                : '纸质作业 · 尚未在线提交'}
                                        </p>
                                    </div>
                                    <div className="gp-sheet-nav">
                                        <button className="btn small ghost" onClick={() => goStep(-1)}>
                                            上一份
                                        </button>
                                        <span className="muted">
                                            {currentIndex + 1}/{queue.length}
                                        </span>
                                        <button className="btn small ghost" onClick={() => goStep(1)}>
                                            下一份
                                        </button>
                                    </div>
                                </div>

                                <div className="gp-columns">
                                    {/* 左：照片 */}
                                    <div className="gp-photos">
                                        <div className="gp-photos-head">
                                            <h4>作答照片</h4>
                                            <div className="gp-photo-actions">
                                                <button className="btn small ghost" onClick={startPhoneUpload} disabled={busy}>
                                                    手机上传
                                                </button>
                                                <button className="btn small primary" onClick={importPhotos} disabled={busy}>
                                                    导入照片
                                                </button>
                                            </div>
                                        </div>

                                        {pagePhotos.length === 0 && blockPhotos.length === 0 ? (
                                            <Empty
                                                title="还没有照片"
                                                hint="拍好的卷子照片导入这里，再切成一块块挂到题上。"
                                            />
                                        ) : (
                                            <>
                                                {pagePhotos.length > 0 && (
                                                    <div className="gp-group">
                                                        <span className="gp-group-label">整页</span>
                                                        <div className="gp-thumbs">
                                                            {pagePhotos.map((asset) => (
                                                                <div key={asset.id} className="gp-thumb-wrap">
                                                                    <PhotoThumb
                                                                        asset={asset}
                                                                        label="整页"
                                                                        onClick={() => setViewer(asset)}
                                                                    />
                                                                    <button
                                                                        className="btn tiny ghost gp-cut"
                                                                        onClick={() => setCropping(asset)}
                                                                    >
                                                                        切题
                                                                    </button>
                                                                </div>
                                                            ))}
                                                        </div>
                                                    </div>
                                                )}
                                                {blockPhotos.length > 0 && (
                                                    <div className="gp-group">
                                                        <span className="gp-group-label">
                                                            切出的题块
                                                            {blockPhotos.some((photo) => !photo.attemptId) && (
                                                                <span className="gp-group-warn">（还有没挂题的）</span>
                                                            )}
                                                        </span>
                                                        <div className="gp-thumbs">
                                                            {blockPhotos.map((asset) => (
                                                                <div key={asset.id} className="gp-thumb-wrap">
                                                                    <PhotoThumb
                                                                        asset={asset}
                                                                        label={
                                                                            asset.attemptId
                                                                                ? `第 ${attemptIds.indexOf(asset.attemptId) + 1} 题`
                                                                                : '未挂题'
                                                                        }
                                                                        onClick={() => setViewer(asset)}
                                                                        onRemove={() => removePhoto(asset)}
                                                                    />
                                                                    {!asset.attemptId && sheet.items.length > 0 && (
                                                                        <select
                                                                            className="gp-attach-select"
                                                                            defaultValue=""
                                                    onChange={(event) => {
                                                        if (!event.target.value) return;
                                                        attachTo(asset, event.target.value);
                                                    }}
                                                                        >
                                                                            <option value="">挂到…</option>
                                                                            {sheet.items.map((item, index) => (
                                                                                <option key={item.attemptId} value={item.attemptId}>
                                                                                    第 {index + 1} 题
                                                                                </option>
                                                                            ))}
                                                                        </select>
                                                                    )}
                                                                </div>
                                                            ))}
                                                        </div>
                                                    </div>
                                                )}
                                            </>
                                        )}
                                    </div>

                                    {/* 右：逐题给分 */}
                                    <div className="gp-items">
                                        {sheet.items.length === 0 ? (
                                            <Empty title="这份作业没有题" hint="先在作业里选题再布置。"/>
                                        ) : (
                                            sheet.items.map((item, index) => {
                                                const draft = drafts[item.attemptId] || {score: 0, remark: ''};
                                                const objective = item.correct !== null;
                                                const itemPhotos = item.photos || [];
                                                return (
                                                    <section key={item.attemptId} className="gp-item">
                                                        <header>
                                                            <span className="row-no">第 {index + 1} 题</span>
                                                            <Pill tone="line">{typeLabel(item.question.type)}</Pill>
                                                            <span className="muted">
                                                                {scoreText(draft.score)} / {scoreText(item.maxScore)} 分
                                                            </span>
                                                            {objective && (
                                                                <Pill tone={item.correct ? 'ok' : 'danger'}>
                                                                    {item.correct ? '自动判对' : '自动判错'}
                                                                </Pill>
                                                            )}
                                                        </header>
                                                        <Rich text={item.question.stem} className="gp-stem"/>

                                                        {itemPhotos.length > 0 && (
                                                            <div className="gp-item-photos">
                                                                {itemPhotos.map((photo) => (
                                                                    <PhotoThumb
                                                                        key={photo.id}
                                                                        asset={photo}
                                                                        label="作答"
                                                                        onClick={() => setViewer(photo)}
                                                                    />
                                                                ))}
                                                            </div>
                                                        )}

                                                        <div className="teach-answer">
                                                            <span className="teach-answer-label">学生答案</span>
                                                            <span className={item.answer ? '' : 'muted'}>
                                                                {item.answer
                                                                    ? <RichInline text={item.answer}/>
                                                                    : '（纸笔作答，看照片）'}
                                                            </span>
                                                        </div>

                                                        {/* 参考答案不分题型都给：客观题也要老师对着照片核一眼，
                                                            以前只有主观题显示，纸质卷子里的选择/填空看不到答案。 */}
                                                        <div className="teach-answer ref">
                                                            <span className="teach-answer-label">参考答案</span>
                                                            <span>
                                                                {item.question.answer
                                                                    ? <RichInline text={item.question.answer}/>
                                                                    : '（没填参考答案）'}
                                                            </span>
                                                        </div>

                                                        {itemPhotos.length > 0 && (
                                                            <div className="gp-ocr-actions">
                                                                {itemPhotos.map((photo) => (
                                                                    <button
                                                                        key={photo.id}
                                                                        className="btn small ghost"
                                                                        onClick={() => transcribe(photo, item.attemptId)}
                                                                        disabled={busy}
                                                                    >
                                                                        AI 抄录这一张
                                                                    </button>
                                                                ))}
                                                            </div>
                                                        )}
                                                        <SuggestionCard
                                                            suggestion={suggestions[item.attemptId]}
                                                            onApply={() => applySuggestion(item.attemptId)}
                                                            onDismiss={() =>
                                                                setSuggestions((current) => {
                                                                    const next = {...current};
                                                                    delete next[item.attemptId];
                                                                    return next;
                                                                })
                                                            }
                                                        />

                                                        <div className="teach-grade-form">
                                                            <Field label="得分">
                                                                <input
                                                                    type="number"
                                                                    min="0"
                                                                    max={item.maxScore}
                                                                    step="0.5"
                                                                    value={draft.score}
                                                                    onChange={(event) =>
                                                                        setDrafts({
                                                                            ...drafts,
                                                                            [item.attemptId]: {
                                                                                ...draft,
                                                                                score: event.target.value,
                                                                            },
                                                                        })
                                                                    }
                                                                />
                                                            </Field>
                                                            <Field label="评语" wide>
                                                                <input
                                                                    value={draft.remark}
                                                                    placeholder="可选，只存在这台电脑上"
                                                                    onChange={(event) =>
                                                                        setDrafts({
                                                                            ...drafts,
                                                                            [item.attemptId]: {
                                                                                ...draft,
                                                                                remark: event.target.value,
                                                                            },
                                                                        })
                                                                    }
                                                                />
                                                            </Field>
                                                        </div>
                                                    </section>
                                                );
                                            })
                                        )}
                                    </div>
                                </div>

                                <div className="gp-save-bar">
                                    <div className="gp-save-total">
                                        合计{' '}
                                        <strong>
                                            {scoreText(
                                                sheet.items.reduce(
                                                    (sum, item) =>
                                                        sum + (Number(drafts[item.attemptId]?.score) || 0),
                                                    0,
                                                ),
                                            )}
                                        </strong>{' '}
                                        / {scoreText(sheet.assignment.totalScore)}
                                    </div>
                                    <button
                                        className="btn primary"
                                        onClick={saveGrades}
                                        disabled={busy || sheet.items.length === 0}
                                    >
                                        保存批改
                                    </button>
                                </div>
                            </>
                        )}
                    </section>
                </div>
            )}

            {viewer && <PhotoViewer asset={viewer} onClose={() => setViewer(null)}/>}
            {cropping && (
                <CropDialog
                    asset={cropping}
                    attemptIds={attemptIds}
                    onClose={() => setCropping(null)}
                    onDone={async () => {
                        setCropping(null);
                        await reloadPhotos();
                        if (sheet) await openSheet(sheet.student.id);
                    }}
                />
            )}
            {phoneUpload && (
                <PhoneUploadModal
                    info={phoneUpload}
                    studentName={sheet?.student?.name}
                    onClose={() => setPhoneUpload(null)}
                    onStop={stopPhoneUpload}
                />
            )}
            {review && (
                <ReviewSheetView
                    assignmentId={review.id}
                    title={review.title}
                    onClose={() => setReview(null)}
                />
            )}
        </div>
    );
}
