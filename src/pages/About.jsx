import {useState} from 'react';
import wechatQr from '../assets/wechat-qr.jpg';
import {Card, Modal, PageHead} from '../components/ui.jsx';

// 联系方式集中放在这里，改完下面的页面会自动更新，不必动结构。
const CONTACT = {
    name: '咚咚锵',
    title: 'NeuralKB 本地题库 · 开发者',
    bio: '做这款工具，是为了把散落的学习资料和题目整理成一台电脑上就能用的知识库——离线优先，数据只留在自己手里。',
    wechatHint: '扫码添加微信好友，交流使用问题与功能建议。',
    projectUrl: 'https://github.com/657793814/neural-local-client',
};

/** 关于我：开发者介绍与联系方式，从左下角账号菜单进入。 */
export default function About() {
    const [qrOpen, setQrOpen] = useState(false);

    return (
        <div className="page">
            <PageHead title="关于我" subtitle="开发者与联系方式"/>

            <div className="about-grid">
                <Card title="开发者" subtitle="这款工具背后的一个人">
                    <div className="about-person">
                        <span className="about-avatar">{CONTACT.name.slice(0, 1)}</span>
                        <span className="about-person-meta">
                            <strong>{CONTACT.name}</strong>
                            <small>{CONTACT.title}</small>
                        </span>
                    </div>
                    <p className="about-bio">{CONTACT.bio}</p>
                    <div className="about-divider"/>
                    <p className="muted small">
                        本地题库不会上传任何内容。只有在你主动点「识别 / 出题 / 归类」时，
                        题目内容会发给你自己配置的模型服务商；应用不收集使用日志。
                    </p>
                </Card>

                <Card title="联系方式" subtitle="欢迎交流使用问题与功能建议">
                    <div className="about-contact">
                        <div className="about-contact-head">
                            <span className="about-contact-name">微信</span>
                            <span className="muted small">扫二维码，添加我为朋友</span>
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

                    <div className="about-divider"/>

                    <div className="about-contact">
                        <div className="about-contact-head">
                            <span className="about-contact-name">项目地址</span>
                        </div>
                        {CONTACT.projectUrl ? (
                            <a className="about-link" href={CONTACT.projectUrl} target="_blank" rel="noreferrer">
                                {CONTACT.projectUrl}
                            </a>
                        ) : (
                            <p className="about-todo">
                                待填写 · 在本文件顶部的 CONTACT.projectUrl 里填入项目地址即可显示。
                            </p>
                        )}
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
