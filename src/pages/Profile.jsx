import {useEffect, useState} from 'react';
import {
    appInfo,
    authChangePassword,
    authLogout,
    authUpdateProfile,
} from '../lib/api.js';
import {Card, Field, PageHead, notify} from '../components/ui.jsx';

/** 个人中心：展示本地资料、改昵称/签名、改密码、退出登录。 */
export default function Profile({auth, onAuthChanged, onLogout, onGo}) {
    const [displayName, setDisplayName] = useState(auth.displayName || '');
    const [bio, setBio] = useState(auth.bio || '');
    const [oldPassword, setOldPassword] = useState('');
    const [newPassword, setNewPassword] = useState('');
    const [newPassword2, setNewPassword2] = useState('');
    const [savingProfile, setSavingProfile] = useState(false);
    const [savingPassword, setSavingPassword] = useState(false);
    const [info, setInfo] = useState(null);

    useEffect(() => {
        appInfo().then(setInfo).catch(() => {});
    }, []);

    const saveProfile = async (event) => {
        event.preventDefault();
        setSavingProfile(true);
        try {
            const next = await authUpdateProfile(displayName, bio);
            onAuthChanged?.(next);
            notify.success('资料已保存');
        } catch (e) {
            notify.error(e.message);
        } finally {
            setSavingProfile(false);
        }
    };

    const savePassword = async (event) => {
        event.preventDefault();
        if (newPassword.length < 6) {
            notify.warning('新密码至少 6 位');
            return;
        }
        if (newPassword !== newPassword2) {
            notify.warning('两次密码不一致');
            return;
        }
        setSavingPassword(true);
        try {
            await authChangePassword(oldPassword, newPassword);
            setOldPassword('');
            setNewPassword('');
            setNewPassword2('');
            notify.success('密码已更新，请记住新密码');
        } catch (e) {
            notify.error(e.message);
        } finally {
            setSavingPassword(false);
        }
    };

    const logout = async () => {
        await authLogout();
        onLogout();
    };

    return (
        <div className="page">
            <PageHead
                title="个人中心"
                subtitle="这台电脑上的本地账号，账号信息不会上传"
            />

            <div className="profile-grid">
                <Card title="我的资料" subtitle="只显示在这台电脑上">
                    <div className="profile-avatar">
                        <span>{(auth.displayName || auth.username || '?').slice(0, 1).toUpperCase()}</span>
                    </div>
                    <div className="profile-name">{auth.displayName || auth.username}</div>
                    <div className="muted small">@{auth.username}</div>
                    {auth.bio && <p className="profile-bio">{auth.bio}</p>}

                    <form className="profile-form" onSubmit={saveProfile}>
                        <Field label="昵称">
                            <input value={displayName} onChange={(e) => setDisplayName(e.target.value)}/>
                        </Field>
                        <Field label="签名">
                            <textarea
                                rows={2}
                                value={bio}
                                onChange={(e) => setBio(e.target.value)}
                                placeholder="写一句给自己看的话"
                            />
                        </Field>
                        <button className="btn primary" disabled={savingProfile} type="submit">
                            {savingProfile ? '保存中…' : '保存资料'}
                        </button>
                    </form>
                </Card>

                <div className="profile-stack">
                    <Card title="修改密码" subtitle="需要输入当前密码，改完保持登录">
                        <form className="profile-form" onSubmit={savePassword}>
                            <Field label="当前密码">
                                <input
                                    type="password"
                                    value={oldPassword}
                                    onChange={(e) => setOldPassword(e.target.value)}
                                    autoComplete="current-password"
                                />
                            </Field>
                            <Field label="新密码" hint="至少 6 位">
                                <input
                                    type="password"
                                    value={newPassword}
                                    onChange={(e) => setNewPassword(e.target.value)}
                                    autoComplete="new-password"
                                />
                            </Field>
                            <Field label="再输一次新密码">
                                <input
                                    type="password"
                                    value={newPassword2}
                                    onChange={(e) => setNewPassword2(e.target.value)}
                                    autoComplete="new-password"
                                />
                            </Field>
                            <button className="btn ghost" disabled={savingPassword} type="submit">
                                {savingPassword ? '修改中…' : '更新密码'}
                            </button>
                        </form>
                    </Card>

                    <Card title="这台设备" subtitle="本地存储信息">
                        {info ? (
                            <div className="profile-info">
                                <div><span>应用版本</span><strong>{info.version}</strong></div>
                                <div><span>数据目录</span><strong className="mono" title={info.dataDir}>{info.dataDir}</strong></div>
                                <div><span>平台</span><strong>{info.platform} · {info.arch}</strong></div>
                                <div><span>账号创建于</span><strong>{auth.createdAt ? auth.createdAt.slice(0, 10) : '—'}</strong></div>
                            </div>
                        ) : (
                            <p className="muted">读取中…</p>
                        )}
                        <div className="profile-actions">
                            <button className="btn ghost small" onClick={() => onGo('settings')}>数据与备份</button>
                            <button className="btn ghost small" onClick={logout}>退出登录</button>
                        </div>
                    </Card>
                </div>
            </div>

            <p className="profile-foot muted small">
                密码用 argon2 加密存在本机，应用不收集使用日志。忘记密码需要到数据目录重置，请务必记住。
            </p>
        </div>
    );
}
