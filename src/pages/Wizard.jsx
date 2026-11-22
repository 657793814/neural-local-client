import {useState} from 'react';
import {open} from '@tauri-apps/plugin-dialog';
import {dataDirSet, listModels, saveModel, wizardFinish} from '../lib/api.js';
import ModelPicker from '../components/ModelPicker.jsx';
import {Field, notify} from '../components/ui.jsx';

const STEPS = ['数据目录', '模型', '教材目录', '同步', '开始使用'];

export default function Wizard({boot, tree, onDone}) {
    const [step, setStep] = useState(0);
    const [form, setForm] = useState({name: '', baseUrl: '', model: '', apiKey: '', supportsVision: true});
    const [saved, setSaved] = useState(false);
    const [busy, setBusy] = useState(false);

    const subjects = tree.length;
    const textbooks = tree.reduce((sum, subject) => sum + (subject.textbooks?.length || 0), 0);
    const points = tree.reduce(
        (sum, subject) =>
            sum +
            (subject.textbooks || []).reduce(
                (inner, textbook) =>
                    inner +
                    (textbook.chapters || []).reduce((count, chapter) => count + (chapter.points?.length || 0), 0),
                0,
            ),
        0,
    );

    const finish = async (go) => {
        try {
            await wizardFinish();
        } catch (error) {
            notify.error(error.message);
        }
        await onDone?.();
        if (go) window.location.hash = `#/${go}`;
    };

    const addModel = async () => {
        setBusy(true);
        try {
            await saveModel({
                name: form.name || '我的模型',
                baseUrl: form.baseUrl,
                model: form.model,
                apiKey: form.apiKey || null,
                supportsVision: form.supportsVision,
                enabled: true,
                note: null,
            });
            setSaved(true);
            notify.success('模型已保存');
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="wizard-mask">
            <div className="wizard">
                <ol className="wizard-steps">
                    {STEPS.map((label, index) => (
                        <li key={label} className={index === step ? 'active' : index < step ? 'done' : ''}>
                            <span>{index + 1}</span>
                            {label}
                        </li>
                    ))}
                </ol>

                <div className="wizard-body">
                    {step === 0 && (
                        <>
                            <h2>题库放在哪里？</h2>
                            <p className="wizard-lead">所有题目、图片、配置都存在这个文件夹，换电脑整份拷走就能用。</p>
                            <div className="wizard-path">
                                <code>{boot.dataDir}</code>
                            </div>
                            <div className="inline-actions">
                                <button
                                    className="btn ghost"
                                    onClick={async () => {
                                        const dir = await open({directory: true, multiple: false});
                                        if (!dir) return;
                                        try {
                                            await dataDirSet(dir);
                                        } catch (error) {
                                            notify.error(error.message);
                                        }
                                    }}
                                >
                                    换个位置
                                </button>
                                <button className="btn ghost" onClick={() => setStep(1)}>
                                    就用这里
                                </button>
                            </div>
                            <p className="muted small">默认是「文稿 / NeuralKB-Local」。想放网盘同步目录也完全可以。</p>
                        </>
                    )}

                    {step === 1 && (
                        <>
                            <h2>接一个模型？</h2>
                            <p className="wizard-lead">
                                截图识别、AI 出题需要模型。只录题和导入的话可以先跳过，随时在设置里加。
                            </p>
                            {saved ? (
                                <p className="ok-text">已保存。可以继续，也可以再加一个。</p>
                            ) : (
                                <div className="form-grid">
                                    <Field label="名称" wide>
                                        <input
                                            value={form.name}
                                            placeholder="例如：主力视觉模型"
                                            onChange={(event) => setForm({...form, name: event.target.value})}
                                        />
                                    </Field>
                                    <Field label="Base URL" wide hint="一般以 /v1 结尾">
                                        <input
                                            value={form.baseUrl}
                                            placeholder="https://api.example.com/v1"
                                            onChange={(event) => setForm({...form, baseUrl: event.target.value})}
                                        />
                                    </Field>
                                    <Field label="API Key" wide hint="只进系统钥匙串，不会显示第二次">
                                        <input
                                            type="password"
                                            value={form.apiKey}
                                            onChange={(event) => setForm({...form, apiKey: event.target.value})}
                                        />
                                    </Field>
                                    <ModelPicker form={form} setForm={setForm}/>
                                    <label className="check wide">
                                        <input
                                            type="checkbox"
                                            checked={form.supportsVision}
                                            onChange={(event) => setForm({...form, supportsVision: event.target.checked})}
                                        />
                                        这个模型支持图片（截图识别要用）
                                    </label>
                                </div>
                            )}
                            <div className="row-end">
                                {!saved && (
                                    <button
                                        className="btn ghost"
                                        onClick={addModel}
                                        disabled={busy || !form.baseUrl || !form.model}
                                    >
                                        {busy ? '保存中…' : '保存这个模型'}
                                    </button>
                                )}
                                <button
                                    className="btn"
                                    onClick={async () => {
                                        try {
                                            const models = await listModels();
                                            setSaved(models.length > 0);
                                            setStep(2);
                                        } catch {
                                            setStep(2);
                                        }
                                    }}
                                >
                                    稍后配置，先继续
                                </button>
                            </div>
                        </>
                    )}

                    {step === 2 && (
                        <>
                            <h2>教材目录已内置</h2>
                            <p className="wizard-lead">题目按「学科 → 教材 → 章 → 知识点」归档，先给你一份通用目录。</p>
                            <div className="wizard-stats">
                                <div>
                                    <strong>{subjects}</strong>
                                    <span>学科</span>
                                </div>
                                <div>
                                    <strong>{textbooks}</strong>
                                    <span>教材</span>
                                </div>
                                <div>
                                    <strong>{points}</strong>
                                    <span>知识点</span>
                                </div>
                            </div>
                            <p className="muted small">之后在「设置」里可以查看版本；目录更新不影响你已经录入的题目。</p>
                        </>
                    )}

                    {step === 3 && (
                        <>
                            <h2>要连接中心题库吗？</h2>
                            <p className="wizard-lead">
                                可选。不连也能完整使用；连上之后可以把题目推给学校或机构自己的题库。
                            </p>
                            <ul className="plain-list">
                                <li>只有你主动点「推送」才会联网上传。</li>
                                <li>默认仅本人与管理员可见，共享要另外勾选。</li>
                                <li>断网、关机、重复点击都不会产生重复题。</li>
                            </ul>
                            <p className="muted small">需要的话，稍后在左下角菜单的「同步」里登录即可。</p>
                        </>
                    )}

                    {step === 4 && (
                        <>
                            <h2>可以开始了</h2>
                            <p className="wizard-lead">最快的路径：按 ⌘⇧O 截一张题，模型整理好后核对入库。</p>
                            <ul className="plain-list">
                                <li>题库是空的，可以先截一张图试试。</li>
                                <li>手上有一堆 Excel？用「导入 → 用 AI 转换」把提示词发给智能体。</li>
                                <li>每一步右上角的「?」都能打开对应的帮助。</li>
                            </ul>
                        </>
                    )}
                </div>

                <div className="wizard-foot">
                    <button className="btn ghost" onClick={() => finish('library')}>
                        跳过引导
                    </button>
                    <div className="inline-actions">
                        {step > 0 && (
                            <button className="btn ghost" onClick={() => setStep(step - 1)}>
                                上一步
                            </button>
                        )}
                        {step < STEPS.length - 1 ? (
                            <button className="btn primary" onClick={() => setStep(step + 1)}>
                                下一步
                            </button>
                        ) : (
                            <button className="btn primary" onClick={() => finish('capture')}>
                                去截第一道题
                            </button>
                        )}
                    </div>
                </div>
            </div>
        </div>
    );
}
