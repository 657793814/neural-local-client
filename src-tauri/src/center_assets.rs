//! 中心作答照片（阶段 B）：把「中心作业」对到本地作业，拉学生照片进批改台。
//!
//! 场景：学生在家里用手机/电脑把纸质作业拍下来，提交到中心；教师第二天在本地批改台
//! 点一下「拉取照片」，照片就落到对应学生名下，接着切题、给分、学情全部走现有链路。
//!
//! **不建平行系统**：拉下来的照片和手机扫码传的照片是同一种东西——`local_attempt_asset`
//! 里 `kind='PAGE'` 的整页图，只是多了 `center_asset_id` 用于去重、`origin='CENTER'` 便于追溯。
//!
//! 两个作业的对应关系由教师手动绑定：本地作业和中心作业是各自独立创建的，
//! 靠标题猜容易错，宁可让人点一次。
//!
//! 学生匹配按「先绑定的中心账号，再班级内同名」两步：中心的 `app_user.id` 是 UUID，
//! 本地学生是独立主体，只有 `center_user_id` 写了才是一一对应；没写的就看名字，
//! 同名学生不猜，直接列出来让老师去绑。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;

use crate::sync;
use crate::teaching::Student;

// ---------------------------------------------------------------------------
// 中心返回的数据形状（与 DesktopEduController 的 JSON 对齐）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CenterClass {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub subject_name: Option<String>,
    #[serde(default)]
    pub term: Option<String>,
    #[serde(default)]
    pub student_count: i64,
    #[serde(default)]
    pub assignment_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CenterAssignment {
    pub id: String,
    pub title: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub due_at: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub question_count: i64,
    #[serde(default)]
    pub submitted_count: i64,
    #[serde(default)]
    pub student_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CenterAsset {
    pub id: String,
    pub assignment_id: String,
    pub student_id: String,
    #[serde(default)]
    pub student_username: String,
    #[serde(default)]
    pub student_name: String,
    #[serde(default)]
    pub ord: i64,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub byte_size: i64,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// 中心账号在本地对不上学生时返回的信息：老师据此挑一个本地学生绑上去。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmatchedStudent {
    pub center_user_id: String,
    pub name: String,
    pub username: String,
    /// 这个账号这次有多少张照片没落地（含同名冲突的）。
    pub photos: usize,
}

/// 拉取结果：拉了几张、跳过几张（之前拉过）、哪些照片的主人没对上本地学生。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullResult {
    pub pulled: usize,
    pub skipped: usize,
    pub unmatched: Vec<UnmatchedStudent>,
}

/// 本地作业绑定的中心作业（没有绑定时为 None）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub center_assignment_id: String,
    pub center_assignment_title: String,
}

// ---------------------------------------------------------------------------
// 中心查询
// ---------------------------------------------------------------------------

pub fn classes(
    conn: &Connection,
    client: &reqwest::blocking::Client,
) -> Result<Vec<CenterClass>, String> {
    get_json(conn, client, "/api/desktop/edu/classes")
}

pub fn assignments(
    conn: &Connection,
    client: &reqwest::blocking::Client,
    class_id: &str,
) -> Result<Vec<CenterAssignment>, String> {
    get_json(
        conn,
        client,
        &format!("/api/desktop/edu/classes/{class_id}/assignments"),
    )
}

pub fn assignment_assets(
    conn: &Connection,
    client: &reqwest::blocking::Client,
    center_assignment_id: &str,
) -> Result<Vec<CenterAsset>, String> {
    get_json(
        conn,
        client,
        &format!("/api/desktop/edu/assignments/{center_assignment_id}/assets"),
    )
}

fn get_json<T: for<'de> Deserialize<'de>>(
    conn: &Connection,
    client: &reqwest::blocking::Client,
    path: &str,
) -> Result<T, String> {
    let (base, token) = sync::connection(conn)?;
    let response = client
        .get(format!("{base}{path}"))
        .bearer_auth(&token)
        .timeout(Duration::from_secs(30))
        .send()
        .map_err(|e| format!("连不上服务器：{e}"))?;
    let code = response.status();
    let body = response.text().map_err(|e| e.to_string())?;
    if !code.is_success() {
        return Err(sync::error_of(code, &body));
    }
    serde_json::from_str(&body).map_err(|e| format!("服务器响应不合法：{e}"))
}

// ---------------------------------------------------------------------------
// 绑定与拉取
// ---------------------------------------------------------------------------

/// 记下「本地作业 ↔ 中心作业」的对应关系。只存标题用于界面显示，不复制题目。
pub fn bind(
    conn: &Connection,
    local_assignment_id: &str,
    center_assignment_id: &str,
    center_title: &str,
) -> Result<(), String> {
    let changed = conn
        .execute(
            "update local_assignment
                set center_assignment_id = ?2, center_assignment_title = ?3, updated_at = ?4
              where id = ?1 and deleted_at is null",
            params![
                local_assignment_id,
                center_assignment_id,
                center_title,
                crate::text::now()
            ],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("作业不存在或已删除".into());
    }
    Ok(())
}

pub fn binding(conn: &Connection, local_assignment_id: &str) -> Result<Option<Binding>, String> {
    conn.query_row(
        "select center_assignment_id, ifnull(center_assignment_title, '')
           from local_assignment
          where id = ?1 and deleted_at is null and center_assignment_id is not null",
        [local_assignment_id],
        |row| {
            Ok(Binding {
                center_assignment_id: row.get(0)?,
                center_assignment_title: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())
}

/// 拉取一份本地作业对应中心作业的全部学生照片。
pub fn pull(
    conn: &Connection,
    data_dir: &Path,
    client: &reqwest::blocking::Client,
    local_assignment_id: &str,
) -> Result<PullResult, String> {
    let (center_assignment_id, class_id): (Option<String>, String) = conn
        .query_row(
            "select center_assignment_id, class_id from local_assignment
              where id = ?1 and deleted_at is null",
            [local_assignment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("作业不存在或已删除")?;
    let center_assignment_id = center_assignment_id
        .ok_or("这份作业还没绑定中心作业：先点「中心同步」选好对应的中心作业")?;

    let assets = assignment_assets(conn, client, &center_assignment_id)?;
    let mut members = crate::teaching::class_members(conn, &class_id)?;
    let mut result = PullResult {
        pulled: 0,
        skipped: 0,
        unmatched: Vec::new(),
    };
    if assets.is_empty() {
        return Ok(result);
    }
    let (base, token) = sync::connection(conn)?;

    for asset in assets {
        let index = match match_student(&members, &asset) {
            MatchResult::Matched(index) => index,
            MatchResult::Missing | MatchResult::Ambiguous => {
                remember_unmatched(&mut result, &asset);
                continue;
            }
        };
        let student_id = members[index].id.clone();
        if members[index].center_user_id.is_none() {
            // 名字对上了就顺手把中心账号记下来：下次改名也能按 id 认人。
            conn.execute(
                "update local_student set center_user_id = ?2, updated_at = ?3
                  where id = ?1 and center_user_id is null",
                params![student_id, asset.student_id, crate::text::now()],
            )
            .map_err(|e| e.to_string())?;
            members[index].center_user_id = Some(asset.student_id.clone());
        }

        let bytes = get_bytes(
            client,
            &base,
            &token,
            &format!("/api/desktop/edu/assets/{}/data", asset.id),
        )?;
        match crate::grading::store_external_page(
            conn,
            data_dir,
            &student_id,
            local_assignment_id,
            &asset.id,
            &bytes,
        ) {
            Ok(Some(_)) => result.pulled += 1,
            Ok(None) => result.skipped += 1,
            Err(error) => return Err(format!("照片存不进本地：{error}")),
        }
    }
    Ok(result)
}

fn get_bytes(
    client: &reqwest::blocking::Client,
    base: &str,
    token: &str,
    path: &str,
) -> Result<Vec<u8>, String> {
    let response = client
        .get(format!("{base}{path}"))
        .bearer_auth(token)
        .timeout(Duration::from_secs(60))
        .send()
        .map_err(|e| format!("照片下载失败：{e}"))?;
    let code = response.status();
    if !code.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(sync::error_of(code, &body));
    }
    response
        .bytes()
        .map(|bytes| bytes.to_vec())
        .map_err(|e| format!("照片下载失败：{e}"))
}

// ---------------------------------------------------------------------------
// 学生匹配
// ---------------------------------------------------------------------------

enum MatchResult {
    Matched(usize),
    Missing,
    Ambiguous,
}

/// 中心的一张照片属于本地哪个学生。
///
/// 1. `center_user_id` 已经绑过的，按 id 认——这是唯一可靠的对应；
/// 2. 没绑过的，看班级里「未绑定 + 同名」的学生是否**恰好一个**；两个同名的不猜。
fn match_student(members: &[Student], asset: &CenterAsset) -> MatchResult {
    if let Some(index) = members
        .iter()
        .position(|member| member.center_user_id.as_deref() == Some(asset.student_id.as_str()))
    {
        return MatchResult::Matched(index);
    }
    let name = asset.student_name.trim();
    if name.is_empty() {
        return MatchResult::Missing;
    }
    let hits: Vec<usize> = members
        .iter()
        .enumerate()
        .filter(|(_, member)| member.center_user_id.is_none() && member.name.trim() == name)
        .map(|(index, _)| index)
        .collect();
    match hits.len() {
        1 => MatchResult::Matched(hits[0]),
        0 => MatchResult::Missing,
        _ => MatchResult::Ambiguous,
    }
}

/// 同一个中心账号的多张照片只报一次，免得界面刷一屏重复的名字。
fn remember_unmatched(result: &mut PullResult, asset: &CenterAsset) {
    if let Some(found) = result
        .unmatched
        .iter_mut()
        .find(|item| item.center_user_id == asset.student_id)
    {
        found.photos += 1;
        return;
    }
    result.unmatched.push(UnmatchedStudent {
        center_user_id: asset.student_id.clone(),
        name: asset.student_name.trim().to_string(),
        username: asset.student_username.trim().to_string(),
        photos: 1,
    });
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn student(id: &str, name: &str, center: Option<&str>) -> Student {
        Student {
            id: id.into(),
            student_no: String::new(),
            name: name.into(),
            grade: String::new(),
            subject_ids: Vec::new(),
            subject_names: Vec::new(),
            note: String::new(),
            center_user_id: center.map(|value| value.to_string()),
            status: "ACTIVE".into(),
            class_count: 1,
            attempt_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn asset(student_id: &str, name: &str, username: &str) -> CenterAsset {
        CenterAsset {
            id: "asset-1".into(),
            assignment_id: "assign-1".into(),
            student_id: student_id.into(),
            student_username: username.into(),
            student_name: name.into(),
            ord: 0,
            content_type: "image/jpeg".into(),
            byte_size: 1,
            created_at: None,
        }
    }

    #[test]
    fn matches_by_bound_center_account_first() {
        let members = vec![
            student("s1", "张三", Some("u-1")),
            student("s2", "李四", None),
        ];
        match match_student(&members, &asset("u-1", "李四", "lisi")) {
            MatchResult::Matched(index) => assert_eq!(members[index].id, "s1"),
            _ => panic!("绑过的中心账号应该优先"),
        }
    }

    #[test]
    fn matches_by_unique_name_in_class() {
        let members = vec![
            student("s1", "张三", None),
            student("s2", "李四", Some("u-2")),
        ];
        match match_student(&members, &asset("u-9", "李四", "lisi")) {
            // 李四虽然同名，但已绑到别的中心账号，不能顶替
            MatchResult::Missing => {}
            _ => panic!("已绑定其他中心账号的学生不应被同名抢占"),
        }
        match match_student(&members, &asset("u-9", "张三", "zhangsan")) {
            MatchResult::Matched(index) => assert_eq!(members[index].id, "s1"),
            _ => panic!("班级内唯一同名应该自动匹配"),
        }
    }

    #[test]
    fn refuses_to_guess_when_names_collide() {
        let members = vec![student("s1", "张三", None), student("s2", "张三", None)];
        assert!(matches!(
            match_student(&members, &asset("u-9", "张三", "zhangsan")),
            MatchResult::Ambiguous
        ));
    }

    #[test]
    fn missing_when_no_name_and_no_binding() {
        let members = vec![student("s1", "张三", None)];
        assert!(matches!(
            match_student(&members, &asset("u-9", "", "")),
            MatchResult::Missing
        ));
    }

    /// 同一张中心照片拉两次只落一份；落盘文件与 origin 都要对。
    #[test]
    fn pulled_photo_lands_once() {
        let dir = std::env::temp_dir().join(format!(
            "nb-center-asset-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("neuralkb.db");
        let mut conn = crate::db::open(&db_path).unwrap();
        crate::db::migrate(&mut conn).unwrap();
        let now = crate::text::now();
        conn.execute(
            "insert into local_student (id, name, created_at, updated_at) values ('s1', '张三', ?1, ?1)",
            [&now],
        )
        .unwrap();
        conn.execute(
            "insert into local_class (id, name, join_code, created_at, updated_at)
             values ('c1', '高一(1)班', 'ABC234', ?1, ?1)",
            [&now],
        )
        .unwrap();
        conn.execute(
            "insert into local_assignment (id, class_id, title, created_at, updated_at)
             values ('a1', 'c1', '第一次作业', ?1, ?1)",
            [&now],
        )
        .unwrap();

        let bytes: &[u8] = b"\x89PNG\r\n\x1a\nfake-body-for-dedupe-check";
        let first =
            crate::grading::store_external_page(&conn, &dir, "s1", "a1", "center-asset-1", bytes)
                .unwrap()
                .expect("第一次应该落库");
        assert_eq!(first.kind, "PAGE");
        assert!(first
            .relative_path
            .starts_with("submissions/s1/center-center-asset-1."));

        let again =
            crate::grading::store_external_page(&conn, &dir, "s1", "a1", "center-asset-1", bytes)
                .unwrap();
        assert!(again.is_none(), "同一张中心照片不应重复落库");

        let count: i64 = conn
            .query_row(
                "select count(*) from local_attempt_asset where center_asset_id = 'center-asset-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let origin: String = conn
            .query_row(
                "select origin from local_attempt_asset where center_asset_id = 'center-asset-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(origin, "CENTER");
        assert!(dir.join("assets").join(&first.relative_path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
