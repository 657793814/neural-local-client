import {useCallback, useEffect, useState} from 'react';
import {listen} from '@tauri-apps/api/event';
import {authLogout, bootstrap, curriculum, syncStatus} from './lib/api.js';
import {FeedbackHost, Loading} from './components/ui.jsx';
import BrandMark from './components/BrandMark.jsx';
import {ImageLightbox} from './components/ImageLightbox.jsx';
import LoginGate from './components/LoginGate.jsx';
import Library from './pages/Library.jsx';
import Stats from './pages/Stats.jsx';
import Papers from './pages/Papers.jsx';
import PaperEditor from './pages/PaperEditor.jsx';
import Capture from './pages/Capture.jsx';
import Classes from './pages/Classes.jsx';
import Assignments from './pages/Assignments.jsx';
import Insights from './pages/Insights.jsx';
import Scores from './pages/Scores.jsx';
import Courseware from './pages/Courseware.jsx';
import Grading from './pages/Grading.jsx';
import Dictation from './pages/Dictation.jsx';
import ImportPack from './pages/ImportPack.jsx';
import Generate from './pages/Generate.jsx';
import SyncPage from './pages/Sync.jsx';
import Settings from './pages/Settings.jsx';
import Help from './pages/Help.jsx';
import Profile from './pages/Profile.jsx';
import Wizard from './pages/Wizard.jsx';
import About from './pages/About.jsx';

// 主功能导航；设置/帮助/个人中心收进底部个人菜单。
// 默认顺序只是初始值：用户可以在设置里拖动调整（localStorage 持久化），
// 以后插件动态注册的菜单项只要往 NAV 里加条目，就会自动排在已知项后面。
const ICON = {
    library: 'M4 5.5A1.5 1.5 0 0 1 5.5 4H9a3 3 0 0 1 3 3v13a3 3 0 0 0-3-3H5.5A1.5 1.5 0 0 1 4 15.5v-10Zm16 0A1.5 1.5 0 0 0 18.5 4H15a3 3 0 0 0-3 3v13a3 3 0 0 1 3-3h3.5a1.5 1.5 0 0 0 1.5-1.5v-10Z',
    papers: 'M7 3.5h7.5L19 8v12.5H7zM14 3.5V8h5M9.5 11.5h6M9.5 14.5h6M9.5 17.5h3.5',
    capture: 'M4 8V6a2 2 0 0 1 2-2h2M16 4h2a2 2 0 0 1 2 2v2M20 16v2a2 2 0 0 1-2 2h-2M8 20H6a2 2 0 0 1-2-2v-2M4 12h16',
    import: 'M12 3v12m0 0 4-4m-4 4-4-4M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2',
    generate: 'M12 3l1.6 4.4L18 9l-4.4 1.6L12 15l-1.6-4.4L6 9l4.4-1.6L12 3Zm7 11 .8 2.2L22 17l-2.2.8L19 20l-.8-2.2L16 17l2.2-.8L19 14Z',
    classes: 'M9 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6Zm7 0a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5ZM3 19a6 6 0 0 1 12 0M14 19a5 5 0 0 1 7-4.6',
    stats: 'M4 20h16M7 20v-6m5 6V6m5 14v-9',
    insights: 'M4 20V10m5 10V4m5 16v-7m5 7V8',
    scores: 'M6 3h12v18l-6-4-6 4V3Zm3 5h6M9 11h6M9 14h3',
    courseware: 'M5 4.5A1.5 1.5 0 0 1 6.5 3H18a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H6.5A1.5 1.5 0 0 0 5 20.5v-16Zm0 16A1.5 1.5 0 0 1 6.5 22H19M9 8h6M9 12h4',
    grading: 'M4 6h16v11H4zM8 17v3m8-3v3M8 10h8M8 13h5',
    dictation: 'M4 9v6a2 2 0 0 0 2 2h2l1.5 2h5L16 17h2a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2Zm4 1h8m-8 3h5',
    sync: 'M20 11a8 8 0 1 0-2.3 6.1M20 5v6h-6',
    profile: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8Zm-7 8a7 7 0 0 1 14 0',
    settings: 'M12 15.3a3.3 3.3 0 1 0 0-6.6 3.3 3.3 0 0 0 0 6.6Zm8-3.3c0-.5-.05-1-.13-1.5l2-1.5-2-3.4-2.3.9a8 8 0 0 0-2.6-1.5L14.5 2h-4l-.4 2.9a8 8 0 0 0-2.6 1.5l-2.3-.9-2 3.4 2 1.5a8.6 8.6 0 0 0 0 3l-2 1.5 2 3.4 2.3-.9a8 8 0 0 0 2.6 1.5l.4 2.9h4l.4-2.9a8 8 0 0 0 2.6-1.5l2.3.9 2-3.4-2-1.5c.08-.5.13-1 .13-1.5Z',
    help: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Zm0-13a3 3 0 1 1-2.8 4M12 17h.01',
    about: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Zm0-13.2a1.15 1.15 0 1 0 0 2.3 1.15 1.15 0 0 0 0-2.3ZM12 12v5.2',
    logout: 'M10 5H6a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h4m4-4 3-3-3-3m3 3H9',
    chevron: 'm7 10 5 5 5-5',
};

function NavIcon({name}) {
    return (
        <svg className="nav-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor"
             strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            <path d={ICON[name]}/>
        </svg>
    );
}

// 同步是可选的连接能力，不占主工作台的位置，收进左下角账号菜单。
const NAV = [
    {key: 'library', label: '题库', hint: '查找、编辑、整理'},
    {key: 'stats', label: '统计', hint: '结构 · 补题建议'},
    {key: 'capture', label: '录入', hint: '截图 · PDF · 手工'},
    {key: 'import', label: '导入', hint: '题目包 · 粘贴 · 转换'},
    {key: 'generate', label: '生成', hint: 'AI 出题 · 批量补充'},
    {key: 'papers', label: '试卷', hint: '组卷 · 排版 · 答题卡'},
    {key: 'classes', label: '班级', hint: '学生 · 作业 · 批改'},
    {key: 'grading', label: '批改台', hint: '拍照 · 逐题给分'},
    {key: 'dictation', label: '听写', hint: '拼音 · 播报 · 核对'},
    {key: 'insights', label: '学情', hint: '掌握度 · 错题'},
    {key: 'scores', label: '成绩', hint: '考试 · 登分'},
    {key: 'courseware', label: '课件', hint: '讲义 · 例题 · 变式'},
];

const NAV_ORDER_KEY = 'neuralkb.nav.order.v1';
const DEFAULT_ORDER = NAV.map((item) => item.key);

/** 读用户拖过的菜单顺序；没有记录或记录里有新菜单（如插件项）时补齐默认位。 */
function loadNavOrder() {
    try {
        const raw = JSON.parse(localStorage.getItem(NAV_ORDER_KEY) || '[]');
        if (!Array.isArray(raw)) return DEFAULT_ORDER;
        const known = raw.filter((key) => DEFAULT_ORDER.includes(key));
        const rest = DEFAULT_ORDER.filter((key) => !known.includes(key));
        return [...known, ...rest];
    } catch {
        return DEFAULT_ORDER;
    }
}

const ACCOUNT_MENU = [
    {key: 'sync', label: '同步', icon: 'sync', hint: '连接中心题库'},
    {key: 'profile', label: '个人中心', icon: 'profile'},
    {key: 'settings', label: '设置', icon: 'settings'},
    {key: 'help', label: '帮助', icon: 'help'},
    {key: 'about', label: '关于我', icon: 'about', hint: '开发者 · 联系方式'},
];

const ALL_NAV = [...NAV, ...ACCOUNT_MENU];

function parseHash() {
    const raw = window.location.hash.replace(/^#\/?/, '');
    const [path, search] = raw.split('?');
    const [page, param, extra] = path.split('/');
    return {
        page: page || 'library',
        param: param ? decodeURIComponent(param) : '',
        // 第三段给「list/id」这种两级入口用（目前只有学情的个人页）。
        extra: extra ? decodeURIComponent(extra) : '',
        // 统计页的「去出题 / 去整理」要带着学科跳过去。
        query: Object.fromEntries(new URLSearchParams(search || '')),
    };
}

export default function App() {
    const [boot, setBoot] = useState(null);
    const [error, setError] = useState('');
    const [tree, setTree] = useState([]);
    const [route, setRoute] = useState(parseHash);
    const [captureTick, setCaptureTick] = useState(0);
    const [captureImagePath, setCaptureImagePath] = useState('');
    const [browserText, setBrowserText] = useState('');
    const [browserJob, setBrowserJob] = useState(null);
    const [showWizard, setShowWizard] = useState(false);
    const [auth, setAuth] = useState(null);
    const [accountOpen, setAccountOpen] = useState(false);
    const [syncPending, setSyncPending] = useState(0);
    const [navOrder, setNavOrderState] = useState(loadNavOrder);
    // 全局热键：即使听写台没打开，也把它带过去并送一个推进信号。
    const [dictationNextTick, setDictationNextTick] = useState(0);
    const [dictationRepeatTick, setDictationRepeatTick] = useState(0);

    /** 设置里拖完菜单：状态与 localStorage 一起落盘；未知 key 补在已知名后面。 */
    const setNavOrder = (order) => {
        const known = order.filter((key) => DEFAULT_ORDER.includes(key));
        const rest = DEFAULT_ORDER.filter((key) => !known.includes(key));
        const next = [...known, ...rest];
        setNavOrderState(next);
        try {
            localStorage.setItem(NAV_ORDER_KEY, JSON.stringify(next));
        } catch {
            // 存不进去（隐私模式等）就只对本次会话生效。
        }
    };

    const refresh = useCallback(async () => {
        try {
            const data = await bootstrap();
            setBoot(data);
            setAuth(data.auth);
            setShowWizard(!data.wizardDone);
            // 待同步数量只在左下角菜单里露一个数字，读不到就算了。
            syncStatus()
                .then((status) => setSyncPending(status.pending || 0))
                .catch(() => {});
            return data;
        } catch (err) {
            setError(err.message);
            return null;
        }
    }, []);

    const loadTree = useCallback(async () => {
        try {
            setTree(await curriculum());
        } catch (err) {
            setError(err.message);
        }
    }, []);

    useEffect(() => {
        refresh();
        loadTree();
    }, [refresh, loadTree]);

    useEffect(() => {
        const onHash = () => setRoute(parseHash());
        window.addEventListener('hashchange', onHash);
        return () => window.removeEventListener('hashchange', onHash);
    }, []);

    useEffect(() => {
        let cancelOne;
        let cancelTwo;
        let cancelThree;
        let cancelFour;
        Promise.all([
            listen('nb://capture', () => {
                window.location.hash = '#/capture';
                setCaptureTick((value) => value + 1);
            }),
            listen('nb://capture-image', (event) => {
                window.location.hash = '#/capture';
                setCaptureImagePath(event.payload || '');
                setCaptureTick((value) => value + 1);
            }),
            listen('nb://browser-text', (event) => {
                window.location.hash = '#/capture';
                setBrowserText(event.payload || '');
                setCaptureTick((value) => value + 1);
            }),
            // 内置浏览器抓到结构化题目：直接给录入页一个已完成任务，右侧逐题核对。
            listen('nb://browser-questions', (event) => {
                const payload = event.payload || {};
                if (!payload.jobId) return;
                window.location.hash = '#/capture';
                setBrowserJob({
                    jobId: payload.jobId,
                    count: payload.count || 0,
                    total: payload.total || payload.count || 0,
                    truncated: !!payload.truncated,
                    at: Date.now(),
                });
            }),
        ])
            .then(([a, b, c, d]) => {
                cancelOne = a;
                cancelTwo = b;
                cancelThree = c;
                cancelFour = d;
            })
            .catch(() => {});
        return () => {
            cancelOne?.();
            cancelTwo?.();
            cancelThree?.();
            cancelFour?.();
        };
    }, []);

    useEffect(() => {
        let offNext;
        let offRepeat;
        Promise.all([
            listen('nb://dictation-next', () => {
                window.location.hash = '#/dictation';
                setDictationNextTick((value) => value + 1);
            }),
            listen('nb://dictation-repeat', () => {
                window.location.hash = '#/dictation';
                setDictationRepeatTick((value) => value + 1);
            }),
        ])
            .then(([a, b]) => { offNext = a; offRepeat = b; })
            .catch(() => {});
        return () => { offNext?.(); offRepeat?.(); };
    }, []);

    const go = (key) => {
        setAccountOpen(false);
        window.location.hash = `#/${key}`;
    };

    const logout = async () => {
        setAccountOpen(false);
        try {
            await authLogout();
        } catch (error) {
            setError(error.message);
            return;
        }
        setAuth((current) => ({...current, unlocked: false}));
    };

    if (error) {
        return (
            <div className="fatal">
                <h1>启动失败</h1>
                <p>{error}</p>
                <p className="muted">数据目录里的 neuralkb.db 可能被占用。关掉其它窗口再试一次。</p>
            </div>
        );
    }
    if (!boot) {
        return (
            <div className="boot">
                <Loading text="正在打开本地题库…"/>
            </div>
        );
    }

    // 登录门禁优先于向导：先有账号、解锁，再谈其它。
    if (auth && (!auth.hasUser || !auth.unlocked)) {
        return (
            <>
                <LoginGate
                    auth={auth}
                    onUnlocked={(next) => setAuth(next)}
                />
                <FeedbackHost/>
            </>
        );
    }

    // 作业页挂在班级下面，侧栏仍然高亮「班级」。
    const known = ALL_NAV.some((item) => item.key === route.page) || route.page === 'assignments';
    const active = known ? route.page : 'library';
    const navActive = active === 'assignments' ? 'classes' : active;
    const pending = boot.stats.needsReview;
    const navItems = navOrder
        .map((key) => NAV.find((item) => item.key === key))
        .filter(Boolean);

    return (
        <div className="app">
            <aside className="sidebar">
                <button className="brand" onClick={() => go('library')}>
                    <BrandMark/>
                    <span className="brand-text">
                        <strong>NeuralKB</strong>
                        <small>本地题库</small>
                    </span>
                </button>

                <div className="nav-group-label">工作台</div>
                <nav className="main-nav">
                    {navItems.map((item) => (
                        <button
                            key={item.key}
                            className={`nav-item ${navActive === item.key ? 'active' : ''}`}
                            onClick={() => go(item.key)}
                        >
                            <NavIcon name={item.key}/>
                            <span className="nav-label">
                                {item.label}
                                {item.key === 'library' && pending > 0 && (
                                    <span className="nav-badge">{pending}</span>
                                )}
                            </span>
                            <span className="nav-hint">{item.hint}</span>
                        </button>
                    ))}
                </nav>

                <div className="sidebar-spacer"/>

                <div className="sidebar-stats">
                    <div className="stat-cell">
                        <strong>{boot.stats.total}</strong>
                        <span>题目</span>
                    </div>
                    <div className="stat-cell">
                        <strong>{boot.stats.withImages}</strong>
                        <span>带图</span>
                    </div>
                    <div className="stat-cell">
                        <strong className={pending ? 'hot' : ''}>{pending}</strong>
                        <span>草稿</span>
                    </div>
                </div>

                <div className="account">
                    {accountOpen && (
                        <>
                            <div className="account-scrim" onClick={() => setAccountOpen(false)}/>
                            <div className="account-menu">
                                <div className="account-menu-head">
                                    <span className="tail-avatar">{(auth.displayName || auth.username || '?').slice(0, 1).toUpperCase()}</span>
                                    <span className="account-menu-id">
                                        <strong>{auth.displayName || auth.username}</strong>
                                        <small>@{auth.username || '本地用户'}</small>
                                    </span>
                                </div>
                                <div className="account-menu-sep"/>
                                {ACCOUNT_MENU.map((item) => (
                                    <button
                                        key={item.key}
                                        className={`account-menu-item ${active === item.key ? 'active' : ''}`}
                                        onClick={() => go(item.key)}
                                    >
                                        <NavIcon name={item.icon}/>
                                        <span className="account-menu-text">
                                            {item.label}
                                            {item.hint && <small>{item.hint}</small>}
                                        </span>
                                        {item.key === 'sync' && syncPending > 0 && (
                                            <span className="menu-badge">{syncPending}</span>
                                        )}
                                    </button>
                                ))}
                                <div className="account-menu-sep"/>
                                <button className="account-menu-item danger" onClick={logout}>
                                    <NavIcon name="logout"/>
                                    <span>退出登录</span>
                                </button>
                            </div>
                        </>
                    )}
                    <button
                        className={`account-chip ${accountOpen ? 'open' : ''}`}
                        onClick={() => setAccountOpen((v) => !v)}
                        aria-expanded={accountOpen}
                    >
                        <span className="tail-avatar">{(auth.displayName || auth.username || '?').slice(0, 1).toUpperCase()}</span>
                        <span className="account-chip-meta">
                            <span className="tail-name">{auth.displayName || auth.username}</span>
                            <span className="tail-sub">本地账号</span>
                        </span>
                        <svg className={`tail-chev ${accountOpen ? 'up' : ''}`} viewBox="0 0 24 24" aria-hidden="true">
                            <path d={ICON.chevron}/>
                        </svg>
                    </button>
                    <p className="offline-note">
                        离线可用 · 数据只在本机 ·
                        <button type="button" className="about-mini-link" onClick={() => go('about')}>关于我</button>
                    </p>
                </div>
            </aside>

            <main className="main">
                {active === 'library' && (
                    <Library
                        tree={tree}
                        initialSubject={route.query?.subject || ''}
                        scopes={boot.stats}
                        onChanged={refresh}
                        onGo={go}
                    />
                )}
                {active === 'stats' && <Stats onGo={go}/>}
                {active === 'papers' &&
                    (route.param ? (
                        <PaperEditor
                            key={route.param}
                            paperId={route.param}
                            initialPreview={route.extra === 'preview'}
                            tree={tree}
                            onGo={go}
                            onChanged={refresh}
                        />
                    ) : (
                        <Papers tree={tree} onGo={go} onChanged={refresh}/>
                    ))}
                {active === 'capture' && (
                    <Capture
                        tree={tree}
                        trigger={captureTick}
                        externalPath={captureImagePath}
                        externalText={browserText}
                        externalWebJob={browserJob}
                        onChanged={refresh}
                    />
                )}
                {active === 'classes' && (
                    <Classes tree={tree} onGo={go} onChanged={refresh}/>
                )}
                {active === 'insights' && (
                    <Insights
                        tab={route.param === 'student' ? 'student' : 'class'}
                        studentId={route.param === 'student' ? route.extra : ''}
                        onGo={go}
                        onChanged={refresh}
                    />
                )}
                {active === 'scores' && (
                    <Scores onGo={go} onChanged={refresh}/>
                )}
                {active === 'courseware' && (
                    <Courseware tree={tree} onGo={go} onChanged={refresh}/>
                )}
                {active === 'grading' && (
                    <Grading onGo={go}/>
                )}
                {active === 'dictation' && (
                    <Dictation nextTick={dictationNextTick} repeatTick={dictationRepeatTick}/>
                )}
                {active === 'assignments' && (
                    <Assignments classId={route.param} onGo={go} onChanged={refresh}/>
                )}
                {active === 'import' && <ImportPack onChanged={refresh}/>}
                {active === 'generate' && (
                    <Generate
                        tree={tree}
                        initialSubject={route.query?.subject || ''}
                        initialDifficulty={route.query?.difficulty || ''}
                        initialTypes={route.query?.types || ''}
                        initialFocus={route.query?.focus || ''}
                        onChanged={refresh}
                        onGo={go}
                    />
                )}
                {active === 'sync' && <SyncPage onChanged={refresh}/>}
                {active === 'profile' && (
                    <Profile
                        auth={auth}
                        onAuthChanged={setAuth}
                        onLogout={() => setAuth((current) => ({...current, unlocked: false}))}
                        onGo={go}
                    />
                )}
                {active === 'help' && <Help slug={route.param}/>}
                {active === 'about' && <About/>}
                {active === 'settings' && (
                    <Settings
                        boot={boot}
                        navMeta={NAV}
                        navOrder={navOrder}
                        onNavOrder={setNavOrder}
                        onChanged={refresh}
                        onGo={go}
                    />
                )}
            </main>

            {showWizard && (
                <Wizard
                    boot={boot}
                    tree={tree}
                    onDone={async () => {
                        await refresh();
                        setShowWizard(false);
                    }}
                />
            )}
            <FeedbackHost/>
            <ImageLightbox/>
        </div>
    );
}
