//! 拍照批改（P3 后半）。
//!
//! 场景：纸质作业收上来，老师用相机拍成照片，一题一块切出来，挂在题上给分。
//!
//! **不另起批改系统**：照片是挂在既有 `local_attempt`（一次提交 = 一个 submission_id）
//! 上的附件。批改、学情、错题本全都走原来那条链路，照片只是多了个「看得到原件」的东西。
//!
//! 三种照片：
//! - `PAGE`     整页照片，拍下来的原始卷子；
//! - `QUESTION` 从整页切出来的一块，对应一道题（`attempt_id` 挂上去）；
//! - 切出来还没挂题的照片先落库（`attempt_id` 为空），老师回头再挂。
//!
//! **模型只做抄录建议**（决策 D4）：AI 把手写的答案抄成文字并给个建议分，
//! 分数和评语必须老师确认才写进 `local_attempt`。原图永远保留，不被模型覆盖。

use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::smart::Region;

/// 照片相对路径的根目录（在 `assets/` 下）。与题目制图分开，清理时互不牵连。
pub const SUBMISSION_DIR: &str = "submissions";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoAsset {
    pub id: String,
    pub attempt_id: Option<String>,
    pub student_id: String,
    pub assignment_id: Option<String>,
    /// PAGE（整页）/ QUESTION（题级）。
    pub kind: String,
    pub relative_path: String,
    pub content_type: String,
    pub sha256: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// 题级照片来自哪张整页。
    pub page_asset_id: Option<String>,
    /// 这块在整页里的位置（原图像素），用于回看与重新切。
    pub bbox_json: Option<String>,
    pub ord: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoInput {
    pub student_id: String,
    #[serde(default)]
    pub assignment_id: Option<String>,
    /// data URL（前端粘贴 / 选图读出来的那种）。
    pub data_url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRequest {
    /// 整页照片 id。
    pub page_asset_id: String,
    pub region: Region,
    /// 切出来先挂这道题（可空）。
    #[serde(default)]
    pub attempt_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// AI 抄出来的学生答案。
    pub answer: String,
    /// 建议得分（0~满分），只是建议，老师可以改。
    pub score: Option<f64>,
    /// 建议评语。
    pub remark: String,
    /// AI 自己的把握：high / low。
    pub confidence: String,
    /// 抄录不了的原因（看不清、不是手写等）。
    pub warning: String,
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn map_photo(row: &rusqlite::Row<'_>) -> rusqlite::Result<PhotoAsset> {
    Ok(PhotoAsset {
        id: row.get(0)?,
        attempt_id: row.get(1)?,
        student_id: row.get(2)?,
        assignment_id: row.get(3)?,
        kind: row.get(4)?,
        relative_path: row.get(5)?,
        content_type: row.get(6)?,
        sha256: row.get(7)?,
        width: row.get(8)?,
        height: row.get(9)?,
        page_asset_id: row.get(10)?,
        bbox_json: row.get(11)?,
        ord: row.get(12)?,
        created_at: row.get(13)?,
    })
}

const PHOTO_COLUMNS: &str = "id, attempt_id, student_id, assignment_id, kind, relative_path, \
     content_type, sha256, width, height, page_asset_id, bbox_json, ord, created_at";

/// 存一张整页照片：落 `assets/submissions/<学生 id>/`，登记 `local_attempt_asset`。
pub fn store_page(
    conn: &Connection,
    data_dir: &std::path::Path,
    input: &PhotoInput,
) -> Result<PhotoAsset, String> {
    let bytes = decode_data_url(&input.data_url)?;
    let (mime, ext) =
        crate::text::sniff_image(&bytes).ok_or("只支持 jpg / png / webp / gif 图片")?;
    let id = new_id();
    let dir = data_dir
        .join("assets")
        .join(SUBMISSION_DIR)
        .join(safe_segment(&input.student_id));
    std::fs::create_dir_all(&dir).map_err(|e| format!("建作答照片目录失败：{e}"))?;
    let file_name = format!("{id}.{ext}");
    let target = dir.join(&file_name);
    std::fs::write(&target, &bytes).map_err(|e| format!("保存照片失败：{e}"))?;
    let relative = format!(
        "{SUBMISSION_DIR}/{}/{file_name}",
        safe_segment(&input.student_id)
    );
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let (width, height) = image::image_dimensions(&target)
        .map(|(w, h)| (Some(w as i64), Some(h as i64)))
        .unwrap_or((None, None));
    let now = crate::text::now();
    conn.execute(
        "insert into local_attempt_asset
            (id, attempt_id, student_id, assignment_id, kind, relative_path, content_type,
             sha256, width, height, page_asset_id, bbox_json, ord, created_at)
         values (?1, null, ?2, ?3, 'PAGE', ?4, ?5, ?6, ?7, ?8, null, null, 0, ?9)",
        params![
            &id,
            input.student_id,
            input.assignment_id,
            &relative,
            mime,
            &sha,
            width,
            height,
            &now
        ],
    )
    .map_err(|e| format!("登记照片失败：{e}"))?;
    get(conn, &id)?.ok_or_else(|| "照片登记后不见了".to_string())
}

/// 把中心拉下来的整页照片落成本地照片（阶段 B）。
///
/// 与 [`store_page`] 只有两处不同：字节已经在内存里（不再走 data URL），
/// 以及带中心附件 id 用于去重。落盘位置、登记字段、后续切题/给分与本地拍的照片完全一致——
/// 批改台不该区分「这张图是手机传的还是中心拉的」。
///
/// 返回 `Ok(None)` 表示这张中心照片之前已经拉过，跳过。
pub fn store_external_page(
    conn: &Connection,
    data_dir: &std::path::Path,
    student_id: &str,
    assignment_id: &str,
    center_asset_id: &str,
    bytes: &[u8],
) -> Result<Option<PhotoAsset>, String> {
    let pulled: Option<String> = conn
        .query_row(
            "select id from local_attempt_asset
              where center_asset_id = ?1 and deleted_at is null limit 1",
            [center_asset_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if pulled.is_some() {
        return Ok(None);
    }
    let (mime, ext) =
        crate::text::sniff_image(bytes).ok_or("中心照片不是支持的 jpg / png / webp / gif 图片")?;
    let id = new_id();
    let dir = data_dir
        .join("assets")
        .join(SUBMISSION_DIR)
        .join(safe_segment(student_id));
    std::fs::create_dir_all(&dir).map_err(|e| format!("建作答照片目录失败：{e}"))?;
    // 文件名带中心附件 id：重复拉取会覆盖同一路径，不会留下孤儿文件。
    let file_name = format!("center-{}.{ext}", safe_segment(center_asset_id));
    let target = dir.join(&file_name);
    std::fs::write(&target, bytes).map_err(|e| format!("保存照片失败：{e}"))?;
    let relative = format!("{SUBMISSION_DIR}/{}/{file_name}", safe_segment(student_id));
    let sha = format!("{:x}", Sha256::digest(bytes));
    let (width, height) = image::image_dimensions(&target)
        .map(|(w, h)| (Some(w as i64), Some(h as i64)))
        .unwrap_or((None, None));
    let now = crate::text::now();
    conn.execute(
        "insert into local_attempt_asset
            (id, attempt_id, student_id, assignment_id, kind, relative_path, content_type,
             sha256, width, height, page_asset_id, bbox_json, ord, created_at,
             center_asset_id, origin)
         values (?1, null, ?2, ?3, 'PAGE', ?4, ?5, ?6, ?7, ?8, null, null, 0, ?9, ?10, 'CENTER')",
        params![
            &id,
            student_id,
            assignment_id,
            &relative,
            mime,
            &sha,
            width,
            height,
            &now,
            center_asset_id
        ],
    )
    .map_err(|e| format!("登记照片失败：{e}"))?;
    Ok(Some(
        get(conn, &id)?.ok_or_else(|| "照片登记后不见了".to_string())?,
    ))
}

/// 按 id 取一张照片。
pub fn get(conn: &Connection, id: &str) -> Result<Option<PhotoAsset>, String> {
    conn.query_row(
        &format!(
            "select {PHOTO_COLUMNS} from local_attempt_asset where id = ?1 and deleted_at is null"
        ),
        [id],
        map_photo,
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// 某个学生在一份作业下的全部照片：整页 + 题级，按创建顺序。
pub fn list(
    conn: &Connection,
    student_id: &str,
    assignment_id: Option<&str>,
) -> Result<Vec<PhotoAsset>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "select {PHOTO_COLUMNS} from local_attempt_asset
              where student_id = ?1 and deleted_at is null
                and (?2 is null or assignment_id = ?2)
              order by kind asc, created_at asc, ord asc"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![student_id, assignment_id], map_photo)
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// 某次作答挂着的题级照片。
pub fn list_by_attempt(conn: &Connection, attempt_id: &str) -> Result<Vec<PhotoAsset>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "select {PHOTO_COLUMNS} from local_attempt_asset
              where attempt_id = ?1 and deleted_at is null
              order by ord asc, created_at asc"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([attempt_id], map_photo)
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

/// 从整页照片切一块出来，落成题级照片（可先不挂题）。
pub fn crop_from_page(
    conn: &Connection,
    data_dir: &std::path::Path,
    request: &CropRequest,
) -> Result<PhotoAsset, String> {
    let page = get(conn, &request.page_asset_id)?.ok_or("整页照片不在了")?;
    if page.kind != "PAGE" {
        return Err("只能从整页照片切块".into());
    }
    let source = data_dir.join("assets").join(&page.relative_path);
    let decoded = image::ImageReader::open(&source)
        .map_err(|e| format!("读整页照片失败：{e}"))?
        .with_guessed_format()
        .map_err(|e| format!("认不出整页照片格式：{e}"))?
        .decode()
        .map_err(|e| format!("解析整页照片失败：{e}"))?;
    let cropped = crate::smart::crop_region(&decoded, &request.region);
    let bytes = encode_png(&cropped)?;
    let (mime, ext) = ("image/png", "png");
    let id = new_id();
    let dir = data_dir
        .join("assets")
        .join(SUBMISSION_DIR)
        .join(safe_segment(&page.student_id));
    std::fs::create_dir_all(&dir).map_err(|e| format!("建作答照片目录失败：{e}"))?;
    let file_name = format!("{id}.{ext}");
    let target = dir.join(&file_name);
    std::fs::write(&target, &bytes).map_err(|e| format!("保存切块失败：{e}"))?;
    let relative = format!(
        "{SUBMISSION_DIR}/{}/{file_name}",
        safe_segment(&page.student_id)
    );
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let (width, height) = image::image_dimensions(&target)
        .map(|(w, h)| (Some(w as i64), Some(h as i64)))
        .unwrap_or((None, None));
    let bbox = serde_json::json!({
        "x": request.region.x,
        "y": request.region.y,
        "w": request.region.w,
        "h": request.region.h,
    })
    .to_string();
    let now = crate::text::now();
    conn.execute(
        "insert into local_attempt_asset
            (id, attempt_id, student_id, assignment_id, kind, relative_path, content_type,
             sha256, width, height, page_asset_id, bbox_json, ord, created_at)
         values (?1, ?2, ?3, ?4, 'QUESTION', ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, ?12)",
        params![
            &id,
            request.attempt_id,
            page.student_id,
            page.assignment_id,
            &relative,
            mime,
            &sha,
            width,
            height,
            &page.id,
            &bbox,
            &now
        ],
    )
    .map_err(|e| format!("登记切块失败：{e}"))?;
    get(conn, &id)?.ok_or_else(|| "切块登记后不见了".to_string())
}

/// 在一张照片上找候选题块（复用 `smart.rs` 的区域检测）。
///
/// 手写卷子的排版不规则，自动检测只能给个大概，最终还得老师自己框。
pub fn detect_regions(
    conn: &Connection,
    data_dir: &std::path::Path,
    asset_id: &str,
) -> Result<Vec<Region>, String> {
    let asset = get(conn, asset_id)?.ok_or("照片不在了")?;
    let file = data_dir.join("assets").join(&asset.relative_path);
    let decoded = image::ImageReader::open(&file)
        .map_err(|e| format!("读照片失败：{e}"))?
        .with_guessed_format()
        .map_err(|e| format!("认不出照片格式：{e}"))?
        .decode()
        .map_err(|e| format!("解析照片失败：{e}"))?;
    Ok(crate::smart::detect_regions(&decoded))
}

/// 把一张照片挂到某道题（切出来还没挂的题级图，或整页图当参考）。
pub fn attach(conn: &Connection, asset_id: &str, attempt_id: &str) -> Result<(), String> {
    let exists: i64 = conn
        .query_row(
            "select count(*) from local_attempt where id = ?1",
            [attempt_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        return Err("找不到这条作答".into());
    }
    let next_ord: i64 = conn
        .query_row(
            "select coalesce(max(ord), -1) + 1 from local_attempt_asset where attempt_id = ?1",
            [attempt_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    conn.execute(
        "update local_attempt_asset set attempt_id = ?2, ord = ?3 where id = ?1 and deleted_at is null",
        params![asset_id, attempt_id, next_ord],
    )
    .map_err(|e| format!("挂照片失败：{e}"))?;
    Ok(())
}

/// 软删一张照片（文件留着，老师还能在回收站外找回来——这里只做逻辑删除）。
pub fn remove(conn: &Connection, asset_id: &str) -> Result<(), String> {
    conn.execute(
        "update local_attempt_asset set deleted_at = ?2 where id = ?1 and deleted_at is null",
        params![asset_id, crate::text::now()],
    )
    .map_err(|e| format!("删照片失败：{e}"))?;
    Ok(())
}

/// 读一张照片，给前端渲染用（data URL，和题库图一个路子）。
pub fn read_data(
    conn: &Connection,
    data_dir: &std::path::Path,
    asset_id: &str,
) -> Result<String, String> {
    let asset = get(conn, asset_id)?.ok_or("照片不在了")?;
    let path = data_dir.join("assets").join(&asset.relative_path);
    let bytes = std::fs::read(&path).map_err(|e| format!("读照片失败：{e}"))?;
    let (mime, _) = crate::text::sniff_image(&bytes).ok_or("照片格式不认识")?;
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    ))
}

/// 纸质作业代提交：学生没在线交卷时，老师先建一次空提交，好让照片有地方挂、题能打分。
///
/// 全部题目 `correct = null`（待批），不会自动判对错，也不动错题本。
pub fn paper_submit(
    conn: &mut Connection,
    assignment_id: &str,
    student_id: &str,
) -> Result<String, String> {
    let assignment = crate::teaching::assignment_get(conn, assignment_id)?;
    if assignment.status == "DRAFT" {
        return Err("这份作业还没发布".into());
    }
    let member: i64 = conn
        .query_row(
            "select count(*) from local_class_member where class_id = ?1 and student_id = ?2",
            params![assignment.class_id, student_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if member == 0 {
        return Err("这个学生不在班里".into());
    }
    // 已有未作废的提交就不重复建，直接用那一次。
    let existing: Option<String> = conn
        .query_row(
            "select submission_id from local_attempt
              where assignment_id = ?1 and student_id = ?2 and superseded_at is null
              group by submission_id order by max(submitted_at) desc limit 1",
            params![assignment_id, student_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let questions = crate::teaching::assignment_questions(conn, assignment_id)?;
    if questions.is_empty() {
        return Err("这份作业没有题目".into());
    }
    let submission_id = new_id();
    let now = crate::text::now();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for row in &questions {
        tx.execute(
            "insert into local_attempt
                (id, assignment_id, question_id, student_id, submission_id, answer, duration_ms,
                 submitted_at, correct, score, max_score, late)
             values (?1, ?2, ?3, ?4, ?5, '', null, ?6, null, null, ?7, 0)",
            params![
                new_id(),
                assignment_id,
                row.question.id,
                student_id,
                &submission_id,
                &now,
                row.score
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(submission_id)
}

/// 抄录建议的内部实现（命令层调用，能拿到数据目录）。
pub fn transcribe_with(
    conn: &Connection,
    http: &reqwest::blocking::Client,
    data_dir: &std::path::Path,
    asset_id: &str,
    attempt_id: &str,
) -> Result<Suggestion, String> {
    let asset = get(conn, asset_id)?.ok_or("照片不在了")?;
    let (question_stem, answer, max_score): (String, String, f64) = conn
        .query_row(
            "select q.stem, q.answer, a.max_score
               from local_attempt a join question q on q.id = a.question_id
              where a.id = ?1",
            [attempt_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "找不到这条作答".to_string())?;
    let file = data_dir.join("assets").join(&asset.relative_path);
    let bytes = std::fs::read(&file).map_err(|e| format!("读照片失败：{e}"))?;
    let (mime, _) = crate::text::sniff_image(&bytes).ok_or("照片格式不认识")?;
    let image = crate::llm::ImageInput {
        mime: mime.to_string(),
        bytes,
    };
    let messages = crate::llm::prompts::grading_transcribe(&question_stem, &answer, max_score);
    let mut cooldowns = std::collections::HashMap::new();
    let raw = crate::llm::complete(
        conn,
        http,
        "grading",
        &messages,
        Some(&image),
        true,
        2048,
        &mut cooldowns,
    )?;
    let parsed = crate::llm::parse_json(&raw, "抄录模型")?;
    Ok(Suggestion {
        answer: parsed
            .get("answer")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string(),
        score: parsed.get("score").and_then(|v| v.as_f64()),
        remark: parsed
            .get("remark")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string(),
        confidence: parsed
            .get("confidence")
            .and_then(|v| v.as_str())
            .unwrap_or("low")
            .trim()
            .to_string(),
        warning: parsed
            .get("warning")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string(),
    })
}

/// data URL → 原始字节。
fn decode_data_url(data_url: &str) -> Result<Vec<u8>, String> {
    let (_, payload) = data_url
        .split_once(',')
        .ok_or_else(|| "图片数据不完整".to_string())?;
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| format!("图片解码失败：{e}"))
}

fn encode_png(image: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("编码切块失败：{e}"))?;
    Ok(out.into_inner())
}

/// 学生 id 拼进目录名，得先消毒，别让路径跑出去。
fn safe_segment(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn fresh() -> (Connection, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "nb-grading-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut conn = db::open(&dir.join("neuralkb.db")).unwrap();
        db::migrate(&mut conn).unwrap();
        db::seed_curriculum(&mut conn).unwrap();
        (conn, dir)
    }

    /// 一张 2×2 的白底黑格测试图（够大，能被区域检测认出块）。
    fn sample_png() -> Vec<u8> {
        let mut img = image::RgbImage::new(240, 160);
        for y in 20..60 {
            for x in 20..220 {
                img.put_pixel(x, y, image::Rgb([10, 10, 10]));
            }
        }
        for y in 100..140 {
            for x in 20..220 {
                img.put_pixel(x, y, image::Rgb([10, 10, 10]));
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    fn data_url(bytes: &[u8]) -> String {
        format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }

    fn setup(conn: &mut Connection) -> (String, String, String, String) {
        // 学生
        let student = crate::teaching::student_save(
            conn,
            &crate::teaching::StudentInput {
                id: None,
                name: "王小明".into(),
                student_no: String::new(),
                grade: "高一".into(),
                subject_ids: vec![],
                note: String::new(),
                status: None,
            },
        )
        .unwrap();
        // 学科
        let subject: String = conn
            .query_row("select id from subject order by sort limit 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        // 题目
        let question = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "insert into question (id, subject_id, type, difficulty, stem, answer, analysis, status, created_at, updated_at)
             values (?1, ?2, 'SOLVED', 3, '求阴影面积', '12', '面积相减', 'PUBLISHED', '2026-01-01T00:00:00+08:00', '2026-01-01T00:00:00+08:00')",
            params![&question, &subject],
        )
        .unwrap();
        // 班级
        let class = crate::teaching::class_save(
            conn,
            &crate::teaching::ClassInput {
                id: None,
                name: "高一(1)班".into(),
                subject_id: Some(subject.clone()),
                term: String::new(),
            },
        )
        .unwrap();
        crate::teaching::class_add_members(conn, &class, &[student.clone()]).unwrap();
        // 作业
        let assignment = crate::teaching::assignment_save(
            conn,
            &crate::teaching::AssignmentInput {
                id: None,
                class_id: class.clone(),
                title: "第一章作业".into(),
                kind: "PRACTICE".into(),
                description: String::new(),
                due_at: None,
                late_hours: 0,
                questions: vec![crate::teaching::AssignmentQuestionInput {
                    question_id: question.clone(),
                    score: 10.0,
                }],
            },
        )
        .unwrap();
        crate::teaching::assignment_publish(conn, &assignment).unwrap();
        (student, class, assignment, question)
    }

    #[test]
    fn photo_stored_and_listed_for_student() {
        let (mut conn, dir) = fresh();
        let (student, _class, assignment, _question) = setup(&mut conn);
        let page = store_page(
            &conn,
            &dir,
            &PhotoInput {
                student_id: student.clone(),
                assignment_id: Some(assignment.clone()),
                data_url: data_url(&sample_png()),
            },
        )
        .unwrap();
        assert_eq!(page.kind, "PAGE");
        assert!(page.relative_path.starts_with("submissions/"));
        assert!(dir.join("assets").join(&page.relative_path).exists());
        assert!(page.width.unwrap_or(0) > 0, "要读出图片尺寸");

        let list = list(&conn, &student, Some(&assignment)).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, page.id);

        // 读图给前端：应是 data URL。
        let data = read_data(&conn, &dir, &page.id).unwrap();
        assert!(data.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn cropped_block_keeps_source_page_and_bbox() {
        let (mut conn, dir) = fresh();
        let (student, _class, assignment, _question) = setup(&mut conn);
        let page = store_page(
            &conn,
            &dir,
            &PhotoInput {
                student_id: student.clone(),
                assignment_id: Some(assignment.clone()),
                data_url: data_url(&sample_png()),
            },
        )
        .unwrap();
        let region = Region {
            x: 10,
            y: 10,
            w: 120,
            h: 60,
        };
        let block = crop_from_page(
            &conn,
            &dir,
            &CropRequest {
                page_asset_id: page.id.clone(),
                region: region.clone(),
                attempt_id: None,
            },
        )
        .unwrap();
        assert_eq!(block.kind, "QUESTION");
        assert_eq!(block.page_asset_id.as_deref(), Some(page.id.as_str()));
        assert_eq!(block.attempt_id, None, "切出来先不挂题");
        let bbox: serde_json::Value =
            serde_json::from_str(block.bbox_json.as_deref().unwrap()).unwrap();
        assert_eq!(bbox["x"], 10);
        assert_eq!(bbox["w"], 120);
        assert!(dir.join("assets").join(&block.relative_path).exists());
    }

    #[test]
    fn paper_submit_creates_pending_attempts_and_photos_attach() {
        let (mut conn, dir) = fresh();
        let (student, _class, assignment, _question) = setup(&mut conn);

        // 学生没在线交卷：先代建一次空提交。
        let submission = paper_submit(&mut conn, &assignment, &student).unwrap();
        assert!(!submission.is_empty());

        let sheet = crate::teaching::attempt_sheet(&conn, &assignment, &student).unwrap();
        assert_eq!(sheet.items.len(), 1, "空提交也要有一行，好让老师打分");
        let attempt = &sheet.items[0];
        assert_eq!(attempt.correct, None, "未判分");
        assert_eq!(attempt.score, None);
        assert_eq!(attempt.answer, "");

        // 拍整页 → 切一块 → 挂到这道题。
        let page = store_page(
            &conn,
            &dir,
            &PhotoInput {
                student_id: student.clone(),
                assignment_id: Some(assignment.clone()),
                data_url: data_url(&sample_png()),
            },
        )
        .unwrap();
        let block = crop_from_page(
            &conn,
            &dir,
            &CropRequest {
                page_asset_id: page.id.clone(),
                region: Region {
                    x: 0,
                    y: 0,
                    w: 200,
                    h: 150,
                },
                attempt_id: None,
            },
        )
        .unwrap();
        attach(&conn, &block.id, &attempt.attempt_id).unwrap();

        let sheet = crate::teaching::attempt_sheet(&conn, &assignment, &student).unwrap();
        assert_eq!(sheet.items[0].photos.len(), 1, "题上要能看到照片");
        assert_eq!(sheet.items[0].photos[0].id, block.id);

        // 打分后错题本跟着走（做错入本）。
        crate::teaching::grade(
            &mut conn,
            &crate::teaching::GradeInput {
                attempt_id: attempt.attempt_id.clone(),
                score: 2.0,
                correct: Some(false),
                remark: "步骤漏了一步".into(),
            },
            "老师",
        )
        .unwrap();
        let sheet = crate::teaching::attempt_sheet(&conn, &assignment, &student).unwrap();
        assert_eq!(sheet.items[0].score, Some(2.0));
        assert_eq!(sheet.items[0].correct, Some(false));
        let wrong: i64 = conn
            .query_row(
                "select count(*) from local_wrong_book where student_id = ?1 and deleted_at is null",
                [&student],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(wrong, 1, "打错分要进错题本");
    }

    #[test]
    fn removed_photo_disappears_from_list() {
        let (mut conn, dir) = fresh();
        let (student, _class, assignment, _question) = setup(&mut conn);
        let page = store_page(
            &conn,
            &dir,
            &PhotoInput {
                student_id: student.clone(),
                assignment_id: Some(assignment.clone()),
                data_url: data_url(&sample_png()),
            },
        )
        .unwrap();
        remove(&conn, &page.id).unwrap();
        assert!(list(&conn, &student, Some(&assignment)).unwrap().is_empty());
        assert!(read_data(&conn, &dir, &page.id).is_err(), "删了就读不到");
    }

    #[test]
    fn paper_submit_is_idempotent() {
        let (mut conn, _dir) = fresh();
        let (student, _class, assignment, _question) = setup(&mut conn);
        let first = paper_submit(&mut conn, &assignment, &student).unwrap();
        let second = paper_submit(&mut conn, &assignment, &student).unwrap();
        assert_eq!(first, second, "重复调用不新建提交");
        let count: i64 = conn
            .query_row(
                "select count(*) from local_attempt where assignment_id = ?1 and student_id = ?2 and superseded_at is null",
                params![&assignment, &student],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn outsider_student_cannot_paper_submit() {
        let (mut conn, _dir) = fresh();
        let (_student, _class, assignment, _question) = setup(&mut conn);
        let stranger = crate::teaching::student_save(
            &conn,
            &crate::teaching::StudentInput {
                id: None,
                name: "外来人".into(),
                student_no: String::new(),
                grade: String::new(),
                subject_ids: vec![],
                note: String::new(),
                status: None,
            },
        )
        .unwrap();
        assert!(paper_submit(&mut conn, &assignment, &stranger).is_err());
    }
}
