import {useEffect, useState} from 'react';
import {syncLogin, syncLogout, syncPull, syncPush, syncRetry, syncStatus} from '../lib/api.js';
import {Card, Empty, Field, PageHead, Pill, confirmDialog, notify} from '../components/ui.jsx';

export default function SyncPage({onChanged}) {
    const [status, setStatus] = useState(null);
    const [form, setForm] = useState({server: '', username: '', password: '', deviceName: '我的电脑'});
    const [busy, setBusy] = useState(false);
    const [password2, setPassword2] = useState(false);

    const load = () => syncStatus().then(setStatus).catch((error) => notify.error(error.message));

    useEffect(() => {
        load();
    }, []);

    const login = async () => {
        setBusy(true);
        try {
            const next = await syncLogin(form.server, form.username, form.password, form.deviceName);
            setStatus(next);
            setForm((current) => ({...current, password: ''}));
            notify.success('已登录，之后可以按需推送题目');
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    const run = async (action, label) => {
        setBusy(true);
        try {
            const result = await action();
            notify.success(
                label === 'push'
                    ? `推送完成：${result.pushed} 道`
                    : `拉取完成：${result.pulled} 道有更新`,
            );
            await load();
            onChanged?.();
        } catch (error) {
            notify.error(error.message);
            await load();
        } finally {
            setBusy(false);
        }
    };

    if (!status) return <div className="page"><div className="loading">正在读同步状态…</div></div>;

    const configured = status.configured && status.enabled;

    return (
        <div className="page">
            <PageHead
                title="同步"
                subtitle="可选功能：把题目推到你自己的中心题库，方便多设备汇总；这一页在左下角账号菜单里"
                help="sync"
            />

            {!configured ? (
                <Card title="连接中心系统" subtitle="用中心系统的账号登录，本机会拿到一个设备令牌">
                    <div className="form-grid">
                        <Field label="服务器地址" wide hint="中心系统的访问地址，必须 https">
                            <input
                                value={form.server}
                                placeholder="https://kb.example.com"
                                onChange={(event) => setForm({...form, server: event.target.value})}
                            />
                        </Field>
                        <Field label="账号">
                            <input
                                value={form.username}
                                onChange={(event) => setForm({...form, username: event.target.value})}
                            />
                        </Field>
                        <Field label="密码">
                            <input
                                type="password"
                                value={form.password}
                                onChange={(event) => setForm({...form, password: event.target.value})}
                            />
                        </Field>
                        <Field label="设备名称" wide hint="方便在管理台认出是哪台电脑">
                            <input
                                value={form.deviceName}
                                onChange={(event) => setForm({...form, deviceName: event.target.value})}
                            />
                        </Field>
                    </div>
                    <div className="row-end">
                        <button
                            className="btn primary"
                            onClick={login}
                            disabled={busy || !form.server || !form.username || !form.password}
                        >
                            {busy ? '登录中…' : '登录'}
                        </button>
                    </div>
                    <div className="note-block">
                        <strong>关于服务端</strong>
                        <p className="muted small">
                            同步走 <code>/api/desktop/**</code>（账号换设备令牌 + 增量推拉）。中心系统还没升级到这个版本时，
                            登录会提示「没有该接口」——本地题库不受影响，可以继续离线使用。
                        </p>
                    </div>
                </Card>
            ) : (
                <>
                    <Card
                        title={status.username || '已登录'}
                        subtitle={status.serverUrl}
                        actions={
                            <button
                                className="btn ghost small"
                                onClick={async () => {
                                    const ok = await confirmDialog({
                                        title: '退出同步？',
                                        message: '会吊销本机设备令牌。本地题目不会删除，也不会再自动推送。',
                                        confirmText: '退出',
                                    });
                                    if (!ok) return;
                                    try {
                                        setStatus(await syncLogout());
                                        notify.success('已退出同步');
                                    } catch (error) {
                                        notify.error(error.message);
                                    }
                                }}
                            >
                                退出登录
                            </button>
                        }
                    >
                        <div className="sync-grid">
                            <div className="sync-stat">
                                <strong>{status.pending}</strong>
                                <span>待推送</span>
                            </div>
                            <div className="sync-stat">
                                <strong>{status.failed}</strong>
                                <span>失败</span>
                            </div>
                            <div className="sync-stat">
                                <strong>{status.synced}</strong>
                                <span>已同步</span>
                            </div>
                        </div>
                        {status.lastError && <p className="error-text">上次错误：{status.lastError}</p>}
                        <div className="inline-actions">
                            <button className="btn primary" onClick={() => run(syncPush, 'push')} disabled={busy}>
                                立即推送
                            </button>
                            <button className="btn ghost" onClick={() => run(syncPull, 'pull')} disabled={busy}>
                                拉取更新
                            </button>
                            <button
                                className="btn ghost"
                                onClick={async () => {
                                    await syncRetry();
                                    await load();
                                    notify.info('失败记录已重置，下次推送会重试');
                                }}
                                disabled={busy}
                            >
                                重试失败
                            </button>
                        </div>
                        <p className="muted small">
                            默认只推「本地 / 待同步」的题，服务端写入时归属到你自己，仅本人与管理员可见。
                            冲突不自动合并：服务端版本保留，本地另存一份并标红。
                        </p>
                    </Card>

                    <Card title="同步范围" subtitle="P0 只同步题目与图片">
                        <ul className="plain-list">
                            <li>题目：题干、选项、答案、解析、难度、标签、知识点路径。</li>
                            <li>图片：先上传拿到 assetId，再随题目引用。</li>
                            <li>不同步：笔记、试卷、练习册（P2 再评估）。</li>
                        </ul>
                        <p className="muted small">
                            服务端尚未部署时，这里的按钮会明确报错，不会假装成功。
                        </p>
                    </Card>
                </>
            )}

            {!status.configured && (
                <Card title="不想连服务器？" subtitle="完全可以">
                    <Empty
                        title="离线也能用"
                        hint="题库、导入、搜索、备份全部在本地。只有 AI 识别与出题需要联网，而且只连你自己配置的模型。"
                    />
                </Card>
            )}
        </div>
    );
}
