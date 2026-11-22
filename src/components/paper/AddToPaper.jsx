import {useEffect, useState} from 'react';
import {paperAddQuestions, paperGet, paperList} from '../../lib/api.js';
import {fullSectionTitle} from '../../lib/paper/model.js';
import {Field, Loading, Modal, notify} from '../ui.jsx';

/**
 * 题库多选 → 加入试卷：只列草稿卷（定稿卷只读），选中卷后再选大题。
 * 题目的分值跟随所在大题的统一分值，进卷后可在编辑器里逐题调。
 */
export function AddToPaper({questionIds, onClose, onDone}) {
    const [papers, setPapers] = useState([]);
    const [loading, setLoading] = useState(true);
    const [paperId, setPaperId] = useState('');
    const [detail, setDetail] = useState(null);
    const [sectionId, setSectionId] = useState('');
    const [busy, setBusy] = useState(false);

    useEffect(() => {
        let alive = true;
        paperList({status: 'DRAFT', page: 1, size: 100})
            .then((page) => {
                if (!alive) return;
                setPapers(page.items || []);
                if (page.items?.length) setPaperId(page.items[0].id);
            })
            .catch((error) => alive && notify.error(error.message))
            .finally(() => alive && setLoading(false));
        return () => {
            alive = false;
        };
    }, []);

    useEffect(() => {
        if (!paperId) return undefined;
        let alive = true;
        paperGet(paperId)
            .then((data) => {
                if (!alive) return;
                setDetail(data);
                setSectionId(data.sections?.[0]?.id || '');
            })
            .catch((error) => alive && notify.error(error.message));
        return () => {
            alive = false;
        };
    }, [paperId]);

    const submit = async () => {
        if (!paperId || !sectionId) return;
        setBusy(true);
        try {
            await paperAddQuestions(paperId, sectionId, questionIds);
            notify.success(`已加入 ${questionIds.length} 道题`);
            onDone?.();
            onClose();
        } catch (error) {
            notify.error(error.message);
        } finally {
            setBusy(false);
        }
    };

    return (
        <Modal
            title="加入试卷"
            subtitle={`把选中的 ${questionIds.length} 道题加进一份草稿卷`}
            onClose={onClose}
            footer={
                <>
                    <button className="btn ghost" onClick={onClose}>
                        取消
                    </button>
                    <button className="btn primary" onClick={submit} disabled={busy || !sectionId}>
                        {busy ? '正在加入…' : '加入'}
                    </button>
                </>
            }
        >
            {loading ? (
                <Loading text="正在找草稿卷…"/>
            ) : papers.length === 0 ? (
                <p className="muted">还没有草稿卷。先去「试卷」新建一份，再回来加题。</p>
            ) : (
                <div className="form-grid">
                    <Field label="目标试卷" wide>
                        <select value={paperId} onChange={(event) => setPaperId(event.target.value)}>
                            {papers.map((paper) => (
                                <option key={paper.id} value={paper.id}>
                                    {paper.title}（{paper.questionCount} 题 / {paper.totalScore} 分）
                                </option>
                            ))}
                        </select>
                    </Field>
                    <Field label="加入哪道大题" wide hint="分值跟随该大题的统一分值">
                        <select
                            value={sectionId}
                            onChange={(event) => setSectionId(event.target.value)}
                            disabled={!detail}
                        >
                            {(detail?.sections || []).map((section, index) => (
                                <option key={section.id} value={section.id}>
                                    {fullSectionTitle(section, index)}（{section.questions.length} 题）
                                </option>
                            ))}
                            {(detail?.sections || []).length === 0 && (
                                <option value="">这份卷还没有大题</option>
                            )}
                        </select>
                    </Field>
                </div>
            )}
        </Modal>
    );
}
