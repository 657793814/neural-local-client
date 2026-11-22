import {invoke} from '@tauri-apps/api/core';

/**
 * 命令封装。参数名与 Rust 侧一一对应（Tauri 自动转 snake_case）。
 * 报错统一成字符串，页面直接展示——本地应用的错误原文就是给用户看的。
 */
async function call(name, args) {
    try {
        return await invoke(name, args);
    } catch (error) {
        const message = typeof error === 'string' ? error : error?.message || String(error);
        throw new Error(message);
    }
}

export const bootstrap = () => call('bootstrap');
export const authStatus = () => call('auth_status');
export const authSignup = (username, password, displayName) =>
    call('auth_signup', {input: {username, password, displayName: displayName || ''}});
export const authLogin = (username, password) =>
    call('auth_login', {input: {username, password}});
export const authLogout = () => call('auth_logout');
export const authChangePassword = (oldPassword, newPassword) =>
    call('auth_change_password', {input: {oldPassword, newPassword}});
export const authUpdateProfile = (displayName, bio) =>
    call('auth_update_profile', {input: {displayName, bio}});
export const stats = () => call('stats');
export const appInfo = () => call('app_info');
export const packPrompt = () => call('pack_prompt');
export const wizardFinish = () => call('wizard_finish');
export const settingGet = (key) => call('setting_get_value', {key});
export const settingSet = (key, value) => call('setting_set_value', {key, value});
export const dataDirSet = (path) => call('data_dir_set', {path});
export const revealDataDir = () => call('reveal_data_dir');
export const backupExport = () => call('backup_export');
export const writeTextFile = (path, content) => call('write_text_file', {path, content});

export const curriculum = (subjectId) => call('curriculum', {subjectId});
export const knowledgePointCreate = (subjectId, name) =>
    call('knowledge_point_create', {subjectId, name});

export const listQuestions = (query) => call('question_list', {query});
export const questionStats = (subjectIds = []) => call('question_stats', {subjectIds});
export const questionSources = () => call('question_sources');
export const getQuestion = (id) => call('question_get', {id});
export const saveQuestion = (input) => call('question_save', {input});
export const reviewQuestions = (ids, action, note) =>
    call('question_review', {input: {ids, action, note: note || null}});
export const deleteQuestions = (ids) => call('question_delete', {ids});
export const restoreQuestions = (ids) => call('question_restore', {ids});
export const purgeQuestion = (id) => call('question_purge', {id});
export const listTags = () => call('tag_list');
export const exportQuestions = (ids) => call('question_export', {ids});
export const exportQuestionsZip = (ids, path) => call('question_export_zip', {ids, path});
export const assetData = (id) => call('asset_data', {id});
export const assetAdd = (questionId, dataUrl, kind = 'FIGURE') =>
    call('asset_add', {questionId, dataUrl, kind});
export const assetRemove = (id) => call('asset_remove', {id});

export const captureRegion = () => call('capture_region');
export const imageFromPath = (path) => call('image_from_path', {path});
// 本地文件取题：图片直读，PDF 按页栅格化（page 从 1 开始）
export const fileOpen = (path, page = 1) => call('file_open', {path, page});
export const saveTempImage = (dataUrl) => call('save_temp_image', {dataUrl});
// AI 制图：按题干生成 SVG 示意图，返回 {svg, dataUrl, path}
export const diagramGenerate = (subjectId, stem, focus = '', mode = 'general') =>
    call('diagram_generate', {subjectId, stem, focus, mode});
// 内置浏览器：打开时带上当前学科，网页里抓到的题就归到这个学科。
export const browserOpen = (url, subjectId = '') =>
    call('browser_open', {url, subjectId: subjectId || null});
export const browserClose = () => call('browser_close');
// 内嵌网页视图跟着主窗里的占位框走；坐标是主窗内容区逻辑像素。
export const browserBounds = (x, y, width, height) =>
    call('browser_bounds', {x, y, width, height});
export const browserHide = () => call('browser_hide');
export const browserStatus = () => call('browser_status');
export const browserSetSubject = (subjectId) => call('browser_set_subject', {subjectId});
// 让页面里的脚本重新扫一遍题块，结果通过 nb://browser-questions 回到主窗。
export const browserExtractQuestions = () => call('browser_extract_questions');
// 本机 OCR（macOS Vision）是否可用：离线、不花模型额度。
export const ocrStatus = () => call('ocr_status');

// 听写台：OCR 切词、拼音建议、批次存取。播报走浏览器系统语音，不经过 IPC。
export const dictationOcrFromPath = (path) => call('dictation_ocr_from_path', {path});
export const dictationPinyin = (text) => call('dictation_pinyin', {text});
export const dictationSetSave = (draft) => call('dictation_set_save', {input: draft});
export const dictationSetList = () => call('dictation_set_list');
export const dictationSetGet = (id) => call('dictation_set_get', {id});
export const dictationSetDelete = (id) => call('dictation_set_delete', {id});
export const smartRegions = (path) => call('smart_regions', {path});
export const smartCrop = (path, region) => call('smart_crop', {path, region});
// 跨页题：相邻几页竖着拼成一张，交给识别链路
export const imageStitch = (paths) => call('image_stitch', {paths});

// 采集来源与连拍队列
export const captureSources = () => call('capture_sources');
export const captureSourceRemember = (url, label = '') => call('capture_source_remember', {url, label});
export const captureSourceStar = (id, starred) => call('capture_source_star', {id, starred});
export const captureSourceForget = (id) => call('capture_source_forget', {id});
export const captureQueueList = () => call('capture_queue_list');
export const captureQueueAdd = (imagePath, sourceUrl = '') => call('capture_queue_add', {imagePath, sourceUrl});
export const captureQueueRetry = (id) => call('capture_queue_retry', {id});
export const captureQueueRemove = (id) => call('capture_queue_remove', {id});
export const captureQueueClearDone = () => call('capture_queue_clear_done');

export const importScan = (path) => call('import_scan', {path});
export const importScanText = (text) => call('import_scan_text', {text});
export const importCommit = (previewId, rows, importDuplicates) =>
    call('import_commit', {previewId, rows, importDuplicates});
export const importDiscard = (previewId) => call('import_discard', {previewId});
export const importBatches = () => call('import_batches');
export const importErrors = (batchId) => call('import_errors', {batchId});
export const importUndo = (batchId) => call('import_undo', {batchId});

export const listModels = () => call('models_list');
export const saveModel = (input) => call('model_save', {input});
export const listRemoteModels = (baseUrl, apiKey, modelId) =>
    call('model_list_remote', {baseUrl, apiKey, modelId: modelId || null});
export const deleteModel = (id) => call('model_delete', {id});
export const testModel = (id) => call('model_test', {id});
export const getRoutes = () => call('routes_get');
export const setRoute = (purpose, modelIds) => call('route_set', {purpose, modelIds});

export const startJob = (kind, input) => call('job_start', {kind, input});
export const getJob = (id) => call('job_get', {id});
export const recentJobs = () => call('job_recent');
export const cancelJob = (id) => call('job_cancel', {id});
export const commitJob = (jobId, rows) => call('job_commit', {jobId, rows});
// 存草稿之前，先在预览里就地改一道题
export const updateJobItem = (jobId, index, patch) => call('job_item_update', {jobId, index, patch});

export const syncStatus = () => call('sync_status');
export const syncLogin = (server, username, password, deviceName) =>
    call('sync_login', {server, username, password, deviceName});
export const syncLogout = () => call('sync_logout');
export const syncPush = () => call('sync_push');
export const syncPull = () => call('sync_pull');
export const syncRetry = () => call('sync_retry');

// 中心作答照片（阶段 B）：绑定中心作业、把学生照片拉进批改台
export const centerClasses = () => call('center_classes');
export const centerAssignments = (classId) => call('center_assignments', {classId});
export const centerBinding = (assignmentId) => call('center_binding', {assignmentId});
export const centerBindAssignment = (assignmentId, centerAssignmentId, centerTitle) =>
    call('center_bind_assignment', {assignmentId, centerAssignmentId, centerTitle});
export const centerPullAssets = (assignmentId) => call('center_pull_assets', {assignmentId});
export const studentBindCenter = (studentId, centerUserId) =>
    call('student_bind_center', {studentId, centerUserId: centerUserId || null});

// 教学闭环：学生 / 班级 / 作业 / 作答
export const studentList = (keyword, status) =>
    call('student_list', {keyword: keyword || null, status: status || null});
export const studentSave = (input) => call('student_save', {input});
export const studentArchive = (id, archived) => call('student_archive', {id, archived});
export const studentDelete = (id) => call('student_delete', {id});
export const studentImportPreview = (text) => call('student_import_preview', {text});
export const studentImportCommit = (text) => call('student_import_commit', {text});

export const classList = () => call('class_list');
export const classGet = (id) => call('class_get', {id});
export const classSave = (input) => call('class_save', {input});
export const classClose = (id, closed) => call('class_close', {id, closed});
export const classDelete = (id) => call('class_delete', {id});
export const classRegenerateCode = (id) => call('class_regenerate_code', {id});
export const classMembers = (classId) => call('class_members', {classId});
export const classAddMembers = (classId, studentIds) => call('class_add_members', {classId, studentIds});
export const classRemoveMember = (classId, studentId) => call('class_remove_member', {classId, studentId});

export const assignmentList = (classId) => call('assignment_list', {classId: classId || null});
export const assignmentGet = (id) => call('assignment_get', {id});
export const assignmentSave = (input) => call('assignment_save', {input});
export const assignmentPublish = (id) => call('assignment_publish', {id});
export const assignmentClose = (id) => call('assignment_close', {id});
export const assignmentReopen = (id) => call('assignment_reopen', {id});
export const assignmentDelete = (id) => call('assignment_delete', {id});

export const attemptSubmit = (input) => call('attempt_submit', {input});
export const attemptSheet = (assignmentId, studentId) => call('attempt_sheet', {assignmentId, studentId});
export const attemptGrade = (input) => call('attempt_grade', {input});
export const attemptBatchGrade = (items) => call('attempt_batch_grade', {items});
export const attemptClear = (assignmentId, studentId) => call('attempt_clear', {assignmentId, studentId});
export const teachingOverview = () => call('teaching_overview');

// 组卷：按知识点 + 难度配额出草稿；换一题只换这道
export const composePool = (subjectId) => call('compose_pool', {subjectId: subjectId || null});
export const composeDraft = (input) => call('compose_draft', {input});
export const composeSwap = (input) => call('compose_swap', {input});
// 讲评清单：按题汇总错题、名单与批改备注
export const reviewSheet = (assignmentId) => call('review_sheet', {assignmentId});

// 本机存储：盘点与清理（tmp / 无主图片 / 旧备份）
export const storageUsage = () => call('storage_usage');
export const storageClean = (kind) => call('storage_clean', {kind});
export const dbExport = (path) => call('db_export', {path});

// 学情与错题本（P3）：掌握度按中心同一套衰减加权公式在本地算
export const insightsClass = (classId) => call('insights_class', {classId});
export const insightsStudent = (studentId) => call('insights_student', {studentId});
// 学生名单：classId 传空表示全部学生；keyword 支持姓名/年级模糊搜。
export const insightsRoster = (classId, keyword) =>
    call('insights_roster', {classId: classId || null, keyword: keyword || null});
export const wrongBookList = (studentId, stateFilter = '') =>
    call('wrong_book_list', {studentId, stateFilter: stateFilter || null});
export const wrongBookReview = (input) => call('wrong_book_review', {input});
export const wrongBookRemove = (id) => call('wrong_book_remove', {id});
export const wrongBookDue = (studentId) => call('wrong_book_due', {studentId: studentId || null});

// 学情诊断：有缓存读缓存，没缓存先用规则版；generate 才会调模型。
export const insightAdvice = (studentId) => call('insight_advice', {studentId});
export const insightAdviceGenerate = (studentId, force = false) =>
    call('insight_advice_generate', {studentId, force});

// 成绩：考试按整卷总分登分，与按题判分的作业分开。
export const scoreExams = (classId) => call('score_exams', {classId: classId || null});
export const scoreExamSheet = (examId) => call('score_exam_sheet', {examId});
export const scoreExamSave = (input) => call('score_exam_save', {input});
export const scoreExamDelete = (examId) => call('score_exam_delete', {examId});
export const scoreSave = (examId, rows) => call('score_save', {examId, rows});
export const scoreStudentExams = (studentId) => call('score_student_exams', {studentId});
// 作业 + 考试合并趋势：统一换成百分制后再混排。
export const studentProgress = (studentId, limit = 12) =>
    call('student_progress', {studentId, limit});

// 拍照批改：整页照片 / 切题 / 挂题 / 抄录建议
export const gradingPhotoAdd = (input) => call('grading_photo_add', {input});
export const gradingPhotoList = (studentId, assignmentId = null) =>
    call('grading_photo_list', {studentId, assignmentId});
export const gradingPhotoByAttempt = (attemptId) =>
    call('grading_photo_by_attempt', {attemptId});
export const gradingPhotoRegions = (assetId) => call('grading_photo_regions', {assetId});
export const gradingPhotoCrop = (input) => call('grading_photo_crop', {input});
export const gradingPhotoAttach = (assetId, attemptId) =>
    call('grading_photo_attach', {assetId, attemptId});
export const gradingPhotoRemove = (assetId) => call('grading_photo_remove', {assetId});
export const gradingPhotoData = (assetId) => call('grading_photo_data', {assetId});
export const gradingPaperSubmit = (assignmentId, studentId) =>
    call('grading_paper_submit', {assignmentId, studentId});
export const gradingTranscribe = (assetId, attemptId) =>
    call('grading_transcribe', {assetId, attemptId});

// 手机扫码上传作答照片：临时局域网入口
export const photoUploadStart = (studentId, assignmentId = null) =>
    call('photo_upload_start', {studentId, assignmentId});
export const photoUploadStop = () => call('photo_upload_stop');
export const photoUploadStatus = () => call('photo_upload_status');

// 教师课件（讲义）：库 / 导入 / 编译 / 预览 / 布置 / AI 补题
export const coursewareList = (keyword, subjectId, source) =>
    call('courseware_list', {
        keyword: keyword || null,
        subjectId: subjectId || null,
        source: source || null,
    });
export const coursewareImport = (input) => call('courseware_import', {input});
export const coursewareUpdate = (input) => call('courseware_update', {input});
export const coursewareDelete = (id) => call('courseware_delete', {id});
export const coursewarePreview = (id) => call('courseware_preview', {id});
export const coursewareBuild = (input, existingId = null) =>
    call('courseware_build', {input, existingId});
export const coursewareSpec = (id) => call('courseware_spec', {id});
export const coursewareAssign = (input) => call('courseware_assign', {input});
export const coursewareUnassign = (coursewareId, classId) =>
    call('courseware_unassign', {coursewareId, classId});
// 草稿：模型从例题补变式/拔高/思维训练，老师过目再编译
export const coursewareAiDraft = (input) => call('courseware_ai_draft', {input});
export const coursewarePrompt = (input) => call('courseware_prompt', {input});
export const coursewareImageAdd = (draftId, imageId, path) =>
    call('courseware_image_add', {draftId, imageId, path});
export const coursewareImageDir = (draftId) => call('courseware_image_dir', {draftId});
export const coursewareAiGenerate = (input, prompt, visionImageId = '') =>
    call('courseware_ai_generate', {input: {input, prompt, visionImageId}});
export const coursewareParseDraft = (raw, base) => call('courseware_parse_draft', {raw, base});
export const coursewareExport = (id, kind, path) => call('courseware_export', {id, kind, path});
export const coursewareImportStart = (input) => call('courseware_import_start', {input});
export const coursewareImportList = (batchId = null) => call('courseware_import_list', {batchId});
export const coursewareImportRetry = (id, useAi, allowDuplicate = false) =>
    call('courseware_import_retry', {id, useAi, allowDuplicate});
export const coursewareImportSave = (id, spec) => call('courseware_import_save', {id, spec});
export const coursewareImportCommit = (id, spec) => call('courseware_import_commit', {id, spec});
export const coursewareImportOriginal = (id) => call('courseware_import_original', {id});

// 试卷：组卷、分值、版面、打印记录（设计见 docs/product/local-paper-management-plan.md）
export const paperSchemes = () => call('paper_schemes');
export const paperList = (query) => call('paper_list', {query});
export const paperGet = (id) => call('paper_get', {id});
export const paperCreate = (input) => call('paper_create', {input});
export const paperUpdate = (id, patch) => call('paper_update', {id, patch});
export const paperSetTotal = (id, total, confirm = false) =>
    call('paper_set_total', {id, total, confirm});
export const paperApplyScheme = (id, schemeId) => call('paper_apply_scheme', {id, schemeId});
export const paperScoreCheck = (id) => call('paper_score_check', {id});
export const paperFinalize = (id) => call('paper_finalize', {id});
export const paperReopen = (id) => call('paper_reopen', {id});
export const paperArchive = (id) => call('paper_archive', {id});
export const paperDuplicate = (id) => call('paper_duplicate', {id});
export const paperDelete = (id) => call('paper_delete', {id});
export const paperRestore = (id) => call('paper_restore', {id});
export const paperSectionSave = (paperId, section) =>
    call('paper_section_save', {paperId, section});
export const paperSectionDelete = (paperId, sectionId) =>
    call('paper_section_delete', {paperId, sectionId});
export const paperSectionReorder = (paperId, ids) =>
    call('paper_section_reorder', {paperId, ids});
export const paperAddQuestions = (paperId, sectionId, questionIds) =>
    call('paper_add_questions', {paperId, sectionId, questionIds});
export const paperRemoveQuestion = (paperId, sectionId, ord) =>
    call('paper_remove_question', {paperId, sectionId, ord});
export const paperReorderQuestions = (paperId, sectionId, ords) =>
    call('paper_reorder_questions', {paperId, sectionId, ords});
export const paperSetScores = (paperId, items) => call('paper_set_scores', {paperId, items});
export const paperRefreshSnapshot = (paperId, items = null) =>
    call('paper_refresh_snapshot', {paperId, items});
export const paperPrintLog = (paperId, variant, copies, pages, channel) =>
    call('paper_print_log', {paperId, variant, copies, pages, channel});
// 蓝图组卷（P1）：先算分再抽题，缺题整次回滚并给缺口报告
export const paperBlueprintSuggest = (paperId) => call('paper_blueprint_suggest', {paperId});
export const paperBlueprintFill = (paperId, blueprint, replace = false) =>
    call('paper_blueprint_fill', {paperId, blueprint, replace});
export const paperBlueprintAgain = (paperId) => call('paper_blueprint_again', {paperId});
// 换题与撤销（P1）
export const paperReplaceCandidates = (paperId, query) =>
    call('paper_replace_candidates', {paperId, query});
export const paperReplaceQuestion = (paperId, sectionId, ord, questionId) =>
    call('paper_replace_question', {paperId, sectionId, ord, questionId});
export const paperUndoReplace = (paperId, sectionId, ord) =>
    call('paper_undo_replace', {paperId, sectionId, ord});
export const openPath = (path) => call('open_path', {path});
