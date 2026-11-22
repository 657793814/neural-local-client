import {useCallback, useEffect, useMemo, useRef, useState} from 'react';
import {open} from '@tauri-apps/plugin-dialog';
import {getCurrentWebview} from '@tauri-apps/api/webview';
import {
    browserBounds,
    browserClose,
    browserExtractQuestions,
    browserHide,
    browserOpen,
    browserSetSubject,
    browserStatus,
    cancelJob,
    captureQueueAdd,
    captureQueueClearDone,
    captureQueueList,
    captureQueueRemove,
    captureQueueRetry,
    captureRegion,
    captureSourceForget,
    captureSourceRemember,
    captureSources,
    captureSourceStar,
    commitJob,
    fileOpen,
    imageFromPath,
    imageStitch,
    ocrStatus,
    saveTempImage,
    settingGet,
    settingSet,
    smartCrop,
    smartRegions,
    startJob,
    updateJobItem,
} from '../lib/api.js';
import {cx, isBlocked, readFileAsDataUrl} from '../lib/format.js';
import {JobItemCard, JobProgress, PickBar, usePollingJob} from '../components/job.jsx';
import {QuestionEditor} from '../components/QuestionEditor.jsx';
import {Card, Empty, PageHead, Pill, Tabs, notify} from '../components/ui.jsx';

// 队列缩略图缓存：同一张图只转一次 dataUrl，避免反复读盘。
const thumbCache = new Map();

function hostOf(url) {
    try {
        return new URL(url).hostname.replace(/^www\./, '');
    } catch {
        return url || '未知来源';
    }
}

function QueueThumb({path}) {
    const [dataUrl, setDataUrl] = useState(() => thumbCache.get(path) || '');
    useEffect(() => {
        if (dataUrl || !path) return undefined;
        let alive = true;
        imageFromPath(path)
            .then((result) => {
                thumbCache.set(path, result.dataUrl);
                if (alive) setDataUrl(result.dataUrl);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, [path, dataUrl]);
    if (!dataUrl) return <div className="queue-thumb empty"/>;
    return <img className="queue-thumb" src={dataUrl} alt=""/>;
}

const STATUS_TEXT = {
    PENDING: {label: '待识别', tone: 'muted'},
    RUNNING: {label: '识别中', tone: 'warn'},
    DONE: {label: '已识别', tone: 'ok'},
    FAILED: {label: '失败', tone: 'danger'},
};

const IMAGE_EXTS = ['png', 'jpg', 'jpeg', 'webp', 'gif', 'bmp', 'tif', 'tiff', 'heic', 'heif'];
const DOC_EXTS = ['docx', 'docm', 'doc', 'rtf', 'odt', 'txt', 'text', 'md', 'markdown', 'csv', 'tsv', 'html', 'htm'];
const FILE_EXTS = [...IMAGE_EXTS, 'pdf', ...DOC_EXTS];

/**
 * 采集台：原文在左、识别结果在右。
 *
 * 一条链路从「打开网站 / 打开本地 PDF」到「存进草稿箱」，中间不切窗口：
 * - 左：取景、翻页、跨页拼接、框选、智能裁剪；
 * - 右：逐题核对，缺答案的题就地改，改好才勾选。
 * 来源与连拍队列是可选的加速器，收在下面，不抢正事的位置。
 */
const ENGINES = [
    {value: 'auto', label: '自动', hint: '先本机 OCR，认不出再交给 AI 识图'},
    {value: 'local', label: '本机 OCR', hint: '离线、不花模型额度；公式图可能认不全'},
    {value: 'vision', label: 'AI 识图', hint: '走视觉模型，图片题和公式更稳'},
];

export default function Capture({
    tree,
    trigger = 0,
    externalPath = '',
    externalText = '',
    externalWebJob = null,
    onChanged,
}) {
    const [image, setImage] = useState(null);
    const [imageQueued, setImageQueued] = useState(false);
    const [burst, setBurst] = useState(true);
    const [subjectId, setSubjectId] = useState(tree[0]?.id || '');
    // 图片识别引擎：auto / local / vision。默认自动，能用本机就先用本机。
    const [engine, setEngine] = useState('auto');
    const [ocr, setOcr] = useState(null);
    const [jobId, setJobId] = useState('');
    const [items, setItems] = useState([]);
    const [selection, setSelection] = useState(() => new Set());
    const [editing, setEditing] = useState(false);
    const [committing, setCommitting] = useState(false);

    const [browserUrl, setBrowserUrl] = useState('');
    const [activeUrl, setActiveUrl] = useState('');
    const [browserOn, setBrowserOn] = useState(false);
    const [sources, setSources] = useState([]);
    // 顶部三个来源页签：取图、网页、文字。识别结果右栏常驻，切页签不丢。
    const [tab, setTab] = useState('shot');
    const [queueOpen, setQueueOpen] = useState(false);
    const [queue, setQueue] = useState([]);
    const [regions, setRegions] = useState([]);
    const [pickedRegions, setPickedRegions] = useState(() => new Set());
    const [smartBusy, setSmartBusy] = useState(false);
    const [pasteText, setPasteText] = useState('');
    const [doc, setDoc] = useState(null);
    // 跨页题：拼了哪几页、拼出来的图路径链
    const [span, setSpan] = useState(null);
    const [spanPaths, setSpanPaths] = useState([]);
    const [selecting, setSelecting] = useState(false);
    const [rect, setRect] = useState(null);
    const [zoom, setZoom] = useState(false);
    // 跨页拼接的第二步：已经取好上一页那段，等下一页的框选。
    const [spanPick, setSpanPick] = useState(null);

    // 每张图（PDF 按页）的识别结果缓存在内存里：翻回去不用重认，勾选状态也跟着回来。
    const shotCache = useRef(new Map());
    const pathKeys = useRef(new Map());
    const viewKey = useRef('');
    const smartToken = useRef(0);
    const jobRef = useRef(null);
    // 内嵌网页视图要贴住的占位框。
    const webFrame = useRef(null);

    const lastTrigger = useRef(trigger);
    const lastExternal = useRef(externalPath);
    const lastExternalText = useRef(externalText);
    const lastWebJob = useRef('');
    const busy = useRef(false);
    const urlRef = useRef('');
    const dragRef = useRef(null);

    urlRef.current = activeUrl;

    const refreshQueue = useCallback(async () => {
        try {
            setQueue(await captureQueueList());
        } catch {
            /* 队列读失败不打断主流程 */
        }
    }, []);

    const refreshSources = useCallback(async () => {
        try {
            setSources(await captureSources());
        } catch {
            /* 同上 */
        }
    }, []);

    useEffect(() => {
        refreshQueue();
        refreshSources();
        browserStatus()
            .then((status) => {
                setBrowserOn(!!status.open);
                if (status.url) {
                    setActiveUrl(status.url);
                    setBrowserUrl((current) => current || status.url);
                }
            })
            .catch(() => {});
    }, [refreshQueue, refreshSources]);

    // 本机 OCR 是否可用 + 上次选的识别引擎。没有本机能力时别把人留在「本机 OCR」。
    useEffect(() => {
        Promise.all([
            ocrStatus().catch(() => null),
            settingGet('capture.engine').catch(() => ''),
        ]).then(([status, saved]) => {
            setOcr(status);
            const available = !!status?.available;
            if (saved === 'local' && !available) {
                setEngine('vision');
            } else if (['auto', 'local', 'vision'].includes(saved)) {
                setEngine(saved);
            }
        });
    }, []);

    // 先在主窗选学科，再去网页里抓题：学科变化随时同步给注入脚本。
    useEffect(() => {
        if (!browserOn || !subjectId) return;
        browserSetSubject(subjectId).catch(() => {});
    }, [browserOn, subjectId]);

    // 内置浏览器里点「识别本页」后，Rust 建好一条已完成的 capture_web 任务。
    useEffect(() => {
        if (!externalWebJob?.jobId || externalWebJob.jobId === lastWebJob.current) return;
        lastWebJob.current = externalWebJob.jobId;
        setTab('web');
        setJobId(externalWebJob.jobId);
    }, [externalWebJob]);

    const job = usePollingJob(jobId, (finished) => {
        refreshQueue();
        if (finished.status === 'FAILED') {
            notify.error(finished.error || '识别失败');
            return;
        }
        if (finished.status !== 'DONE') return;

        // 网页结构化抓取：结果不属于某张图，单独摆出来，别误报成连拍队列。
        if (finished.kind === 'capture_web') {
            const webItems = finished.result?.items || [];
            const key = `web:${finished.id}`;
            viewKey.current = key;
            const picked = new Set(
                webItems
                    .map((item, index) => (isBlocked(item) ? null : index))
                    .filter((value) => value !== null),
            );
            shotCache.current.set(key, {items: webItems, selection: [...picked]});
            setItems(webItems);
            setSelection(picked);
            if (externalWebJob?.jobId === finished.id && externalWebJob.truncated) {
                notify.warning(`本页有 ${externalWebJob.total} 道题，已读取前 ${webItems.length} 道，请翻页后继续`);
            } else {
                notify.success(`网页里收到 ${webItems.length} 道题，右边逐题核对`);
            }
            return;
        }

        const list = finished.result?.items || [];
        const failures = finished.result?.failures || [];
        // 批量识别时每道题都带 capturePath；按来源图分组，各回各页的缓存。
        const groups = new Map();
        for (const item of list) {
            const path = item.capturePath || '';
            const key = (path && pathKeys.current.get(path)) || path || viewKey.current;
            if (!key) continue;
            if (!groups.has(key)) groups.set(key, []);
            groups.get(key).push(item);
        }
        groups.forEach((group, key) => {
            const picked = new Set(
                group
                    .map((item, index) => (isBlocked(item) ? null : index))
                    .filter((value) => value !== null),
            );
            shotCache.current.set(key, {items: group, selection: [...picked]});
        });

        // 右栏只摆当前这张图的结果；别的图的结果留在缓存里，翻回去就有了。
        const current = viewKey.current ? shotCache.current.get(viewKey.current) : null;
        if (current) {
            setItems(current.items);
            setSelection(new Set(current.selection));
        } else if (list.length > 0) {
            // 结果来自连拍队列里的裁块，跟当前页没有一一对应关系；
            // 先整体摆出来，总比「认完了却看不到」强。
            setItems(list);
            setSelection(
                new Set(
                    list
                        .map((item, index) => (isBlocked(item) ? null : index))
                        .filter((value) => value !== null),
                ),
            );
            notify.info('这批结果来自连拍队列，逐题核对后可一起存草稿');
        } else {
            setItems([]);
            setSelection(new Set());
        }

        if (failures.length) notify.warning(failures.slice(0, 2).join('；'));
        notify.success(
            list.length > 0
                ? `认出 ${list.length} 道题${groups.size > 1 ? `（${groups.size} 张图）` : ''}，右边逐题核对`
                : '没有认出题目',
        );
    });

    jobRef.current = job;

    // 右栏里的改动（改题、勾选）随时写回当前这张图的缓存。
    useEffect(() => {
        const key = viewKey.current;
        if (!key || items.length === 0) return;
        shotCache.current.set(key, {items, selection: [...selection]});
    }, [items, selection]);

    // 识别进行中，队列状态会一张张变，隔一会儿刷一次。
    useEffect(() => {
        if (job?.status !== 'RUNNING') return undefined;
        const timer = setInterval(refreshQueue, 2500);
        return () => clearInterval(timer);
    }, [job?.status, refreshQueue]);

    useEffect(() => {
        if (!subjectId && tree.length) setSubjectId(tree[0].id);
    }, [tree, subjectId]);

    const rememberSource = useCallback(
        async (url) => {
            const target = (url || '').trim();
            if (!target) return;
            try {
                await captureSourceRemember(target, hostOf(target));
                refreshSources();
            } catch {
                /* 来源记忆失败不影响取题 */
            }
        },
        [refreshSources],
    );

    /** 清掉「这张图是从哪来的」这一层状态，翻页 / 换图时都要走一遍。 */
    const resetShot = useCallback(() => {
        setRegions([]);
        setPickedRegions(new Set());
        setRect(null);
        setSelecting(false);
        setSpan(null);
        setSpanPaths([]);
        setSpanPick(null);
        viewKey.current = '';
        setJobId('');
        setItems([]);
        setSelection(new Set());
    }, []);

    /** 当前取景的缓存键：PDF 用「文件 + 页码」，其它图用图片路径。 */
    const keyOf = useCallback(
        (path, page) => (path && page ? `${path}#${page}` : path || ''),
        [],
    );

    /**
     * 把某个键的结果摆到右栏；没有缓存就是空的。
     * 故意不恢复 jobId：旧任务已经结束，恢复它会让轮询再跑一遍，
     * 把用户在预览里改过的内容用模型原稿覆盖掉。
     */
    const loadShot = useCallback((key) => {
        viewKey.current = key || '';
        const hit = key ? shotCache.current.get(key) : null;
        // 还在识别的任务不能掐掉：掐了轮询就停，结果回来也没人收。
        setJobId((current) => (current && jobRef.current?.status === 'RUNNING' ? current : ''));
        setItems(hit?.items || []);
        setSelection(new Set(hit?.selection || []));
    }, []);

    /**
     * 自动找题块：打开一页就跑一遍，用户点一下就能选中那道题。
     * 翻页很快时旧请求会晚回来，用 token 丢掉过期结果。
     */
    const detectRegions = useCallback(async (path) => {
        if (!path) return [];
        const token = ++smartToken.current;
        setSmartBusy(true);
        let found = [];
        try {
            found = (await smartRegions(path)) || [];
            if (token !== smartToken.current) return found;
            setRegions(found);
            setPickedRegions(new Set());
        } catch {
            if (token === smartToken.current) setRegions([]);
        } finally {
            if (token === smartToken.current) setSmartBusy(false);
        }
        return found;
    }, []);

    /** 把一张图放进连拍队列；来源网址跟着一起记。 */
    const enqueue = useCallback(
        async (path, note = '') => {
            if (!path) return;
            try {
                const sourceUrl = urlRef.current || '';
                await captureQueueAdd(path, sourceUrl);
                if (sourceUrl) rememberSource(sourceUrl);
                await refreshQueue();
                if (note) notify.success(note);
            } catch (error) {
                notify.error(error.message);
            }
        },
        [refreshQueue, rememberSource],
    );

    /** 新图到位：连拍模式下直接入队，精细模式下留在取景区等指令。 */
    const adoptImage = useCallback(
        async (result, how) => {
            setImage(result);
            setDoc(null);
            resetShot();
            const key = keyOf(result.path);
            pathKeys.current.set(result.path, key);
            loadShot(key);
            detectRegions(result.path);
            if (burst) {
                setImageQueued(true);
                await enqueue(result.path, `已入队（${how}），继续取下一张`);
            } else {
                setImageQueued(false);
            }
        },
        [burst, detectRegions, enqueue, keyOf, loadShot, resetShot],
    );

    /**
     * 打开本地文件：图片直读，PDF 按页渲成图。
     * PDF 自动切到精细模式——整卷要按页裁，连拍在这里没有意义。
     */
    const openLocal = useCallback(
        async (path, page = 1, quiet = false) => {
            try {
                const file = await fileOpen(path, page);

                // Word / txt 等纯文字文件：没有版面图，走「粘贴文字」链路。
                if (file.kind === 'text') {
                    setTab('text');
                    setPasteText(file.text || '');
                    notify.success(`${file.name} · 已读取文字，点「整理成题目」即可`);
                    return file;
                }

                const key = keyOf(file.path, file.page);
                pathKeys.current.set(file.image.path, key);
                setDoc({
                    path: file.path,
                    name: file.name,
                    kind: file.kind,
                    page: file.page,
                    pages: file.pages,
                });
                setImage(file.image);
                setImageQueued(false);
                resetShot();
                loadShot(key);
                detectRegions(file.image.path);
                if (file.kind === 'pdf') {
                    setBurst(false);
                    const cached = shotCache.current.has(key);
                    if (!quiet) {
                        notify.success(
                            cached
                                ? `${file.name} · 第 ${file.page}/${file.pages} 页（上次的识别结果已恢复）`
                                : `${file.name} · 第 ${file.page}/${file.pages} 页`,
                        );
                    }
                }
                return file;
            } catch (error) {
                notify.error(error.message);
                return null;
            }
        },
        [detectRegions, keyOf, loadShot, resetShot],
    );

    /** 一批本地路径：PDF/Word/txt 各自走对应链路；图片按当前模式入队或留在取景。 */
    const handlePaths = useCallback(
        async (paths) => {
            const list = (paths || []).filter(Boolean);
            if (list.length === 0) return;
            const isTextPath = (p) => /\.(docx?|docm|rtf|odt|txt|text|md|markdown|csv|tsv|html?|webarchive)$/i.test(p);
            const isPdf = (p) => /\.pdf$/i.test(p);

            // 文字文件和 PDF 一样，一次只处理一份；图片可批量。
            const docLike = list.find((path) => isPdf(path) || isTextPath(path));
            if (docLike) {
                if (isPdf(docLike)) setTab('shot');
                await openLocal(docLike);
                if (list.length > 1) notify.info('一次打开一份文档；纯图片可以多选一起入队');
                return;
            }
            setTab('shot');
            let last = null;
            for (const path of list) {
                // eslint-disable-next-line no-await-in-loop
                const file = await openLocal(path);
                if (!file) continue;
                last = file;
                // eslint-disable-next-line no-await-in-loop
                if (burst) await enqueue(file.image.path, '');
            }
            if (!last) return;
            if (burst) {
                setImageQueued(true);
                notify.success(`已入队 ${list.length} 张，继续拖入下一批`);
            } else {
                notify.success('文件已就位，可框选或智能裁剪');
            }
        },
        [burst, enqueue, openLocal],
    );

    const shoot = async () => {
        if (busy.current) return;
        busy.current = true;
        try {
            const result = await captureRegion();
            setTab('shot');
            await adoptImage(result, '截图');
        } catch (error) {
            if (!String(error.message).includes('取消')) notify.error(error.message);
        } finally {
            busy.current = false;
        }
    };

    // 全局热键 ⌘⇧O 推进 trigger；窗口里不再重复绑热键。
    useEffect(() => {
        if (trigger !== lastTrigger.current) {
            lastTrigger.current = trigger;
            shoot();
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [trigger]);

    useEffect(() => {
        const onPaste = async (event) => {
            const files = [...(event.clipboardData?.files || [])].filter((file) =>
                file.type.startsWith('image/'),
            );
            if (files.length === 0) return;
            event.preventDefault();
            setTab('shot');
            try {
                for (const file of files) {
                    // eslint-disable-next-line no-await-in-loop
                    const dataUrl = await readFileAsDataUrl(file);
                    // eslint-disable-next-line no-await-in-loop
                    const path = await saveTempImage(dataUrl);
                    if (burst) {
                        // eslint-disable-next-line no-await-in-loop
                        await enqueue(path);
                        setImage({path, dataUrl, width: 0, height: 0});
                        setDoc(null);
                        resetShot();
                        setImageQueued(true);
                    } else {
                        setImage({path, dataUrl, width: 0, height: 0});
                        setDoc(null);
                        setImageQueued(false);
                        resetShot();
                    }
                }
                notify.success(burst ? `已入队 ${files.length} 张，翻页继续粘贴` : '图片已就位，点「开始识别」');
            } catch (error) {
                notify.error(error.message);
            }
        };
        window.addEventListener('paste', onPaste);
        return () => window.removeEventListener('paste', onPaste);
    }, [burst, enqueue, resetShot]);

    // 浏览器控制条截完图，把路径送进来；顺手记住当前真实网址。
    useEffect(() => {
        if (!externalPath || externalPath === lastExternal.current) return;
        lastExternal.current = externalPath;
        // 网页截图落回「本地 / 截图」页签：那里才有舞台和「识别这张」。
        setTab('shot');
        browserStatus()
            .then((status) => {
                if (status.url) setActiveUrl(status.url);
            })
            .catch(() => {});
        imageFromPath(externalPath)
            .then((result) => adoptImage(result, '网页截图'))
            .catch((error) => notify.error(error.message));
    }, [externalPath, adoptImage]);

    // 结构化提取失败时收下整页文字，直接在网页页签里跑文字识别，不再跳到「粘贴文字」。
    useEffect(() => {
        if (!externalText || externalText === lastExternalText.current) return;
        lastExternalText.current = externalText;
        setPasteText(externalText);
        browserStatus()
            .then((status) => {
                if (status.url) setActiveUrl(status.url);
            })
            .catch(() => {});
        setSelection(new Set());
        setItems([]);
        startJob('capture_text', {
            text: externalText,
            subjectId,
            sourceUrl: urlRef.current || '',
        })
            .then((id) => setJobId(id))
            .catch((error) => notify.error(error.message));
    }, [externalText]);

    // 从 Finder 直接把整卷 PDF / 图片拖进来，省掉「打开文件」这一步。
    useEffect(() => {
        let unlisten = null;
        let alive = true;
        getCurrentWebview()
            .onDragDropEvent((event) => {
                if (event.payload?.type !== 'drop') return;
                const paths = event.payload.paths || [];
                if (paths.length) handlePaths(paths);
            })
            .then((fn) => {
                if (alive) unlisten = fn;
                else fn();
            })
            .catch(() => {});
        return () => {
            alive = false;
            unlisten?.();
        };
    }, [handlePaths]);

    const chooseEngine = (value) => {
        if (value === 'local' && ocr && !ocr.available) return;
        setEngine(value);
        settingSet('capture.engine', value).catch(() => {});
    };

    const openBrowser = async (url) => {
        const target = (url ?? browserUrl).trim();
        if (!target) {
            notify.error('先填或选一个试题网址');
            return;
        }
        try {
            await browserOpen(target, subjectId);
            setBrowserOn(true);
            setBrowserUrl(target);
            setActiveUrl(target);
            rememberSource(target);
            notify.success('已打开；在页面右下角点「识别本页 N 题」，或逐题点「收这题」');
        } catch (error) {
            notify.error(error.message);
        }
    };

    /** 让内置浏览器里的脚本重新扫题块；结果通过事件回到这里。 */
    const readPage = async () => {
        if (!browserOn) {
            notify.error('先在下面打开一个试题网址');
            return;
        }
        try {
            await browserExtractQuestions();
            notify.info('正在读取当前页题块…');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const closeBrowser = async () => {
        try {
            await browserClose();
            setBrowserOn(false);
        } catch (error) {
            notify.error(error.message);
        }
    };

    /**
     * 内嵌网页视图不是 DOM 节点，只能由 Rust 侧按主窗坐标摆放。
     * 这里把占位框的位置报上去；切走页签就藏起来，回来再显示，登录态不丢。
     */
    useEffect(() => {
        if (tab !== 'web' || !browserOn) {
            browserHide().catch(() => {});
            return undefined;
        }
        let alive = true;
        const sync = () => {
            const node = webFrame.current;
            if (!alive || !node) return;
            const box = node.getBoundingClientRect();
            const top = Math.max(0, box.top);
            const left = Math.max(0, box.left);
            const width = Math.min(window.innerWidth, box.right) - left;
            const height = Math.min(window.innerHeight, box.bottom) - top;
            if (width < 2 || height < 2) return;
            browserBounds(left, top, width, height).catch(() => {});
        };
        sync();
        const timer = window.setInterval(sync, 200);
        return () => {
            alive = false;
            window.clearInterval(timer);
            browserHide().catch(() => {});
        };
    }, [tab, browserOn]);

    /**
     * 跨页拼接第一步：定住「上一页要接的那段」，翻到下一页等框选。
     *
     * 老做法是把两整页竖着拼起来，模型面对的是一张超高的图：页眉页脚、上一页
     * 已经认过的题全在里面，十次有八次只认一半。这里改成「各页只取需要的那段」：
     * 上一页有框就用框，没有才用整页；下一页同理。
     */
    const stitchNext = async () => {
        if (!doc || doc.kind !== 'pdf' || !image?.path) return;
        const from = span ? span.from : doc.page;
        const to = span ? span.to + 1 : doc.page + 1;
        if (to > doc.pages) {
            notify.info('已经是最后一页了');
            return;
        }
        setSmartBusy(true);
        try {
            let base = image.path;
            // 已拼过的图不能再拿残留的手选框去裁，否则前几页会被裁掉一截。
            const cropped = !!rect && !span;
            if (cropped) {
                const cut = await smartCrop(image.path, regionFromRect(image, rect));
                base = cut.path;
            }
            const next = await fileOpen(doc.path, to);
            pathKeys.current.set(next.image.path, keyOf(doc.path, to));
            setDoc((current) => (current ? {...current, page: to} : current));
            setImage(next.image);
            setSpanPick({from, to, base, cropped});
            setRegions([]);
            setPickedRegions(new Set());
            setRect(null);
            setSelecting(false);
            loadShot(keyOf(doc.path, to));
            detectRegions(next.image.path);
            notify.info(
                `已取第 ${from} 页${cropped ? '的选区' : '整页'}；在第 ${to} 页框出要接的部分，或直接「接上」`,
            );
        } catch (error) {
            notify.error(error.message);
        } finally {
            setSmartBusy(false);
        }
    };

    /** 跨页拼接第二步：把两段竖着拼成一张，可顺手识别。 */
    const finishSpan = async (andRecognize = false) => {
        if (!spanPick || !image?.path || !doc) return;
        setSmartBusy(true);
        try {
            let tail = image.path;
            if (rect) {
                const cut = await smartCrop(image.path, regionFromRect(image, rect));
                tail = cut.path;
            }
            const chain = spanPaths.length > 0 ? [...spanPaths, tail] : [spanPick.base, tail];
            const stitched = await imageStitch(chain);
            const key = `${doc.path}#${spanPick.from}-${spanPick.to}`;
            pathKeys.current.set(stitched.path, key);
            setImage(stitched);
            setSpan({from: spanPick.from, to: spanPick.to});
            setSpanPaths(chain);
            setSpanPick(null);
            setRegions([]);
            setPickedRegions(new Set());
            setRect(null);
            setSelecting(false);
            loadShot(key);
            detectRegions(stitched.path);
            if (andRecognize) {
                await recognize(stitched.path);
            } else {
                notify.success(`已接上第 ${spanPick.from}–${spanPick.to} 页，点「识别这张」`);
            }
        } catch (error) {
            notify.error(error.message);
        } finally {
            setSmartBusy(false);
        }
    };

    const cancelSpan = async () => {
        const back = spanPick?.from || span?.from || doc?.page || 1;
        setSpanPick(null);
        setSpan(null);
        setSpanPaths([]);
        if (doc) await openLocal(doc.path, back);
    };

    const runSmartCrop = async () => {
        if (!image?.path) return;
        const found = await detectRegions(image.path);
        if (found.length === 0) {
            notify.info('没找到题块，直接用整图识别，或换张更清晰的整卷图');
        } else {
            notify.success(`找到 ${found.length} 块，点一下选中该题`);
        }
    };

    /** 点某块 → 裁下来 → 直接识别，不用先入队再批量。 */
    const recognizeRegion = async (index) => {
        const region = regions[index];
        if (!image?.path || !region) return;
        try {
            const cropped = await smartCrop(image.path, region);
            pathKeys.current.set(cropped.path, cropped.path);
            setImage(cropped);
            setRegions([]);
            setPickedRegions(new Set());
            setRect(null);
            setSpanPick(null);
            setSpan(null);
            setSpanPaths([]);
            loadShot(cropped.path);
            await recognize(cropped.path);
        } catch (error) {
            notify.error(error.message);
        }
    };

    const enqueuePickedRegions = async () => {
        if (!image?.path || pickedRegions.size === 0) return;
        try {
            const list = [...pickedRegions].sort((a, b) => a - b);
            for (const index of list) {
                // eslint-disable-next-line no-await-in-loop
                const cropped = await smartCrop(image.path, regions[index]);
                // eslint-disable-next-line no-await-in-loop
                await captureQueueAdd(cropped.path, urlRef.current || '');
            }
            if (urlRef.current) rememberSource(urlRef.current);
            await refreshQueue();
            setPickedRegions(new Set());
            notify.success(`已裁下 ${list.length} 块并加入队列`);
        } catch (error) {
            notify.error(error.message);
        }
    };

    const pickFiles = async () => {
        const picked = await open({
            multiple: true,
            filters: [
                {name: '试卷文件（图片 / PDF / Word / 文本）', extensions: FILE_EXTS},
                {name: '图片', extensions: IMAGE_EXTS},
                {name: '全部文件', extensions: ['*']},
            ],
        });
        if (!picked) return;
        await handlePaths(Array.isArray(picked) ? picked : [picked]);
    };

    /** 显示坐标（百分比）→ 原图像素；smart_crop 自己会夹紧越界，这里只管换算。 */
    const regionFromRect = (target, box) => ({
        x: Math.max(0, Math.round((box.left / 100) * target.width)),
        y: Math.max(0, Math.round((box.top / 100) * target.height)),
        w: Math.max(8, Math.round((box.w / 100) * target.width)),
        h: Math.max(8, Math.round((box.h / 100) * target.height)),
    });

    const startDrag = (event) => {
        if (!selecting || !image) return;
        dragRef.current = {
            box: event.currentTarget.getBoundingClientRect(),
            x0: event.clientX,
            y0: event.clientY,
        };
        event.currentTarget.setPointerCapture?.(event.pointerId);
        setRect({left: 0, top: 0, w: 0, h: 0});
    };

    const moveDrag = (event) => {
        const drag = dragRef.current;
        if (!drag) return;
        const clamp = (value) => Math.min(100, Math.max(0, value));
        const x1 = clamp(((drag.x0 - drag.box.left) / drag.box.width) * 100);
        const y1 = clamp(((drag.y0 - drag.box.top) / drag.box.height) * 100);
        const x2 = clamp(((event.clientX - drag.box.left) / drag.box.width) * 100);
        const y2 = clamp(((event.clientY - drag.box.top) / drag.box.height) * 100);
        setRect({
            left: Math.min(x1, x2),
            top: Math.min(y1, y2),
            w: Math.abs(x2 - x1),
            h: Math.abs(y2 - y1),
        });
    };

    const endDrag = () => {
        if (!dragRef.current) return;
        dragRef.current = null;
        // 手抖画出的 1~2 像素小框没有意义，直接丢掉。
        setRect((current) => (current && current.w > 1.5 && current.h > 1.5 ? current : null));
    };

    const cropRect = async (enqueueOnly = true) => {
        if (!image?.path || !rect) return;
        try {
            const cropped = await smartCrop(image.path, regionFromRect(image, rect));
            if (enqueueOnly) {
                await captureQueueAdd(cropped.path, urlRef.current || '');
                if (urlRef.current) rememberSource(urlRef.current);
                await refreshQueue();
                notify.success('已裁下选区并入队');
            } else {
                setImage(cropped);
                pathKeys.current.set(cropped.path, cropped.path);
                setSpan(null);
                setSpanPaths([]);
                setSpanPick(null);
                loadShot(cropped.path);
                notify.success('已按选区裁好，点「识别这张」');
            }
            setRect(null);
            setZoom(false);
        } catch (error) {
            notify.error(error.message);
        }
    };

    const recognize = async (pathOverride = '') => {
        const path = pathOverride || image?.path || '';
        if (!path) {
            notify.error('先截一张图、拖一张图片，或打开本地文件（PDF / Word / 文本）');
            return;
        }
        try {
            // 重新认这一张就以新结果为准，旧缓存作废。
            if (viewKey.current) shotCache.current.delete(viewKey.current);
            setSelection(new Set());
            setItems([]);
            // 这张图如果已经进了连拍队列，把队列项 id 一起带上：识别过程会回写
            // 待识别 → 识别中 → 已识别，否则队列里永远是「待识别」，看着像没生效。
            const queued = queue.find(
                (entry) => entry.imagePath === path && entry.status !== 'RUNNING',
            );
            setJobId(
                await startJob('capture', {
                    imagePath: path,
                    subjectId,
                    sourceUrl: urlRef.current || '',
                    engine,
                    queueId: queued?.id || '',
                }),
            );
            if (queued) refreshQueue();
        } catch (error) {
            notify.error(error.message);
        }
    };

    /** 题块点击：普通点选一块，⌘ / Shift 追加；再点同一块取消。 */
    const pickRegion = useCallback((index, additive) => {
        setPickedRegions((current) => {
            if (!additive) {
                const only = current.size === 1 && current.has(index);
                return only ? new Set() : new Set([index]);
            }
            const next = new Set(current);
            if (next.has(index)) next.delete(index);
            else next.add(index);
            return next;
        });
    }, []);

    const pending = useMemo(() => queue.filter((item) => item.status === 'PENDING'), [queue]);
    // 当前这张图在队列里的那一条：状态跟着识别任务一起变，界面直接读它，别自己另记一份。
    const shotQueue = useMemo(
        () => queue.find((entry) => entry.imagePath === image?.path),
        [queue, image?.path],
    );
    const doneCount = useMemo(
        () => queue.filter((item) => item.status === 'DONE' || item.status === 'FAILED').length,
        [queue],
    );

    const runBatch = async () => {
        if (pending.length === 0) {
            notify.info('队列里没有待识别的图片');
            return;
        }
        try {
            // 批量识别期间右栏保持原样，结果回来再按图分组覆盖。
            setSelection(new Set());
            setJobId(
                await startJob('capture_batch', {
                    subjectId,
                    engine,
                    images: pending.map((item) => ({
                        path: item.imagePath,
                        sourceUrl: item.sourceUrl,
                        queueId: item.id,
                    })),
                }),
            );
        } catch (error) {
            notify.error(error.message);
        }
    };

    const runText = async () => {
        const text = pasteText.trim();
        if (!text) {
            notify.error('先粘贴题目文字');
            return;
        }
        try {
            setSelection(new Set());
            setItems([]);
            setJobId(
                await startJob('capture_text', {
                    text,
                    subjectId,
                    sourceUrl: urlRef.current || '',
                }),
            );
        } catch (error) {
            notify.error(error.message);
        }
    };

    const toggle = (index, checked) => {
        setSelection((current) => {
            const next = new Set(current);
            if (checked) next.add(index);
            else next.delete(index);
            return next;
        });
    };

    /** 预览里改一道题：改完重算问题清单，补齐了自动勾上，缺的当场列出来。 */
    const updateItem = async (index, patch) => {
        if (!job) return false;
        try {
            const updated = await updateJobItem(job.id, index, patch);
            setItems((current) => current.map((item, position) => (position === index ? updated : item)));
            const left = updated.issues || [];
            if (left.length > 0) {
                notify.warning(`还差 ${left.length} 项：${left.join('、')}`);
            } else {
                setSelection((current) => {
                    const next = new Set(current);
                    next.add(index);
                    return next;
                });
                notify.success('改好了，已经勾上，可以存草稿');
            }
            return true;
        } catch (error) {
            notify.error(error.message);
            return false;
        }
    };

    const readyCount = items.filter((item) => !isBlocked(item)).length;

    const commit = async () => {
        if (!job || selection.size === 0) return;
        // 存之前先记下「下一站在哪」：整卷 PDF 存完一页就自动翻到下一页，不用重新打开文件。
        const docAtCommit = doc;
        const nextPage = docAtCommit?.kind === 'pdf' ? (span ? span.to : docAtCommit.page) + 1 : 0;
        setCommitting(true);
        try {
            const result = await commitJob(job.id, [...selection]);
            const imported = result.imported || 0;
            const skipped = result.skipped || 0;
            const problems = result.problems || [];
            if (imported === 0) {
                notify.error(problems[0] || '没有题目存入草稿箱，请检查后重新识别');
                problems.slice(1, 3).forEach((problem) => notify.warning(problem));
                // 后端已经结束了这次提交；清掉旧任务，但保留原文，避免拿着旧任务反复提交。
                resetShot();
                refreshQueue();
                return;
            }

            const summary = `已存入草稿箱 ${imported} 道${result.needsWork ? `（其中 ${result.needsWork} 道仍有待补项）` : ''}${skipped ? `，${skipped} 道未存` : ''}；复核通过后才是正式入库`;
            if (skipped > 0 || problems.length > 0) {
                notify.warning(summary);
            } else {
                notify.success(summary);
            }
            problems.slice(0, 3).forEach((problem) => notify.warning(problem));
            refreshQueue();
            onChanged?.();

            if (skipped > 0) {
                // 已存的不能重复提交，未存的也不该被自动翻页带走；留下原文供重新识别。
                resetShot();
                notify.info('未存的题目请重新识别，下次只勾选还没存的题目');
                return;
            }
            setImage(null);
            setImageQueued(false);
            resetShot();
            if (docAtCommit?.kind === 'pdf' && nextPage <= docAtCommit.pages) {
                await openLocal(docAtCommit.path, nextPage, true);
            } else {
                setDoc(null);
            }
        } catch (error) {
            notify.error(error.message);
        } finally {
            setCommitting(false);
        }
    };

    const toggleStar = async (source) => {
        try {
            await captureSourceStar(source.id, !source.starred);
            refreshSources();
        } catch (error) {
            notify.error(error.message);
        }
    };

    const forgetSource = async (source) => {
        try {
            await captureSourceForget(source.id);
            refreshSources();
        } catch (error) {
            notify.error(error.message);
        }
    };

    const removeQueueItem = async (item) => {
        try {
            await captureQueueRemove(item.id);
            refreshQueue();
        } catch (error) {
            notify.error(error.message);
        }
    };

    const retryQueueItem = async (item) => {
        try {
            await captureQueueRetry(item.id);
            refreshQueue();
        } catch (error) {
            notify.error(error.message);
        }
    };

    const clearDone = async () => {
        try {
            const removed = await captureQueueClearDone();
            refreshQueue();
            notify.success(`清理了 ${removed} 项`);
        } catch (error) {
            notify.error(error.message);
        }
    };

    // 只表示「这是拼接页」；普通 PDF 的页码由 doc 自己给，别混在这里。
    const spanLabel = span && doc ? `第 ${span.from}–${span.to} 页（拼接）` : '';

    const pageNumbers = useMemo(() => {
        if (doc?.kind !== 'pdf' || !doc.pages) return [];
        return Array.from({length: doc.pages}, (_, index) => index + 1);
    }, [doc]);

    const viewTitle = tab === 'web' ? '网页取题' : tab === 'text' ? '粘贴文字' : doc ? doc.name : '原文';

    return (
        <div className="page studio-page">
            <PageHead
                title="录入"
                subtitle="挑一个来源取原文，右边逐题核对，改好存草稿"
                help="capture"
                actions={
                    <button className="btn ghost" onClick={() => setEditing(true)}>
                        手工录题
                    </button>
                }
            />

            <Tabs
                value={tab}
                onChange={setTab}
                items={[
                    {value: 'shot', label: '本地 / 截图', count: pending.length},
                    {value: 'web', label: '网页取题'},
                    {value: 'text', label: '粘贴文字'},
                ]}
            />

            <div className={cx('studio', tab === 'web' && 'studio-vertical')}>
                <Card
                    className={cx('studio-view', tab !== 'shot' && 'compact')}
                    title={viewTitle}
                    subtitle={
                        tab === 'shot' && doc
                            ? spanLabel
                                ? `${spanLabel} · 跨页题已拼接`
                                : `第 ${doc.page}/${doc.pages} 页`
                            : tab === 'web'
                                ? '粘网址打开试题页，题就在网页里点「收这题」'
                                : tab === 'text'
                                    ? '题目文字粘进来，不用图片'
                                    : '打开 PDF 或截一张图，一次处理一页'
                    }
                    actions={
                        tab === 'shot' ? (
                            <div className="inline-actions">
                                <button className="btn primary small" onClick={pickFiles}>
                                    打开文件
                                </button>
                                <button className="btn ghost small" onClick={shoot}>
                                    截图选区
                                </button>
                            </div>
                        ) : null
                    }
                >
                    <div className="studio-body">
                        {tab === 'shot' && (
                            <>
                                {pageNumbers.length > 1 && (
                                    <div className="page-strip" role="tablist" aria-label="页面">
                                        {pageNumbers.map((page) => (
                                            <button
                                                key={page}
                                                className={cx(
                                                    'page-pill',
                                                    (span ? span.from : doc.page) === page && 'active',
                                                )}
                                                onClick={() => openLocal(doc.path, page)}
                                                title={`第 ${page} 页`}
                                            >
                                                {page}
                                            </button>
                                        ))}
                                    </div>
                                )}

                                <div className="studio-tools">
                                    <button
                                        className={cx('chip', burst && 'active')}
                                        onClick={() => setBurst((value) => !value)}
                                        title="开着：每次截图 / 粘贴自动入队；关掉：留在取景区精细处理"
                                    >
                                        连拍{burst ? ' · 开' : ''}
                                    </button>
                                    <button
                                        className={cx('chip', selecting && 'active')}
                                        onClick={() => {
                                            setSelecting((value) => !value);
                                            setRect(null);
                                        }}
                                        disabled={!image}
                                        title="在图上按住拖出一个框，只裁这一块"
                                    >
                                        框选{selecting ? ' · 开' : ''}
                                    </button>
                                    {doc?.kind === 'pdf' && (
                                        <button
                                            className="btn ghost small"
                                            onClick={stitchNext}
                                            disabled={
                                                smartBusy ||
                                                !image ||
                                                !!spanPick ||
                                                (span ? span.to : doc.page) >= doc.pages
                                            }
                                            title="先在上一页框出要接的那段，再翻到下一页框出其余部分，拼成一张再识别"
                                        >
                                            {span ? `再拼第 ${span.to + 1} 页` : '跨页拼接'}
                                        </button>
                                    )}
                                    {span && !spanPick && (
                                        <button className="btn ghost small" onClick={cancelSpan}>
                                            取消拼接
                                        </button>
                                    )}
                                    <button
                                        className="btn ghost small"
                                        onClick={runSmartCrop}
                                        disabled={!image || smartBusy}
                                        title="重新在本页找题块；打开页面时会自动找一次"
                                    >
                                        {smartBusy ? '找题块…' : '重新找题块'}
                                    </button>
                                    <button
                                        className="btn ghost small"
                                        onClick={() => setZoom(true)}
                                        disabled={!image}
                                    >
                                        放大
                                    </button>
                                </div>

                                <div className="capture-controls">
                                    <label className="field">
                                        <span className="field-label">学科</span>
                                        <select
                                            value={subjectId}
                                            onChange={(event) => setSubjectId(event.target.value)}
                                        >
                                            {tree.map((subject) => (
                                                <option key={subject.id} value={subject.id}>
                                                    {subject.name}
                                                </option>
                                            ))}
                                        </select>
                                    </label>
                                    <label className="field">
                                        <span className="field-label">识别方式</span>
                                        <select
                                            value={engine}
                                            onChange={(event) => chooseEngine(event.target.value)}
                                        >
                                            {ENGINES.map((item) => (
                                                <option
                                                    key={item.value}
                                                    value={item.value}
                                                    disabled={
                                                        item.value === 'local' &&
                                                        ocr &&
                                                        !ocr.available
                                                    }
                                                >
                                                    {item.label}
                                                </option>
                                            ))}
                                        </select>
                                    </label>
                                    <button
                                        className="btn primary"
                                        onClick={() => recognize()}
                                        disabled={!image || job?.status === 'RUNNING'}
                                    >
                                        {job?.status === 'RUNNING' ? '识别中…' : '识别这张'}
                                    </button>
                                </div>

                                <p className="muted small">
                                    {engine === 'local'
                                        ? '本机离线识别，不花模型额度；公式图认不全时切「AI 识图」'
                                        : engine === 'vision'
                                            ? '图片会发给你配置的视觉模型'
                                            : ocr?.available
                                                ? '先本机 OCR，认不出再自动交给 AI 识图'
                                                : '本机 OCR 不可用，自动模式会直接用 AI 识图'}
                                </p>

                                {image ? (
                                    <div className="studio-stage">
                                        <div
                                            className={cx(
                                                'shot-frame studio-frame',
                                                selecting && 'selecting',
                                                doc?.kind === 'pdf' && 'fit',
                                            )}
                                            onPointerDown={startDrag}
                                            onPointerMove={moveDrag}
                                            onPointerUp={endDrag}
                                            onPointerCancel={endDrag}
                                        >
                                            <img src={image.dataUrl} alt="待识别的原文"/>
                                            {shotQueue ? (
                                                <span
                                                    className={cx(
                                                        'shot-badge',
                                                        shotQueue.status.toLowerCase(),
                                                    )}
                                                >
                                                    {
                                                        (STATUS_TEXT[shotQueue.status] ||
                                                            STATUS_TEXT.PENDING).label
                                                    }
                                                    {shotQueue.status === 'DONE' && shotQueue.itemCount > 0
                                                        ? ` ${shotQueue.itemCount} 道`
                                                        : ''}
                                                </span>
                                            ) : (
                                                imageQueued && <span className="shot-badge">已入队</span>
                                            )}
                                            {doc && (
                                                <span className="shot-file">
                                                    {doc.name}
                                                    {spanLabel ? ` · ${spanLabel}` : ''}
                                                </span>
                                            )}
                                            <RegionLayer
                                                regions={regions}
                                                picked={pickedRegions}
                                                image={image}
                                                onPick={(index, event) =>
                                                    pickRegion(
                                                        index,
                                                        event.metaKey || event.ctrlKey || event.shiftKey,
                                                    )
                                                }
                                            />
                                            {rect && (
                                                <div
                                                    className="rect-box"
                                                    style={{
                                                        left: `${rect.left}%`,
                                                        top: `${rect.top}%`,
                                                        width: `${rect.w}%`,
                                                        height: `${rect.h}%`,
                                                    }}
                                                />
                                            )}
                                        </div>
                                    </div>
                                ) : (
                                    <div className="shot empty-shot studio-empty">
                                        <p>打开本地 PDF / Word / 文本 / 图片，或点「截图选区」</p>
                                        <span className="muted small">
                                            也可以直接把文件拖进来；PDF 会按页渲成图，能翻页、拼页、裁剪
                                        </span>
                                    </div>
                                )}

                                {spanPick && (
                                    <div className="shot-hint span-hint">
                                        <strong>
                                            跨页拼接 · 第 {spanPick.from}–{spanPick.to} 页
                                        </strong>
                                        <span className="muted small">
                                            上一页取了{spanPick.cropped ? '你框的那段' : '整页'}；
                                            {rect
                                                ? '下一页也框好了，点「接上并识别」'
                                                : '在这页框出要接的部分，不框就整页接上'}
                                        </span>
                                        <div className="inline-actions">
                                            <button
                                                className="btn primary small"
                                                onClick={() => finishSpan(true)}
                                                disabled={smartBusy}
                                            >
                                                接上并识别
                                            </button>
                                            <button
                                                className="btn ghost small"
                                                onClick={() => finishSpan(false)}
                                                disabled={smartBusy}
                                            >
                                                只接上
                                            </button>
                                            <button className="btn ghost small" onClick={cancelSpan}>
                                                取消
                                            </button>
                                        </div>
                                    </div>
                                )}

                                {!spanPick && rect && (
                                    <div className="shot-hint">
                                        <strong>选区就绪</strong>
                                        <span className="muted small">只处理框住的部分，其余不动</span>
                                        <div className="inline-actions">
                                            <button className="btn primary small" onClick={() => cropRect(false)}>
                                                按选区重新识别
                                            </button>
                                            <button className="btn ghost small" onClick={() => cropRect(true)}>
                                                裁下并入队
                                            </button>
                                            <button className="btn ghost small" onClick={() => setRect(null)}>
                                                重画
                                            </button>
                                        </div>
                                    </div>
                                )}

                                {!spanPick && !rect && regions.length > 0 && (
                                    <div className="shot-hint">
                                        <strong>
                                            {pickedRegions.size === 1
                                                ? `已选第 ${[...pickedRegions][0] + 1} 题`
                                                : `找到 ${regions.length} 块`}
                                        </strong>
                                        <span className="muted small">
                                            {pickedRegions.size === 1
                                                ? '点「识别这道」直接裁下并识别；⌘ 点可多选后批量入队'
                                                : '点一下选中该题；不选就是整页识别'}
                                        </span>
                                        <div className="inline-actions">
                                            {pickedRegions.size === 1 && (
                                                <button
                                                    className="btn primary small"
                                                    onClick={() => recognizeRegion([...pickedRegions][0])}
                                                    disabled={smartBusy}
                                                >
                                                    识别这道
                                                </button>
                                            )}
                                            <button
                                                className="btn ghost small"
                                                onClick={enqueuePickedRegions}
                                                disabled={pickedRegions.size === 0}
                                            >
                                                裁下选中块入队
                                            </button>
                                            {pickedRegions.size > 0 && (
                                                <button
                                                    className="btn ghost small"
                                                    onClick={() => setPickedRegions(new Set())}
                                                >
                                                    取消选择
                                                </button>
                                            )}
                                        </div>
                                    </div>
                                )}

                                {job?.status === 'RUNNING' && (
                                    <JobProgress job={job} onCancel={() => cancelJob(job.id)}/>
                                )}

                                {queue.length > 0 && (
                                    <div className="queue-strip">
                                        <button
                                            className="queue-strip-head"
                                            onClick={() => setQueueOpen((value) => !value)}
                                        >
                                            <strong>连拍队列</strong>
                                            <span className="muted small">
                                                {pending.length} 张待识别
                                                {doneCount > 0 ? ` · ${doneCount} 张已完成` : ''}
                                            </span>
                                            <span className="link-btn">
                                                {queueOpen ? '收起' : '展开'}
                                            </span>
                                        </button>
                                        {queueOpen && (
                                            <>
                                                <div className="inline-actions">
                                                    {doneCount > 0 && (
                                                        <button className="btn ghost small" onClick={clearDone}>
                                                            清理已完成 {doneCount}
                                                        </button>
                                                    )}
                                                    <button
                                                        className="btn primary small"
                                                        onClick={runBatch}
                                                        disabled={pending.length === 0 || job?.status === 'RUNNING'}
                                                    >
                                                        识别全部待处理{pending.length > 0 ? ` (${pending.length})` : ''}
                                                    </button>
                                                </div>
                                                <div className="queue-list">
                                                    {queue.map((item) => {
                                                        const status =
                                                            STATUS_TEXT[item.status] || STATUS_TEXT.PENDING;
                                                        return (
                                                            <div
                                                                key={item.id}
                                                                className={cx(
                                                                    'queue-item',
                                                                    item.status.toLowerCase(),
                                                                )}
                                                            >
                                                                <QueueThumb path={item.imagePath}/>
                                                                <div className="queue-meta">
                                                                    <div className="queue-title">
                                                                        <span>
                                                                            {item.sourceUrl
                                                                                ? hostOf(item.sourceUrl)
                                                                                : '本地截图'}
                                                                        </span>
                                                                        <Pill tone={status.tone}>
                                                                            {status.label}
                                                                            {item.status === 'DONE' && item.itemCount > 0
                                                                                ? ` ${item.itemCount} 道`
                                                                                : ''}
                                                                        </Pill>
                                                                    </div>
                                                                    <span className="muted small">
                                                                        {item.createdAt?.slice(5, 16)}
                                                                        {item.error ? ` · ${item.error}` : ''}
                                                                    </span>
                                                                </div>
                                                                <div className="queue-actions">
                                                                    {item.status === 'FAILED' && (
                                                                        <button
                                                                            className="btn ghost small"
                                                                            onClick={() => retryQueueItem(item)}
                                                                        >
                                                                            重试
                                                                        </button>
                                                                    )}
                                                                    <button
                                                                        className="btn ghost small"
                                                                        onClick={() => removeQueueItem(item)}
                                                                    >
                                                                        移除
                                                                    </button>
                                                                </div>
                                                            </div>
                                                        );
                                                    })}
                                                </div>
                                            </>
                                        )}
                                    </div>
                                )}
                            </>
                        )}

                        {tab === 'web' && (
                            <>
                                <div className="browser-row">
                                    <input
                                        className="browser-url"
                                        placeholder="粘贴试题网址，回车打开"
                                        value={browserUrl}
                                        onChange={(event) => setBrowserUrl(event.target.value)}
                                        onKeyDown={(event) => {
                                            if (event.key === 'Enter') openBrowser();
                                        }}
                                    />
                                    <button className="btn primary small" onClick={() => openBrowser()}>
                                        打开网站
                                    </button>
                                    <button
                                        className="btn ghost small"
                                        onClick={readPage}
                                        disabled={!browserOn}
                                        title="重新扫一遍当前页的题块"
                                    >
                                        识别本页
                                    </button>
                                    <button
                                        className="btn ghost small"
                                        onClick={closeBrowser}
                                        disabled={!browserOn}
                                    >
                                        关闭网页
                                    </button>
                                    <button
                                        className="btn ghost small"
                                        onClick={() => rememberSource(browserUrl)}
                                        title="手动把当前网址记进来源"
                                    >
                                        记住来源
                                    </button>
                                </div>

                                <p className="muted small">
                                    网页就在下面打开，站内翻页、登录都在这一块里完成。每道题上方有「收这题」，
                                    右下角有「识别本页 N 题」。抓的是题干、选项、答案、解析与知识点，不截图，
                                    也不拿整页文字让模型猜；页面没有结构化题块时才退回文字整理。被登录或验证码
                                    拦住时，用系统浏览器打开再按 ⌘⇧O 截图，图会回到「本地 / 截图」里。
                                </p>

                                <div className="browser-frame" ref={webFrame}>
                                    {!browserOn && (
                                        <p className="muted small">
                                            粘贴试题网址，点「打开网站」，网页会显示在这里
                                        </p>
                                    )}
                                </div>

                                {sources.length > 0 ? (
                                    <div className="source-grid">
                                        {sources.map((source) => (
                                            <div
                                                key={source.id}
                                                className={cx(
                                                    'source-card',
                                                    activeUrl === source.url && 'active',
                                                )}
                                            >
                                                <button
                                                    className="source-main"
                                                    onClick={() => openBrowser(source.url)}
                                                    title={source.url}
                                                >
                                                    <strong>{source.label || hostOf(source.url)}</strong>
                                                    <span className="source-url">{source.url}</span>
                                                    <span className="source-meta">
                                                        采集 {source.captureCount} 次 · {source.lastAt?.slice(0, 16)}
                                                    </span>
                                                </button>
                                                <div className="source-actions">
                                                    <button
                                                        className={cx('icon-btn', source.starred && 'on')}
                                                        onClick={() => toggleStar(source)}
                                                        title={source.starred ? '取消置顶' : '置顶'}
                                                    >
                                                        {source.starred ? '★' : '☆'}
                                                    </button>
                                                    <button
                                                        className="icon-btn"
                                                        onClick={() => forgetSource(source)}
                                                        title="忘记这个来源"
                                                    >
                                                        ×
                                                    </button>
                                                </div>
                                            </div>
                                        ))}
                                    </div>
                                ) : (
                                    <Empty
                                        title="还没有记下来源"
                                        hint="打开过一次的试题网站会自动出现在这里，下次一点就回"
                                    />
                                )}
                            </>
                        )}

                        {tab === 'text' && (
                            <>
                                <textarea
                                    className="text-capture-area"
                                    rows={10}
                                    placeholder="把题目文字粘贴进来（可一次多道，按题号分隔）"
                                    value={pasteText}
                                    onChange={(event) => setPasteText(event.target.value)}
                                />
                                <div className="inline-actions">
                                    <button
                                        className="btn primary"
                                        onClick={runText}
                                        disabled={!pasteText.trim() || job?.status === 'RUNNING'}
                                    >
                                        整理成题目
                                    </button>
                                    <span className="muted small">不调用视觉模型，纯文字也能存草稿</span>
                                </div>
                                {job?.status === 'RUNNING' && (
                                    <JobProgress job={job} onCancel={() => cancelJob(job.id)}/>
                                )}
                            </>
                        )}
                    </div>
                </Card>

                <Card
                    className="studio-result"
                    title="识别结果"
                    subtitle={
                        items.length > 0
                            ? `${doc ? (span ? `第 ${span.from}–${span.to} 页 · ` : `第 ${doc.page} 页 · `) : ''}共 ${items.length} 道，可存 ${readyCount} 道；缺答案的也能先存，复核时补`
                            : '识别后逐题核对，改好再存草稿；翻页不会丢，各页结果分开留着'
                    }
                    actions={
                        items.length > 0 && (
                            <button
                                className="btn primary small"
                                onClick={commit}
                                disabled={committing || selection.size === 0}
                            >
                                {committing ? '保存中…' : `存为草稿 ${selection.size} 道`}
                            </button>
                        )
                    }
                >
                    <div className="studio-body">
                        {!job && items.length === 0 && (
                            <Empty
                                title="还没有识别结果"
                                hint="左边取一张原文，点「识别这张」，认出来的题会排在这里；一页一页认也行，翻回去结果还在"
                            />
                        )}
                        {job?.status === 'RUNNING' && <div className="skeleton-list">正在读图…</div>}
                        <PickBar items={items} selection={selection} onChange={setSelection}/>
                        {items.length > 0 && (
                            <div className="job-list">
                                {items.map((item, index) => (
                                    <JobItemCard
                                        key={index}
                                        index={index}
                                        item={item}
                                        tree={tree}
                                        selected={selection.has(index)}
                                        onToggle={toggle}
                                        kind="capture"
                                        onUpdate={updateItem}
                                    />
                                ))}
                            </div>
                        )}
                    </div>
                </Card>
            </div>

            {zoom && image && (
                <div className="zoom-mask" onClick={() => setZoom(false)}>
                    <div className="zoom-card" onClick={(event) => event.stopPropagation()}>
                        <div className="zoom-bar">
                            <strong>{doc ? doc.name : '当前取景'}</strong>
                            <span className="muted small">
                                {selecting
                                    ? '按住拖出一个框，只裁这一块'
                                    : '点题块直接选中该题，也可以点「框选」自己拖'}
                            </span>
                            <div className="inline-actions">
                                <button
                                    className={cx('chip', selecting && 'active')}
                                    onClick={() => {
                                        setSelecting(true);
                                        setRect(null);
                                    }}
                                >
                                    框选
                                </button>
                                <button className="btn ghost small" onClick={() => cropRect(true)} disabled={!rect}>
                                    裁下并入队
                                </button>
                                <button className="btn ghost small" onClick={() => setZoom(false)}>
                                    关闭
                                </button>
                            </div>
                        </div>
                        <div
                            className={cx('zoom-stage', selecting && 'selecting')}
                            onPointerDown={startDrag}
                            onPointerMove={moveDrag}
                            onPointerUp={endDrag}
                            onPointerCancel={endDrag}
                        >
                            <img src={image.dataUrl} alt="放大预览"/>
                            <RegionLayer
                                regions={regions}
                                picked={pickedRegions}
                                image={image}
                                onPick={(index, event) => {
                                    pickRegion(
                                        index,
                                        event.metaKey || event.ctrlKey || event.shiftKey,
                                    );
                                    setZoom(false);
                                }}
                            />
                            {rect && (
                                <div
                                    className="rect-box"
                                    style={{
                                        left: `${rect.left}%`,
                                        top: `${rect.top}%`,
                                        width: `${rect.w}%`,
                                        height: `${rect.h}%`,
                                    }}
                                />
                            )}
                        </div>
                    </div>
                </div>
            )}

            {editing && (
                <QuestionEditor
                    tree={tree}
                    defaultSubjectId={subjectId}
                    onClose={() => setEditing(false)}
                    onSaved={() => onChanged?.()}
                />
            )}
        </div>
    );
}

/** 题块层：主视图和放大层共用。点一下选中，⌘ 点追加，避免误触开始拖框。 */
function RegionLayer({regions, picked, image, onPick}) {
    if (!regions || regions.length === 0) return null;
    const width = image?.width || 1;
    const height = image?.height || 1;
    return (
        <div className="region-layer">
            {regions.map((region, index) => (
                <button
                    key={index}
                    className={cx('region-box', picked.has(index) && 'picked')}
                    title={`第 ${index + 1} 块 · 点一下选中，⌘ 点可多选`}
                    onPointerDown={(event) => event.stopPropagation()}
                    onClick={(event) => {
                        event.stopPropagation();
                        onPick(index, event);
                    }}
                    style={{
                        left: `${(region.x / width) * 100}%`,
                        top: `${(region.y / height) * 100}%`,
                        width: `${(region.w / width) * 100}%`,
                        height: `${(region.h / height) * 100}%`,
                    }}
                >
                    <span>{index + 1}</span>
                </button>
            ))}
        </div>
    );
}
