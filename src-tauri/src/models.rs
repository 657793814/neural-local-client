//! 前后端共享的数据形状。字段一律 camelCase，前端直接当 JS 对象用。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionItem {
    pub key: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub kind: String,
    pub content_type: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub id: String,
    pub subject_id: Option<String>,
    pub subject_name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub type_label: String,
    pub stem: String,
    pub excerpt: String,
    pub options: Vec<OptionItem>,
    pub answer: String,
    pub analysis: String,
    pub difficulty: i64,
    pub tags: Vec<String>,
    /// 知识点是题目归章 / 组卷的唯一依据。
    pub knowledge: Vec<KnowledgeRef>,
    /// 草稿还缺什么（缺答案 / 解析 / 知识点）。入库门槛靠它提示复核。
    #[serde(default)]
    pub issues: Vec<String>,
    pub assets: Vec<Asset>,
    pub thumb: Option<String>,
    pub source: Option<String>,
    pub year: Option<i64>,
    pub status: String,
    pub origin: String,
    pub needs_review: bool,
    pub ai_answer: bool,
    pub ai_analysis: bool,
    pub confidence: Option<f64>,
    pub review_note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub sync_state: String,
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub size: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuestionQuery {
    pub keyword: Option<String>,
    pub subject_id: Option<String>,
    pub kind: Option<String>,
    pub difficulty: Option<i64>,
    pub knowledge_id: Option<String>,
    pub tag: Option<String>,
    pub source: Option<String>,
    pub needs_review: Option<bool>,
    /// 只看被驳回的题（review_note 有值）。
    pub rejected: Option<bool>,
    pub trashed: Option<bool>,
    /// 批量归类用：只看完全没有挂知识点的题。
    pub uncategorized: Option<bool>,
    pub sort: Option<String>,
    pub page: Option<i64>,
    pub size: Option<i64>,
}

/// 题库里出现过的来源（采集网址）及题量，供题库按来源筛选。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCount {
    pub source: String,
    pub count: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionInput {
    pub id: Option<String>,
    pub subject_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub stem: String,
    #[serde(default)]
    pub options: Vec<OptionItem>,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub analysis: String,
    #[serde(default = "default_difficulty")]
    pub difficulty: i64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub knowledge_ids: Vec<String>,
    pub source: Option<String>,
    pub year: Option<i64>,
    pub status: Option<String>,
    pub origin: Option<String>,
    pub needs_review: Option<bool>,
    pub review_note: Option<String>,
}

fn default_difficulty() -> i64 {
    3
}

/// 复核动作：approve 通过并发布，reject 驳回退回草稿。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewInput {
    pub ids: Vec<String>,
    pub action: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub total: i64,
    pub draft: i64,
    pub needs_review: i64,
    pub rejected: i64,
    pub trashed: i64,
    pub with_images: i64,
    pub subjects: Vec<SubjectStat>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubjectStat {
    pub id: String,
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub data_dir: String,
    pub db_path: String,
    pub has_data: bool,
    pub wizard_done: bool,
    pub search_mode: String,
    pub stats: Stats,
    pub models: i64,
    pub app_version: String,
    pub platform: String,
    pub auth: AuthStatus,
}

/// 本地登录状态。has_user=false 表示还没设置账号（首次启动）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub has_user: bool,
    pub unlocked: bool,
    pub username: String,
    pub display_name: String,
    pub bio: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSignupInput {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthLoginInput {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthPasswordInput {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProfileInput {
    pub display_name: String,
    pub bio: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResult {
    pub path: String,
    pub data_url: String,
    pub width: i64,
    pub height: i64,
}

/// 本地文件（图片 / PDF 单页 / Word 文本）在采集台里的一份视图。
/// `image` 是当前页的栅格图，和截图返回的结构一致，前端可以走同一套裁剪 / 识别链路；
/// `text` 只对 Word / txt 这类纯文本文件有值——它们没有「页」，走的是文字取题。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalFile {
    pub path: String,
    pub name: String,
    /// `image` / `pdf` / `text`
    pub kind: String,
    pub page: i64,
    pub pages: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<CaptureResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpec {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub supports_vision: bool,
    pub enabled: bool,
    pub note: Option<String>,
    pub has_key: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInput {
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub note: Option<String>,
    /// 留空表示不修改已保存的 Key；新建时必须给。
    pub api_key: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub ok: bool,
    pub latency_ms: i64,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub progress: i64,
    pub error: Option<String>,
    pub input: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub configured: bool,
    pub server_url: String,
    pub username: String,
    pub enabled: bool,
    pub pending: i64,
    pub failed: i64,
    pub synced: i64,
    pub last_error: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInfo {
    pub id: String,
    pub filename: String,
    pub source_format: String,
    pub total: i64,
    pub imported: i64,
    pub skipped: i64,
    pub failed: i64,
    pub status: String,
    pub created_at: String,
    pub remaining: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportError {
    pub row_no: i64,
    pub code: String,
    pub message: String,
}

/// 采集来源记忆（常用站点 / 上次浏览的试题网址）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSource {
    pub id: String,
    pub url: String,
    pub label: String,
    pub starred: bool,
    pub capture_count: i64,
    pub last_at: String,
}

/// 连拍队列项：一次截图/粘贴/裁剪 = 一项，批量识别后再入库。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub id: String,
    pub image_path: String,
    pub source_id: Option<String>,
    pub source_url: String,
    pub status: String,
    pub job_id: Option<String>,
    pub error: String,
    pub item_count: i64,
    pub created_at: String,
}
