import {useState} from 'react';
import wechatQr from '../assets/wechat-qr.jpg';
import {Card, Modal, PageHead, notify} from '../components/ui.jsx';
import {Typewriter} from '../components/effects.jsx';

// 联系方式集中放在这里，改完下面的页面会自动更新，不必动结构。
const CONTACT = {
    name: '咚咚锵',
    title: 'NeuralKB 本地题库 · 开发者',
    bio: '做这款工具，是为了把散落的学习资料和题目整理成一台电脑上就能用的知识库——离线优先，数据只留在自己手里。',
    wechatHint: '扫码添加微信好友，交流使用问题与功能建议。',
    projectUrl: 'https://github.com/657793814/neural-local-client',
};

// 开发者卡底部的四条事实，写法保守，避免和实际版本对不上。
const FACTS = [
    {label: '版本', value: 'v0.1.0'},
    {label: '运行方式', value: '本地离线'},
    {label: '数据', value: '只存在本机'},
    {label: '更新', value: 'GitHub 下载'},
];

/** 关于我：开发者介绍与联系方式，从左下角账号菜单进入。 */
export default function About() {
    const [qrOpen, setQrOpen] = useState(false);

    const copyProject = async () => {
        try {
            await navigator.clipboard.writeText(CONTACT.projectUrl);
            notify.success('项目地址已复制');
        } catch {
            notify.error('复制失败，请手动选中链接复制');
        }
    };

    return (
        <div className="page about-page">
            <PageHead title="关于我" subtitle="开发者与联系方式"/>

            <div className="about-grid">
                <Card
                    className="about-card about-card--dev about-flow"
                    title="开发者"
                    subtitle="这款工具背后的一个人"
                    tilt
                >
                    <span className="flow-ring" aria-hidden="true"/>
                    <span className="flow-sheen" aria-hidden="true"/>

                    <div className="about-block">
                        <div className="about-person">
                            <span className="about-avatar">{CONTACT.name.slice(0, 1)}</span>
                            <span className="about-person-meta">
                                <strong>{CONTACT.name}</strong>
                                <small>{CONTACT.title}</small>
                            </span>
                        </div>
                        <p className="about-bio"><Typewriter text={CONTACT.bio}/></p>
                    </div>

                    <div className="about-divider"/>

                    <div className="about-block">
                        <div className="about-contact-head">
                            <span className="about-contact-name">项目地址</span>
                            <span className="muted small">源码与版本记录</span>
                        </div>
                        {CONTACT.projectUrl ? (
                            <div className="about-project">
                                <a
                                    className="about-link"
                                    href={CONTACT.projectUrl}
                                    target="_blank"
                                    rel="noreferrer"
                                >
                                    <svg viewBox="0 0 24 24" aria-hidden="true">
                                        <path d="M12 2.6a9.4 9.4 0 0 0-3 18.3c.5.1.7-.2.7-.5v-1.7c-2.6.6-3.2-1.1-3.2-1.1-.4-1.1-1-1.4-1-1.4-.9-.6 0-.6 0-.6.9 0 1.4 1 1.4 1 .9 1.5 2.3 1.1 2.9.8.1-.6.3-1.1.6-1.3-2.1-.2-4.3-1-4.3-4.6 0-1 .4-1.9 1-2.5-.1-.3-.4-1.3.1-2.6 0 0 .8-.3 2.6 1a9 9 0 0 1 4.7 0c1.8-1.3 2.6-1 2.6-1 .5 1.3.2 2.3.1 2.6.6.6 1 1.5 1 2.5 0 3.6-2.2 4.4-4.3 4.6.3.3.6.9.6 1.8v2.7c0 .3.2.6.7.5A9.4 9.4 0 0 0 12 2.6z"/>
                                    </svg>
                                    <span>{CONTACT.projectUrl.replace(/^https?:\/\//, '')}</span>
                                </a>
                                <button type="button" className="about-copy" onClick={copyProject} title="复制链接">
                                    复制
                                </button>
                            </div>
                        ) : (
                            <p className="about-todo">
                                待填写 · 在本文件顶部的 CONTACT.projectUrl 里填入项目地址即可显示。
                            </p>
                        )}
                    </div>

                    <div className="about-divider"/>

                    <div className="about-facts">
                        {FACTS.map((fact) => (
                            <div key={fact.label}>
                                <small>{fact.label}</small>
                                <strong>{fact.value}</strong>
                            </div>
                        ))}
                    </div>

                    <div className="about-spacer"/>

                    <p className="about-note">
                        本地题库不会上传任何内容。只有在你主动点「识别 / 出题 / 归类」时，
                        题目内容会发给你自己配置的模型服务商；应用不收集使用日志。
                    </p>
                </Card>

                <Card
                    className="about-card about-scan"
                    title="联系方式"
                    subtitle="欢迎交流使用问题与功能建议"
                    tilt
                >
                    <span className="scan-ring" aria-hidden="true"/>
                    <span className="scan-sweep" aria-hidden="true"/>
                    <span className="scan-line" aria-hidden="true"/>

                    <div className="about-block">
                        <div className="about-contact-head">
                            <span className="about-contact-name">微信</span>
                            <span className="muted small">扫码添加</span>
                        </div>
                        <button
                            type="button"
                            className="about-qr"
                            onClick={() => setQrOpen(true)}
                            title="点击放大二维码"
                        >
                            <img src={wechatQr} alt="微信二维码：咚咚锵"/>
                        </button>
                        <p className="muted small about-contact-hint">
                            {CONTACT.wechatHint}点击图片可放大。
                        </p>
                    </div>

                    <div className="about-spacer"/>

                    <div className="about-meta">
                        <span>交流范围</span>
                        <span>使用问题 · 功能建议 · 题库格式</span>
                    </div>
                </Card>
            </div>

            {qrOpen && (
                <Modal
                    title="微信二维码"
                    subtitle="扫二维码，添加我为朋友"
                    small
                    onClose={() => setQrOpen(false)}
                >
                    <img className="about-qr-large" src={wechatQr} alt="微信二维码：咚咚锵"/>
                </Modal>
            )}
        </div>
    );
}
