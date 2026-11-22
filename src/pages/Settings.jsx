import {useEffect, useState} from 'react';
import {open} from '@tauri-apps/plugin-dialog';
import {
    backupExport,
    dataDirSet,
    dbExport,
    deleteModel,
    getRoutes,
    listModels,
    revealDataDir,
    saveModel,
    setRoute,
    settingGet,
    storageClean,
    storageUsage,
    testModel,
} from '../lib/api.js';
import {Card, Empty, Field, Modal, PageHead, Pill, confirmDialog, notify} from '../components/ui.jsx';
import ModelPicker from '../components/ModelPicker.jsx';
import {cx, formatBytes, pickSavePath} from '../lib/format.js';

const EMPTY_MODEL = {
    id: '',
    name: '',
    baseUrl: '',
    model: '',
    apiKey: '',
    supportsVision: false,
    enabled: true,
    note: '',
};

const SECTIONS = [
    {key: 'model', label: '模型服务', desc: '接口、密钥与用途路由'},
    {key: 'ui', label: '界面与菜单', desc: '左侧导航排序'},
    {key: 'data', label: '数据与备份', desc: '存储、目录与换机'},
    {key: 'app', label: '关于应用', desc: '版本与隐私边界'},
];

export default function Settings({boot, navMeta = [], navOrder, onNavOrder, onChanged, onGo}) {
    const [section, setSection] = useState('model');
    const [models, setModels] = useState([]);
    const [routes, setRoutes] = useState({purposes: [], routes: {}});
    const [editing, setEditing] = useState(null);
    const [testing, setTesting] = useState({});
    const [curriculumVersion, setCurriculumVersion] = useState('');
    const [backupPath, setBackupPath] = useState('');
    const [snapshotPath, setSnapshotPath] = useState('');

    const loadModels = () => listModels().then(setModels).catch((error) => notify.error(error.message));
    const loadRoutes = () => getRoutes().then(setRoutes).catch((error) => notify.error(error.message));

    useEffect(() => {
        loadModels();
        loadRoutes();
        settingGet('curriculum.version').then((value) => setCurriculumVersion(value || '')).catch(() => {});
    }, []);

    const test = async (model) => {
        setTesting((current) => ({...current, [model.id]: {busy: true}}));
        try {
            const result = await testModel(model.id);
            setTesting((current) => ({...current, [model.id]: result}));
        } catch (error) {
            setTesting((current) => ({...current, [model.id]: {ok: false, detail: error.message}}));
        }
    };

    const remove = async (model) => {
        const ok = await confirmDialog({
            title: '删除这个模型？',
            message: `「${model.name}」会从模型池与所有用途里移除，系统钥匙串里的 Key 也会删除。`,
            danger: true,
            confirmText: '删除',
        });
        if (!ok) return;
        try {
            await deleteModel(model.id);
            await loadModels();
            await loadRoutes();
            notify.success('已删除');
        } catch (error) {
            notify.error(error.message);
        }
    };

    const changeRoute = async (purpose, ids) => {
        try {
            await setRoute(purpose, ids);
            await loadRoutes();
        } catch (error) {
            notify.error(error.message);
        }
    };

    const move = (purpose, index, delta) => {
        const ids = [...(routes.routes[purpose] || [])];
        const target = index + delta;
        if (target < 0 || target >= ids.length) return;
        [ids[index], ids[target]] = [ids[target], ids[index]];
        changeRoute(purpose, ids);
    };

    // 菜单排序：拖到哪一行就落在哪一行；后续插件注册的菜单项也会出现在这个列表里。
    const dragNav = (event, index) => {
        event.dataTransfer.effectAllowed = 'move';
        event.dataTransfer.setData('text/plain', String(index));
    };
    const dropNav = (event, index) => {
        event.preventDefault();
        const from = Number(event.dataTransfer.getData('text/plain'));
        if (Number.isNaN(from) || from === index || !onNavOrder) return;
        const next = [...navOrder];
        const [moved] = next.splice(from, 1);
        next.splice(index, 0, moved);
        onNavOrder(next);
    };

    return (
        <div className="page">
            <PageHead title="设置" subtitle="模型、菜单、数据目录与备份都在这；Key 只存本机" help="models"/>

            <div className="settings-layout">
                <nav className="settings-nav">
                    {SECTIONS.map((item) => (
                        <button
                            type="button"
                            key={item.key}
                            className={section === item.key ? 'active' : ''}
                            onClick={() => setSection(item.key)}
                        >
                            <span className="settings-nav-mark" aria-hidden="true"/>
                            <span className="settings-nav-text">
                                <strong>{item.label}</strong>
                                <small>{item.desc}</small>
                            </span>
                        </button>
                    ))}
                </nav>
                <div className="settings-content">
                    <div className="settings-pane" hidden={section !== 'model'}>
            <Card
                title="模型"
                subtitle="不内置任何模型。填你服务商的 OpenAI 兼容地址（一般到 /v1），可以加多个"
                actions={
                    <button className="btn primary small" onClick={() => setEditing({...EMPTY_MODEL})}>
                        添加模型
                    </button>
                }
            >
                {models.length === 0 ? (
                    <Empty
                        title="还没有模型"
                        hint={
                            <span>
                                只录题、导入、搜索的话不需要模型。
                                <br/>
                                要用截图识别或 AI 出题，再添加一个 OpenAI 兼容端点。
                            </span>
                        }
                        action={
                            <div className="empty-actions">
                                <button className="btn primary" onClick={() => setEditing({...EMPTY_MODEL})}>
                                    添加模型
                                </button>
                                <button className="btn ghost" onClick={() => onGo('help')}>
                                    看怎么填
                                </button>
                            </div>
                        }
                    />
                ) : (
                    <div className="model-list">
                        {models.map((model) => {
                            const state = testing[model.id];
                            return (
                                <div className="model" key={model.id}>
                                    <div className="model-main">
                                        <div className="model-title">
                                            <strong>{model.name}</strong>
                                            {model.supportsVision && <Pill tone="line">支持图片</Pill>}
                                            {!model.enabled && <Pill tone="muted">已停用</Pill>}
                                            {model.hasKey ? <Pill tone="ok">Key 已保存</Pill> : <Pill tone="warn">没有 Key</Pill>}
                                        </div>
                                        <div className="model-meta">
                                            <span>{model.baseUrl}</span>
                                            <span>·</span>
                                            <span>{model.model}</span>
                                        </div>
                                        {model.note && <p className="muted small">{model.note}</p>}
                                        {state && !state.busy && (
                                            <p className={state.ok ? 'ok-text' : 'error-text'}>
                                                {state.ok ? `${state.detail}（${state.latencyMs}ms）` : state.detail}
                                            </p>
                                        )}
                                    </div>
                                    <div className="inline-actions">
                                        <button className="btn ghost small" onClick={() => test(model)} disabled={state?.busy}>
                                            {state?.busy ? '测试中…' : '测试连接'}
                                        </button>
                                        <button className="btn ghost small" onClick={() => setEditing({...model, apiKey: ''})}>
                                            编辑
                                        </button>
                                        <button className="btn ghost small" onClick={() => remove(model)}>
                                            删除
                                        </button>
                                    </div>
                                </div>
                            );
                        })}
                    </div>
                )}
                <div className="note-block">
                    <strong>怎么填</strong>
                    <ul className="plain-list">
                        <li>
                            <b>Base URL</b>：服务商文档里的接口地址，通常以 <code>/v1</code> 结尾，例如
                            <code>https://api.example.com/v1</code>。本地 Ollama 是 <code>http://127.0.0.1:11434/v1</code>。
                        </li>
                        <li>
                            <b>模型名</b>：填完地址和 Key 后点「拉取列表」，从服务商读回全部模型 ID 直接选，
                            下拉里还能按关键字过滤；读不到或想用别名时也可以手填。
                        </li>
                        <li>
                            <b>API Key</b>：只写进系统钥匙串，界面只显示「已保存」，任何时候都不会回传。
                            应用运行期间只向钥匙串要一次（之后走内存），所以批量补充不会反复弹授权框；
                            改完 Key 会立刻替换缓存，不用重启。
                        </li>
                        <li>
                            <b>支持图片</b>：只有勾上，截图识别才会用到它。
                        </li>
                    </ul>
                </div>
            </Card>

            <Card title="用途路由" subtitle="每个用途按顺序挑模型，失败自动换下一个；排在后面的不会被删掉">
                {models.length === 0 ? (
                    <Empty title="先添加模型" hint="加完之后这里会自动排好，再按需要调整顺序"/>
                ) : (
                    <div className="route-list">
                        {routes.purposes.map((purpose) => {
                            const ids = routes.routes[purpose.key] || [];
                            return (
                                <div className="route" key={purpose.key}>
                                    <div className="route-label">{purpose.label}</div>
                                    <div className="route-models">
                                        {ids.length === 0 && <span className="muted small">未配置，会按添加顺序挑</span>}
                                        {ids.map((id, index) => {
                                            const model = models.find((item) => item.id === id);
                                            return (
                                                <span className="route-chip" key={id}>
                                                    {model?.name || '已删除的模型'}
                                                    <button className="mini" onClick={() => move(purpose.key, index, -1)}>
                                                        ↑
                                                    </button>
                                                    <button className="mini" onClick={() => move(purpose.key, index, 1)}>
                                                        ↓
                                                    </button>
                                                    <button
                                                        className="mini"
                                                        onClick={() => changeRoute(purpose.key, ids.filter((each) => each !== id))}
                                                    >
                                                        ✕
                                                    </button>
                                                </span>
                                            );
                                        })}
                                        <select
                                            value=""
                                            onChange={(event) => {
                                                const id = event.target.value;
                                                if (id && !ids.includes(id)) changeRoute(purpose.key, [...ids, id]);
                                            }}
                                        >
                                            <option value="">+ 加模型</option>
                                            {models.map((model) => (
                                                <option key={model.id} value={model.id}>
                                                    {model.name}
                                                </option>
                                            ))}
                                        </select>
                                    </div>
                                </div>
                            );
                        })}
                    </div>
                )}
            </Card>
                    </div>

                    <div className="settings-pane" hidden={section !== 'ui'}>
            <Card
                title="菜单排序"
                subtitle="拖动调整左侧导航的顺序，改动立刻生效并记在这台机器上"
                actions={
                    onNavOrder && navMeta.length > 0 && (
                        <button
                            className="btn ghost small"
                            onClick={() => onNavOrder(navMeta.map((item) => item.key))}
                        >
                            恢复默认
                        </button>
                    )
                }
            >
                {navOrder.length === 0 ? (
                    <Empty title="没有可排序的菜单"/>
                ) : (
                    <ul className="nav-order-list">
                        {navOrder.map((key, index) => {
                            const item = navMeta.find((entry) => entry.key === key);
                            return (
                                <li
                                    key={key}
                                    className="nav-order-item"
                                    draggable
                                    onDragStart={(event) => dragNav(event, index)}
                                    onDragOver={(event) => event.preventDefault()}
                                    onDrop={(event) => dropNav(event, index)}
                                    title="按住拖动调整顺序"
                                >
                                    <span className="nav-order-grip" aria-hidden="true">
                                        ⠿
                                    </span>
                                    <span className="nav-order-label">{item?.label || key}</span>
                                    {item?.hint && <span className="nav-order-hint">{item.hint}</span>}
                                    <span className="nav-order-pos">{index + 1}</span>
                                </li>
                            );
                        })}
                    </ul>
                )}
            </Card>
                    </div>

                    <div className="settings-pane" hidden={section !== 'data'}>
            <StorageCard/>

            <Card title="数据与备份" subtitle="题库是一个文件夹：换电脑只需整份拷走">
                <div className="kv">
                    <span>数据目录</span>
                    <code>{boot.dataDir}</code>
                </div>
                <div className="kv">
                    <span>内置教材目录版本</span>
                    <code>{curriculumVersion || '随安装包'}</code>
                </div>
                <div className="kv">
                    <span>检索方式</span>
                    <code>{boot.searchMode === 'trigram' ? 'SQLite FTS5（中文按字匹配）' : 'SQLite LIKE'}</code>
                </div>
                <div className="inline-actions">
                    <button className="btn ghost" onClick={() => revealDataDir()}>
                        在 Finder 里打开
                    </button>
                    <button
                        className="btn ghost"
                        onClick={async () => {
                            const dir = await open({directory: true, multiple: false});
                            if (!dir) return;
                            const ok = await confirmDialog({
                                title: '把数据搬到新目录？',
                                message: `当前题库会复制到 ${dir}，然后应用自动重启。原目录会保留，确认没问题再自己删。`,
                                confirmText: '复制并重启',
                            });
                            if (!ok) return;
                            try {
                                await dataDirSet(dir);
                            } catch (error) {
                                notify.error(error.message);
                            }
                        }}
                    >
                        更换目录
                    </button>
                    <button
                        className="btn ghost"
                        onClick={async () => {
                            try {
                                const stamp = new Date().toISOString().slice(0, 10).replace(/-/g, '');
                                const path = await pickSavePath(`neuralkb-${stamp}.db`, [
                                    {name: 'SQLite 数据库', extensions: ['db', 'sqlite']},
                                ]);
                                if (!path) return;
                                const saved = await dbExport(path);
                                setSnapshotPath(saved);
                                notify.success('数据库副本已导出');
                            } catch (error) {
                                notify.error(error.message);
                            }
                        }}
                    >
                        导出数据库副本
                    </button>
                    <button
                        className="btn primary"
                        onClick={async () => {
                            try {
                                const path = await backupExport();
                                setBackupPath(path);
                                notify.success('备份已生成');
                            } catch (error) {
                                notify.error(error.message);
                            }
                        }}
                    >
                        导出整库备份
                    </button>
                </div>
                {backupPath && (
                    <p className="muted small">
                        备份文件：<code>{backupPath}</code>（含数据库与所有图片）
                    </p>
                )}
                {snapshotPath && (
                    <p className="muted small">
                        数据库副本：<code>{snapshotPath}</code>
                    </p>
                )}
                <p className="muted small">
                    提示：备份包是普通 ZIP，换机时解压到任意目录，再在「更换目录」里指过去即可。
                    「导出数据库副本」是给 Navicat / DB Browser 看数据用的单文件快照——
                    直接打开数据目录里的 <code>neuralkb.db</code> 常常因为 WAL 读到旧数据或打不开，
                    这个副本没有这个问题；改副本不会影响题库。
                </p>
            </Card>
                    </div>

                    <div className="settings-pane" hidden={section !== 'app'}>
            <Card title="关于" subtitle="应用信息与使用边界">
                <div className="kv">
                    <span>版本</span>
                    <code>{boot.appVersion}</code>
                </div>
                <div className="kv">
                    <span>平台</span>
                    <code>{boot.platform}</code>
                </div>
                <div className="kv">
                    <span>许可</span>
                    <code>PolyForm Noncommercial 1.0.0</code>
                </div>
                <p className="muted small">
                    本地题库不会上传任何内容。只有在你主动点「识别 / 出题 / 归类」时，题目内容会发给你自己配置的模型服务商；
                    开启同步后才会把题目推到中心系统。应用不收集使用日志。
                </p>
                <div className="inline-actions">
                    <a className="btn ghost" href="#/help">
                        打开帮助
                    </a>
                </div>
            </Card>
                    </div>
                </div>
            </div>

            {editing && (
                <ModelForm
                    model={editing}
                    onClose={() => setEditing(null)}
                    onSaved={async () => {
                        setEditing(null);
                        await loadModels();
                        await loadRoutes();
                        onChanged?.();
                    }}
                />
            )}
        </div>
    );
}

function ModelForm({model, onClose, onSaved}) {
    const [form, setForm] = useState({...model});
    const [saving, setSaving] = useState(false);

    const submit = async () => {
        setSaving(true);
        try {
            await saveModel({
                id: form.id || null,
                name: form.name,
                baseUrl: form.baseUrl,
                model: form.model,
                supportsVision: form.supportsVision,
                enabled: form.enabled,
                note: form.note || null,
                apiKey: form.apiKey || null,
            });
            notify.success('已保存');
            onSaved();
        } catch (error) {
            notify.error(error.message);
        } finally {
            setSaving(false);
        }
    };

    return (
        <Modal
            title={model.id ? '编辑模型' : '添加模型'}
            subtitle="只支持 OpenAI 兼容接口（/chat/completions）"
            onClose={onClose}
            footer={
                <>
                    <button className="btn ghost" onClick={onClose}>
                        取消
                    </button>
                    <button className="btn primary" onClick={submit} disabled={saving}>
                        {saving ? '保存中…' : '保存'}
                    </button>
                </>
            }
        >
            <div className="form-grid">
                <Field label="名称" hint="自己认得出就行，例如「主力视觉模型」">
                    <input value={form.name} onChange={(event) => setForm({...form, name: event.target.value})}/>
                </Field>
                <Field label="备注" hint="例如额度、限速，给自己看的">
                    <input value={form.note || ''} onChange={(event) => setForm({...form, note: event.target.value})}/>
                </Field>
                <Field label="Base URL" wide hint="一般以 /v1 结尾">
                    <input
                        value={form.baseUrl}
                        placeholder="https://api.example.com/v1"
                        onChange={(event) => setForm({...form, baseUrl: event.target.value})}
                    />
                </Field>
                <Field
                    label="API Key"
                    wide
                    hint={model.hasKey ? '已保存；留空表示不修改' : '只写进系统钥匙串，不会显示第二次'}
                >
                    <input
                        type="password"
                        value={form.apiKey}
                        placeholder={model.hasKey ? '已保存（留空不改）' : 'sk-…'}
                        onChange={(event) => setForm({...form, apiKey: event.target.value})}
                    />
                </Field>
                <ModelPicker form={form} setForm={setForm} savedId={model.id}/>
                <div className="checks wide">
                    <label className="check">
                        <input
                            type="checkbox"
                            checked={form.supportsVision}
                            onChange={(event) => setForm({...form, supportsVision: event.target.checked})}
                        />
                        支持图片（截图识别要用）
                    </label>
                    <label className="check">
                        <input
                            type="checkbox"
                            checked={form.enabled}
                            onChange={(event) => setForm({...form, enabled: event.target.checked})}
                        />
                        启用
                    </label>
                </div>
            </div>
        </Modal>
    );
}

/**
 * 本机存储：把「过程图」和「题目图」分开摆出来。
 * 清理入口只对 tmp / 无主图片 / 旧备份开放，数据库和题目引用的图没有删除路径。
 */
function StorageCard() {
    const [store, setStore] = useState(null);
    const [error, setError] = useState('');
    const [busy, setBusy] = useState('');

    const load = async () => {
        try {
            setStore(await storageUsage());
            setError('');
        } catch (err) {
            setError(err.message);
        }
    };

    useEffect(() => {
        load();
    }, []);

    const clean = async (kind) => {
        const copy = {
            tmp: {
                title: '清理临时文件？',
                message: `会删掉 tmp/ 里的截图、PDF 页渲染和格式转换中间图，约 ${formatBytes(store.tmp.bytes)}。最近 10 分钟内生成的文件、以及连拍队列里等识别的图会保留。题目里的图在 assets/，不受影响。`,
                confirmText: '清理临时文件',
                danger: false,
            },
            orphans: {
                title: '清理无主图片？',
                message: `assets/ 里有 ${store.orphanFiles} 个文件没有被任何题目引用（约 ${formatBytes(store.orphanBytes)}），通常是导入中断或题目彻底删除时留下的。被题目引用的图一张都不会动。`,
                confirmText: '清理无主图片',
                danger: true,
            },
            backups: {
                title: '清理旧备份？',
                message: `backup/ 里只保留最近 3 份整库备份，更老的会删除，约释放 ${formatBytes(store.backups.bytes)} 中的一部分。最近 3 份不受影响。`,
                confirmText: '清理旧备份',
                danger: false,
            },
        }[kind];
        const ok = await confirmDialog({
            title: copy.title,
            message: copy.message,
            confirmText: copy.confirmText,
            danger: copy.danger,
        });
        if (!ok) return;
        setBusy(kind);
        try {
            const result = await storageClean(kind);
            const kept = result.kept > 0 ? `，${result.kept} 项太新已保留` : '';
            const pinned = result.pinned > 0 ? `，${result.pinned} 张在队列中已保留` : '';
            const failed = result.errors > 0 ? `，${result.errors} 项没删掉` : '';
            notify.success(`已清理 ${result.removed} 项，释放 ${formatBytes(result.freedBytes)}${kept}${pinned}${failed}`);
            await load();
        } catch (err) {
            notify.error(err.message);
        } finally {
            setBusy('');
        }
    };

    const segments = store
        ? [
              {key: 'assets', tone: 'assets', bytes: store.assets.bytes, label: '题目图片'},
              {key: 'db', tone: 'db', bytes: store.db.bytes, label: '数据库'},
              {key: 'tmp', tone: 'tmp', bytes: store.tmp.bytes, label: '临时文件'},
              {key: 'backups', tone: 'backups', bytes: store.backups.bytes, label: '备份'},
              {key: 'other', tone: 'other', bytes: store.other.bytes, label: '其他'},
          ].filter((part) => part.bytes > 0)
        : [];
    const totalBytes = store?.total.bytes || 0;
    const usedPercent =
        store?.diskTotal && store.diskFree != null
            ? Math.min(100, Math.max(0, ((store.diskTotal - store.diskFree) / store.diskTotal) * 100))
            : 0;

    return (
        <Card
            title="本机存储"
            subtitle="题目图片与过程文件分开算：截图、PDF 预览的中间图随手可清，题目里的图一张不动"
            actions={
                <div className="inline-actions">
                    <a className="btn ghost small" href="#/help/backup">
                        说明
                    </a>
                    <button className="btn ghost small" onClick={load} disabled={!!busy}>
                        重新统计
                    </button>
                </div>
            }
        >
            {error && <p className="error-text">{error}</p>}
            {!store ? (
                <p className="muted small">正在统计…</p>
            ) : (
                <>
                    <div className="store-top">
                        <div className="store-total">
                            <strong>{formatBytes(totalBytes)}</strong>
                            <span>题库共 {store.total.files} 个文件</span>
                        </div>
                        {store.diskFree != null && (
                            <div className="store-disk">
                                <span>
                                    磁盘剩余 <b>{formatBytes(store.diskFree)}</b>
                                    <span className="muted"> / 共 {formatBytes(store.diskTotal)}</span>
                                </span>
                                <div className="store-track">
                                    <i style={{width: `${usedPercent}%`}}/>
                                </div>
                                <span className="muted small">已用 {usedPercent.toFixed(0)}%</span>
                            </div>
                        )}
                    </div>

                    <div className="store-bar" role="img" aria-label="存储构成">
                        {segments.map((part) => (
                            <i
                                key={part.key}
                                className={cx('store-seg', part.tone)}
                                style={{width: `${(part.bytes / Math.max(1, totalBytes)) * 100}%`}}
                                title={`${part.label} ${formatBytes(part.bytes)}`}
                            />
                        ))}
                    </div>

                    <div className="store-rows">
                        <StorageRow
                            tone="assets"
                            title="题目图片"
                            path="assets/"
                            detail={`${store.assetsQuestions} 道题在用 · 题干和解析引用的图，导出、备份都靠它`}
                            bytes={store.assets.bytes}
                            files={store.assets.files}
                            action={
                                store.orphanFiles > 0 ? (
                                    <button
                                        className="btn ghost small"
                                        onClick={() => clean('orphans')}
                                        disabled={!!busy}
                                    >
                                        {busy === 'orphans' ? '清理中…' : `清理无主图片（${store.orphanFiles}）`}
                                    </button>
                                ) : (
                                    <Pill tone="ok">没有多余图片</Pill>
                                )
                            }
                        />
                        <StorageRow
                            tone="tmp"
                            title="临时文件"
                            path="tmp/"
                            detail={`截图、PDF 页渲染、格式转换的中间图 · 最近 10 分钟${
                                store.tmpPinned > 0 ? ` 与队列里的 ${store.tmpPinned} 张` : ''
                            }自动保留`}
                            bytes={store.tmp.bytes}
                            files={store.tmp.files}
                            action={
                                <button
                                    className="btn ghost small"
                                    onClick={() => clean('tmp')}
                                    disabled={!!busy || store.tmp.files === 0}
                                >
                                    {busy === 'tmp' ? '清理中…' : store.tmp.files === 0 ? '没有临时文件' : '清理临时文件'}
                                </button>
                            }
                        />
                        <StorageRow
                            tone="db"
                            title="数据库"
                            path="neuralkb.db"
                            detail="本地 SQLite 文件，没有账号密码，靠系统文件权限保护"
                            bytes={store.db.bytes}
                            files={store.db.files}
                            action={<span className="muted small">随题库备份</span>}
                        />
                        <StorageRow
                            tone="backups"
                            title="备份包"
                            path="backup/"
                            detail="整库备份 ZIP，只保留最近 3 份"
                            bytes={store.backups.bytes}
                            files={store.backups.files}
                            action={
                                store.backups.files > 3 ? (
                                    <button
                                        className="btn ghost small"
                                        onClick={() => clean('backups')}
                                        disabled={!!busy}
                                    >
                                        {busy === 'backups' ? '清理中…' : '清理旧备份'}
                                    </button>
                                ) : (
                                    <Pill tone="muted">不足 4 份</Pill>
                                )
                            }
                        />
                        <StorageRow
                            tone="other"
                            title="其他"
                            path=""
                            detail="日志、检索索引等零散文件"
                            bytes={store.other.bytes}
                            files={store.other.files}
                        />
                    </div>

                    <div className="note-block">
                        <strong>为什么截图能清、题目图不能清</strong>
                        <ul className="plain-list">
                            <li>
                                截图、PDF 翻页、HEIC 转换都先落在 <code>tmp/</code>，识别完就没用了；入题时会把用到的图复制一份到{' '}
                                <code>assets/&lt;题目 id&gt;/</code>，原件才允许删。
                            </li>
                            <li>
                                <code>assets/</code> 里是题目正文引用的图，删了题目会缺图，导出和备份也会少图，
                                所以「清理临时文件」不会碰这个目录。
                            </li>
                            <li>
                                本地数据库没有账号密码：<code>neuralkb.db</code> 就是一个文件，能打开数据目录就能读。
                                防的是别人拿到这台电脑（用系统账户与磁盘加密），不是数据库连接。
                            </li>
                        </ul>
                    </div>
                </>
            )}
        </Card>
    );
}

function StorageRow({tone, title, path, detail, bytes, files, action}) {
    return (
        <div className="store-row">
            <span className={cx('store-dot', tone)}/>
            <div className="store-main">
                <div className="store-name">
                    {title}
                    {path && <code>{path}</code>}
                </div>
                <div className="store-detail">{detail}</div>
            </div>
            <div className="store-size">
                <b>{formatBytes(bytes)}</b>
                <span>{files} 个文件</span>
            </div>
            {action && <div className="store-act">{action}</div>}
        </div>
    );
}
