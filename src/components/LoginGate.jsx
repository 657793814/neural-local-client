import {useState} from 'react';
import {authLogin, authSignup} from '../lib/api.js';
import {Field, notify} from './ui.jsx';
import BrandMark from './BrandMark.jsx';

/**
 * 启动门禁：没有账号时引导首次设置，有账号时登录。
 * 密码只经 IPC 送到 Rust 侧做 argon2 哈希，前端不留明文、不回显。
 */
export default function LoginGate({auth, onUnlocked}) {
    const signupMode = !auth.hasUser;
    const [username, setUsername] = useState('');
    const [password, setPassword] = useState('');
    const [password2, setPassword2] = useState('');
    const [displayName, setDisplayName] = useState('');
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState('');

    const submit = async (event) => {
        event.preventDefault();
        setError('');
        if (!username.trim()) {
            setError('请填写用户名');
            return;
        }
        if (!password) {
            setError('请填写密码');
            return;
        }
        if (signupMode) {
            if (password.length < 6) {
                setError('密码至少 6 位');
                return;
            }
            if (password !== password2) {
                setError('两次密码不一致');
                return;
            }
        }
        setBusy(true);
        try {
            const next = signupMode
                ? await authSignup(username, password, displayName)
                : await authLogin(username, password);
            notify.success(signupMode ? '本地账号已创建' : '欢迎回来');
            onUnlocked(next);
        } catch (e) {
            setError(e.message);
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="gate">
            <div className="gate-aurora" aria-hidden="true">
                <i/><i/><i/><i/>
            </div>
            <form className="gate-card liquid" onSubmit={submit}>
                <span className="liquid-caustics" aria-hidden="true"/>
                <span className="liquid-sheen" aria-hidden="true"/>
                <span className="liquid-specular" aria-hidden="true"/>
                <span className="liquid-edge" aria-hidden="true"/>
                <div className="gate-brand">
                    <BrandMark/>
                    <div>
                        <h1>NeuralKB 本地题库</h1>
                        <p>{signupMode ? '第一次使用，先设置一个本地账号' : '数据只在这台电脑上，登录后继续'}</p>
                    </div>
                </div>

                <Field label="用户名">
                    <input
                        autoFocus
                        value={username}
                        onChange={(e) => setUsername(e.target.value)}
                        placeholder="2–32 个字符"
                        autoComplete="username"
                    />
                </Field>

                {signupMode && (
                    <Field label="昵称（可选）" hint="留空就用用户名">
                        <input
                            value={displayName}
                            onChange={(e) => setDisplayName(e.target.value)}
                            placeholder="想让你怎么称呼？"
                        />
                    </Field>
                )}

                <Field label="密码" hint={signupMode ? '至少 6 位' : undefined}>
                    <input
                        type="password"
                        value={password}
                        onChange={(e) => setPassword(e.target.value)}
                        placeholder="••••••••"
                        autoComplete={signupMode ? 'new-password' : 'current-password'}
                    />
                </Field>

                {signupMode && (
                    <Field label="再输一次密码">
                        <input
                            type="password"
                            value={password2}
                            onChange={(e) => setPassword2(e.target.value)}
                            placeholder="••••••••"
                            autoComplete="new-password"
                        />
                    </Field>
                )}

                {error && <p className="error-text">{error}</p>}

                <button className="btn primary gate-submit" disabled={busy} type="submit">
                    {busy ? '请稍候…' : signupMode ? '创建账号并开始' : '登录'}
                </button>

                <p className="gate-note">
                    密码用 argon2 加密后存在本机，应用不会上传，也不提供找回。
                    {!signupMode && ' 忘记密码需要重置数据目录，请谨慎。'}
                </p>
            </form>
        </div>
    );
}
