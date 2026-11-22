//! 教学闭环：学生 / 班级 / 作业 / 作答 / 批改。
//!
//! 全部落在本地 SQLite，断网可用；同步由 `sync.rs` 的 outbox 后续接入。
//! 约定与题库一致：一律软删除，查询带 `deleted_at is null`。

use crate::insights;
use crate::models::Question;
use crate::repo;
use crate::text;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// 数据形状
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Student {
    pub id: String,
    /// 学号：学校里的固定编号，空字符串表示没编。
    pub student_no: String,
    pub name: String,
    pub grade: String,
    pub subject_ids: Vec<String>,
    pub subject_names: Vec<String>,
    pub note: String,
    pub center_user_id: Option<String>,
    pub status: String,
    pub class_count: i64,
    pub attempt_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudentInput {
    pub id: Option<String>,
    pub name: String,
    /// 留空则新建时自动编号（1001 起，跳过已用的号）。
    #[serde(default)]
    pub student_no: String,
    #[serde(default)]
    pub grade: String,
    #[serde(default)]
    pub subject_ids: Vec<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassRow {
    pub id: String,
    pub name: String,
    pub subject_id: Option<String>,
    pub subject_name: String,
    pub term: String,
    pub join_code: String,
    pub status: String,
    pub member_count: i64,
    pub assignment_count: i64,
    pub open_assignment_count: i64,
    pub pending_grade: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassInput {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub subject_id: Option<String>,
    #[serde(default)]
    pub term: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentRow {
    pub id: String,
    pub class_id: String,
    pub class_name: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub description: String,
    pub due_at: Option<String>,
    pub late_hours: i64,
    pub status: String,
    pub question_count: i64,
    pub total_score: f64,
    pub student_count: i64,
    pub submitted_count: i64,
    pub pending_grade: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentQuestionInput {
    pub question_id: String,
    #[serde(default = "one")]
    pub score: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentInput {
    pub id: Option<String>,
    pub class_id: String,
    pub title: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub due_at: Option<String>,
    #[serde(default)]
    pub late_hours: i64,
    #[serde(default)]
    pub questions: Vec<AssignmentQuestionInput>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentQuestionRow {
    pub ord: i64,
    pub score: f64,
    pub question: Question,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionRow {
    pub student_id: String,
    pub student_name: String,
    pub submission_id: Option<String>,
    pub submitted_at: Option<String>,
    pub late: bool,
    pub score: f64,
    pub max_score: f64,
    pub pending: i64,
    pub graded: bool,
    /// 这个学生交上来的照片数（整页 + 题级）。批改台用来看「谁还没拍」。
    pub photo_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignmentDetail {
    pub assignment: AssignmentRow,
    pub questions: Vec<AssignmentQuestionRow>,
    pub submissions: Vec<SubmissionRow>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnswerInput {
    pub question_id: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitInput {
    pub assignment_id: String,
    pub student_id: String,
    #[serde(default)]
    pub answers: Vec<AnswerInput>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitResult {
    pub submission_id: String,
    pub answered: i64,
    pub graded: i64,
    pub pending: i64,
    pub score: f64,
    pub max_score: f64,
    pub late: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeInput {
    pub attempt_id: String,
    pub score: f64,
    #[serde(default)]
    pub correct: Option<bool>,
    #[serde(default)]
    pub remark: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptItem {
    pub attempt_id: String,
    pub question: Question,
    pub answer: String,
    pub score: Option<f64>,
    pub max_score: f64,
    pub correct: Option<bool>,
    pub remark: String,
    pub graded_by: String,
    pub graded_at: Option<String>,
    /// 挂在这道题上的作答照片（拍照批改）。在线作答时是空的。
    pub photos: Vec<crate::grading::PhotoAsset>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptSheet {
    pub assignment: AssignmentRow,
    pub student: Student,
    pub submission_id: Option<String>,
    pub submitted_at: Option<String>,
    pub late: bool,
    pub items: Vec<AttemptItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRow {
    pub student_no: String,
    pub name: String,
    pub grade: String,
    pub note: String,
    pub duplicate: bool,
    /// 学号已被别人占用：不算重复学生，但也不能直接建。
    pub conflict: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub rows: Vec<ImportRow>,
    pub total: i64,
    pub duplicates: i64,
    /// 学号已被占用的行数（不是重复学生，但会被跳过）。
    pub conflicts: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub created: i64,
    pub skipped: i64,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

// 邀请码字符集与中心一致：去掉 I/O/0/1，避免口头传达时听错。
const CODE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn new_code() -> String {
    let bytes = uuid::Uuid::new_v4();
    let raw = bytes.as_bytes();
    let mut out = String::with_capacity(6);
    for index in 0..6 {
        let byte = raw[index];
        out.push(CODE_CHARS[(byte as usize) % CODE_CHARS.len()] as char);
    }
    out
}

fn subject_names(conn: &Connection) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare("select id, name from subject")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn parse_ids(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn map_student(
    row: &rusqlite::Row<'_>,
    names: &HashMap<String, String>,
) -> rusqlite::Result<Student> {
    let subject_ids = parse_ids(&row.get::<_, String>(3)?);
    Ok(Student {
        id: row.get(0)?,
        name: row.get(1)?,
        grade: row.get(2)?,
        subject_names: subject_ids
            .iter()
            .filter_map(|id| names.get(id).cloned())
            .collect(),
        subject_ids,
        note: row.get(4)?,
        center_user_id: row.get(5)?,
        status: row.get(6)?,
        class_count: row.get(7)?,
        attempt_count: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        student_no: row.get(11)?,
    })
}

const STUDENT_SELECT: &str =
    "select s.id, s.name, s.grade, s.subject_ids, s.note, s.center_user_id,
        s.status,
        (select count(*) from local_class_member m join local_class c on c.id = m.class_id
          where m.student_id = s.id and c.deleted_at is null) as class_count,
        (select count(*) from local_attempt a
          where a.student_id = s.id and a.superseded_at is null) as attempt_count,
        s.created_at, s.updated_at, s.student_no
   from local_student s";

// ---------------------------------------------------------------------------
// 学生
// ---------------------------------------------------------------------------

pub fn student_list(
    conn: &Connection,
    keyword: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<Student>, String> {
    let names = subject_names(conn)?;
    let mut sql = format!("{STUDENT_SELECT} where s.deleted_at is null");
    let mut args: Vec<String> = Vec::new();
    if let Some(word) = keyword.map(str::trim).filter(|value| !value.is_empty()) {
        sql.push_str(
            " and (s.name like ? or s.grade like ? or s.note like ? or s.student_no like ?)",
        );
        let pattern = format!("%{word}%");
        args.push(pattern.clone());
        args.push(pattern.clone());
        args.push(pattern.clone());
        args.push(pattern);
    }
    match status.unwrap_or("ACTIVE") {
        "ALL" => {}
        "ARCHIVED" => sql.push_str(" and s.status = 'ARCHIVED'"),
        _ => sql.push_str(" and s.status = 'ACTIVE'"),
    }
    sql.push_str(" order by s.name collate nocase");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
            map_student(row, &names)
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn student_get(conn: &Connection, id: &str) -> Result<Student, String> {
    let names = subject_names(conn)?;
    conn.query_row(&format!("{STUDENT_SELECT} where s.id = ?1"), [id], |row| {
        map_student(row, &names)
    })
    .map_err(|_| "找不到这个学生".to_string())
}

/// 下一个可用学号：1001 起，跳过已占用的。
/// 只认纯数字；手工填的「2026-01」这类不参与计算，免得把序号顶到天上。
pub fn next_student_no(conn: &Connection) -> String {
    let mut stmt = match conn.prepare(
        "select student_no from local_student
          where deleted_at is null and student_no <> '' and student_no not glob '*[^0-9]*'",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return "1001".into(),
    };
    let max = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map(|rows| {
            rows.filter_map(Result::ok)
                .filter_map(|no| no.parse::<i64>().ok())
                .max()
                .unwrap_or(1000)
        })
        .unwrap_or(1000);
    (max + 1).to_string()
}

/// 学号是否已被别人占用（软删的不算）。
fn student_no_taken(conn: &Connection, student_no: &str, except_id: Option<&str>) -> bool {
    conn.query_row(
        "select exists(select 1 from local_student
           where deleted_at is null and student_no = ?1 and id <> coalesce(?2, ''))",
        params![student_no, except_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| value != 0)
    .unwrap_or(false)
}

pub fn student_save(conn: &Connection, input: &StudentInput) -> Result<String, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("学生姓名不能为空".into());
    }
    let now = text::now();
    let subjects = serde_json::to_string(&input.subject_ids).unwrap_or_else(|_| "[]".into());
    let typed_no = input.student_no.trim();
    match input.id.as_deref().filter(|value| !value.is_empty()) {
        Some(id) => {
            // 学号留空 = 不动；想清空得显式给一个空格以外的值也不清，避免误删。
            let student_no = if typed_no.is_empty() {
                conn.query_row(
                    "select student_no from local_student where id = ?1",
                    [id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap_or_default()
            } else {
                if student_no_taken(conn, typed_no, Some(id)) {
                    return Err(format!("学号 {typed_no} 已经被别的学生用了"));
                }
                typed_no.to_string()
            };
            let changed = conn
                .execute(
                    "update local_student set name = ?2, grade = ?3, subject_ids = ?4,
                        note = ?5, status = coalesce(?6, status), updated_at = ?7,
                        student_no = ?8
                     where id = ?1 and deleted_at is null",
                    params![
                        id,
                        name,
                        input.grade.trim(),
                        subjects,
                        input.note.trim(),
                        input.status,
                        now,
                        student_no
                    ],
                )
                .map_err(|e| e.to_string())?;
            if changed == 0 {
                return Err("找不到这个学生".into());
            }
            Ok(id.to_string())
        }
        None => {
            // 手工新建允许同名（一个班真可能有两个张伟）；只有批量导入按「同名同年级」去重。
            let student_no = if typed_no.is_empty() {
                next_student_no(conn)
            } else {
                if student_no_taken(conn, typed_no, None) {
                    return Err(format!("学号 {typed_no} 已经被别的学生用了"));
                }
                typed_no.to_string()
            };
            let id = new_id();
            conn.execute(
                "insert into local_student
                    (id, name, grade, subject_ids, note, status, created_at, updated_at, student_no)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
                params![
                    id,
                    name,
                    input.grade.trim(),
                    subjects,
                    input.note.trim(),
                    input.status.clone().unwrap_or_else(|| "ACTIVE".into()),
                    now,
                    student_no
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(id)
        }
    }
}

pub fn student_archive(conn: &Connection, id: &str, archived: bool) -> Result<(), String> {
    let changed = conn
        .execute(
            "update local_student set status = ?2, updated_at = ?3 where id = ?1 and deleted_at is null",
            params![id, if archived { "ARCHIVED" } else { "ACTIVE" }, text::now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("找不到这个学生".into());
    }
    Ok(())
}

pub fn student_delete(conn: &Connection, id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "update local_student set deleted_at = ?2, updated_at = ?2 where id = ?1 and deleted_at is null",
            params![id, text::now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("找不到这个学生".into());
    }
    Ok(())
}

/// 一行导入内容，列可以省、可以不按顺序（有表头时按表头认）。
struct ImportRaw {
    line: usize,
    student_no: String,
    name: String,
    grade: String,
    note: String,
}

/// 表头认列：只认到就够，缺哪列就是空。
#[derive(Clone, Copy, Default)]
struct ImportHeader {
    no: Option<usize>,
    name: Option<usize>,
    grade: Option<usize>,
    note: Option<usize>,
}

fn header_of(cells: &[String]) -> Option<ImportHeader> {
    let find = |keys: &[&str]| {
        cells.iter().position(|cell| {
            let text = cell.trim().to_ascii_lowercase();
            keys.contains(&text.as_str())
        })
    };
    let name = find(&["姓名", "学生", "学生姓名", "name"]);
    name?;
    Some(ImportHeader {
        no: find(&["学号", "考号", "编号", "序号", "no", "id"]),
        name,
        grade: find(&["年级", "班级", "grade", "class"]),
        note: find(&["备注", "说明", "note"]),
    })
}

/// 纯数字（允许带前导零）才当学号，避免把「张伟」当成学号。
fn looks_like_no(cell: &str) -> bool {
    let text = cell.trim();
    !text.is_empty() && text.len() <= 24 && text.chars().all(|ch| ch.is_ascii_digit())
}

/// 逐行解析，兼容三种写法：
/// 1. 有表头（姓名 / 学号 / 年级 / 备注，顺序随意）；
/// 2. 无表头、老格式：姓名, 年级, 备注；
/// 3. 无表头、带学号：学号, 姓名, 年级, 备注。
/// 分隔符逗号、制表符、顿号、分号都认。
fn parse_import(text: &str) -> Vec<ImportRaw> {
    let mut header: Option<ImportHeader> = None;
    let mut first_line = true;
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let raw = line.trim().trim_start_matches(['-', '*', ' ']);
        if raw.is_empty() {
            continue;
        }
        let cells: Vec<String> = raw
            .split(['\t', ',', '，', ';', '；'])
            .map(|cell| cell.trim().to_string())
            .collect();
        // 表头认第一行非空内容；从表格复制时前面常带空行。
        if first_line {
            first_line = false;
            if let Some(found) = header_of(&cells) {
                header = Some(found);
                continue;
            }
        }
        let (student_no, name, grade, note) = match header {
            Some(map) => {
                let pick = |slot: Option<usize>| {
                    slot.and_then(|at| cells.get(at).cloned())
                        .unwrap_or_default()
                };
                (
                    pick(map.no),
                    pick(map.name),
                    pick(map.grade),
                    pick(map.note),
                )
            }
            None => {
                // 老格式第一列一定是姓名；只有当第一列像学号、第二列不像时才右移一位。
                let shifted =
                    cells.len() >= 3 && looks_like_no(&cells[0]) && !looks_like_no(&cells[1]);
                if shifted {
                    (
                        cells[0].clone(),
                        cells[1].clone(),
                        cells.get(2).cloned().unwrap_or_default(),
                        cells.get(3).cloned().unwrap_or_default(),
                    )
                } else {
                    (
                        String::new(),
                        cells[0].clone(),
                        cells.get(1).cloned().unwrap_or_default(),
                        cells.get(2).cloned().unwrap_or_default(),
                    )
                }
            }
        };
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        out.push(ImportRaw {
            line: index + 1,
            student_no: student_no.trim().to_string(),
            name,
            grade: grade.trim().to_string(),
            note: note.trim().to_string(),
        });
    }
    out
}

pub fn student_import_preview(conn: &Connection, text: &str) -> Result<ImportPreview, String> {
    let rows = parse_import(text);
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut seen_nos: Vec<String> = Vec::new();
    let mut out = Vec::new();
    let mut duplicates = 0;
    let mut conflicts = 0;
    for raw in rows {
        let exists: bool = conn
            .query_row(
                "select exists(select 1 from local_student where deleted_at is null and name = ?1 and grade = ?2)",
                params![raw.name, raw.grade],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value != 0)
            .unwrap_or(false);
        let repeated = seen
            .iter()
            .any(|(name, grade)| name == &raw.name && grade == &raw.grade);
        let duplicate = exists || repeated;
        // 学号重复但人不一样：直接建会撞唯一索引，预览里先标出来。
        // 同一份名单里自己重复也算——两行同号必然有一行建不进去。
        let conflict = !raw.student_no.is_empty()
            && !duplicate
            && (student_no_taken(conn, &raw.student_no, None)
                || seen_nos.iter().any(|no| no == &raw.student_no));
        if duplicate {
            duplicates += 1;
        }
        if conflict {
            conflicts += 1;
        }
        if !raw.student_no.is_empty() {
            seen_nos.push(raw.student_no.clone());
        }
        seen.push((raw.name.clone(), raw.grade.clone()));
        out.push(ImportRow {
            student_no: raw.student_no,
            name: raw.name,
            grade: raw.grade,
            note: raw.note,
            duplicate,
            conflict,
        });
    }
    Ok(ImportPreview {
        total: out.len() as i64,
        duplicates,
        conflicts,
        rows: out,
    })
}

pub fn student_import_commit(conn: &mut Connection, text: &str) -> Result<ImportResult, String> {
    let rows = parse_import(text);
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let now = text::now();
    let mut created = 0;
    let mut skipped = 0;
    let mut used_nos: Vec<String> = Vec::new();
    let mut errors = Vec::new();
    for raw in rows {
        let exists: i64 = tx
            .query_row(
                "select count(*) from local_student where deleted_at is null and name = ?1 and grade = ?2",
                params![raw.name, raw.grade],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if exists > 0 {
            skipped += 1;
            continue;
        }
        let taken: i64 = if raw.student_no.is_empty() {
            0
        } else {
            tx.query_row(
                "select count(*) from local_student where deleted_at is null and student_no = ?1",
                [&raw.student_no],
                |row| row.get(0),
            )
            .unwrap_or(0)
        };
        if taken > 0 || used_nos.iter().any(|no| no == &raw.student_no) {
            errors.push(format!(
                "第 {} 行（{}）：学号 {} 已被别的学生占用",
                raw.line, raw.name, raw.student_no
            ));
            continue;
        }
        let student_no = if raw.student_no.is_empty() {
            next_student_no(&tx)
        } else {
            raw.student_no.clone()
        };
        let result = tx.execute(
            "insert into local_student
                (id, name, grade, subject_ids, note, status, created_at, updated_at, student_no)
             values (?1, ?2, ?3, '[]', ?4, 'ACTIVE', ?5, ?5, ?6)",
            params![new_id(), raw.name, raw.grade, raw.note, now, student_no],
        );
        match result {
            Ok(_) => {
                created += 1;
                used_nos.push(student_no);
            }
            Err(error) => errors.push(format!("第 {} 行（{}）：{error}", raw.line, raw.name)),
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(ImportResult {
        created,
        skipped,
        errors,
    })
}

// ---------------------------------------------------------------------------
// 班级
// ---------------------------------------------------------------------------

const CLASS_SELECT: &str =
    "select c.id, c.name, c.subject_id, coalesce(s.name, ''), c.term, c.join_code,
        c.status, c.created_at,
        (select count(*) from local_class_member m join local_student st on st.id = m.student_id
          where m.class_id = c.id and st.deleted_at is null) as member_count,
        (select count(*) from local_assignment a
          where a.class_id = c.id and a.deleted_at is null) as assignment_count,
        (select count(*) from local_assignment a
          where a.class_id = c.id and a.deleted_at is null and a.status <> 'DRAFT') as open_count,
        (select count(*) from local_attempt t
           join local_assignment a on a.id = t.assignment_id
          where a.class_id = c.id and a.deleted_at is null
            and t.superseded_at is null and t.correct is null) as pending_grade
   from local_class c
   left join subject s on s.id = c.subject_id";

fn map_class(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClassRow> {
    Ok(ClassRow {
        id: row.get(0)?,
        name: row.get(1)?,
        subject_id: row.get(2)?,
        subject_name: row.get(3)?,
        term: row.get(4)?,
        join_code: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        member_count: row.get(8)?,
        assignment_count: row.get(9)?,
        open_assignment_count: row.get(10)?,
        pending_grade: row.get(11)?,
    })
}

pub fn class_list(conn: &Connection) -> Result<Vec<ClassRow>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "{CLASS_SELECT} where c.deleted_at is null order by c.status = 'CLOSED', c.updated_at desc"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], map_class).map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn class_get(conn: &Connection, id: &str) -> Result<ClassRow, String> {
    conn.query_row(&format!("{CLASS_SELECT} where c.id = ?1"), [id], map_class)
        .map_err(|_| "找不到这个班级".to_string())
}

pub fn class_save(conn: &Connection, input: &ClassInput) -> Result<String, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("班级名称不能为空".into());
    }
    let now = text::now();
    match input.id.as_deref().filter(|value| !value.is_empty()) {
        Some(id) => {
            let changed = conn
                .execute(
                    "update local_class set name = ?2, subject_id = ?3, term = ?4, updated_at = ?5
                     where id = ?1 and deleted_at is null",
                    params![id, name, input.subject_id, input.term.trim(), now],
                )
                .map_err(|e| e.to_string())?;
            if changed == 0 {
                return Err("找不到这个班级".into());
            }
            Ok(id.to_string())
        }
        None => {
            let id = new_id();
            // 邀请码撞了就换一个，最多试 8 次。
            for _ in 0..8 {
                let code = new_code();
                let taken: i64 = conn
                    .query_row(
                        "select count(*) from local_class where join_code = ?1 and deleted_at is null",
                        [&code],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                if taken > 0 {
                    continue;
                }
                conn.execute(
                    "insert into local_class (id, name, subject_id, term, join_code, status, created_at, updated_at)
                     values (?1, ?2, ?3, ?4, ?5, 'ACTIVE', ?6, ?6)",
                    params![id, name, input.subject_id, input.term.trim(), code, now],
                )
                .map_err(|e| e.to_string())?;
                return Ok(id);
            }
            Err("生成邀请码失败，请重试".into())
        }
    }
}

pub fn class_close(conn: &Connection, id: &str, closed: bool) -> Result<(), String> {
    let changed = conn
        .execute(
            "update local_class set status = ?2, updated_at = ?3 where id = ?1 and deleted_at is null",
            params![id, if closed { "CLOSED" } else { "ACTIVE" }, text::now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("找不到这个班级".into());
    }
    Ok(())
}

pub fn class_delete(conn: &Connection, id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "update local_class set deleted_at = ?2, updated_at = ?2 where id = ?1 and deleted_at is null",
            params![id, text::now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("找不到这个班级".into());
    }
    Ok(())
}

pub fn class_regenerate_code(conn: &Connection, id: &str) -> Result<String, String> {
    for _ in 0..8 {
        let code = new_code();
        let taken: i64 = conn
            .query_row(
                "select count(*) from local_class where join_code = ?1 and deleted_at is null and id <> ?2",
                params![code, id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if taken > 0 {
            continue;
        }
        let changed = conn
            .execute(
                "update local_class set join_code = ?2, updated_at = ?3 where id = ?1 and deleted_at is null",
                params![id, code, text::now()],
            )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("找不到这个班级".into());
        }
        return Ok(code);
    }
    Err("生成邀请码失败，请重试".into())
}

pub fn class_members(conn: &Connection, class_id: &str) -> Result<Vec<Student>, String> {
    let names = subject_names(conn)?;
    let sql = format!(
        "{STUDENT_SELECT} join local_class_member m on m.student_id = s.id
         where m.class_id = ?1 and s.deleted_at is null order by s.name collate nocase"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([class_id], |row| map_student(row, &names))
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn class_add_members(
    conn: &mut Connection,
    class_id: &str,
    student_ids: &[String],
) -> Result<i64, String> {
    if student_ids.is_empty() {
        return Ok(0);
    }
    let exists: i64 = conn
        .query_row(
            "select count(*) from local_class where id = ?1 and deleted_at is null",
            [class_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        return Err("找不到这个班级".into());
    }
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let now = text::now();
    let mut added = 0;
    for student_id in student_ids {
        let changed = tx
            .execute(
                "insert or ignore into local_class_member (class_id, student_id, joined_at)
                 values (?1, ?2, ?3)",
                params![class_id, student_id, now],
            )
            .map_err(|e| e.to_string())?;
        added += changed as i64;
    }
    tx.execute(
        "update local_class set updated_at = ?2 where id = ?1",
        params![class_id, now],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(added)
}

pub fn class_remove_member(
    conn: &Connection,
    class_id: &str,
    student_id: &str,
) -> Result<(), String> {
    conn.execute(
        "delete from local_class_member where class_id = ?1 and student_id = ?2",
        params![class_id, student_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 作业
// ---------------------------------------------------------------------------

const ASSIGNMENT_SELECT: &str = "select a.id, a.class_id, c.name, a.title, a.type, a.description,
        a.due_at, a.late_hours, a.status, a.created_at, a.updated_at,
        (select count(*) from local_assignment_question q where q.assignment_id = a.id) as question_count,
        (select coalesce(sum(q.score), 0) from local_assignment_question q where q.assignment_id = a.id) as total_score,
        (select count(*) from local_class_member m join local_student st on st.id = m.student_id
          where m.class_id = a.class_id and st.deleted_at is null) as student_count,
        (select count(distinct t.student_id) from local_attempt t
          where t.assignment_id = a.id and t.superseded_at is null) as submitted_count,
        (select count(*) from local_attempt t
          where t.assignment_id = a.id and t.superseded_at is null and t.correct is null) as pending_grade
   from local_assignment a
   join local_class c on c.id = a.class_id";

fn map_assignment(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssignmentRow> {
    Ok(AssignmentRow {
        id: row.get(0)?,
        class_id: row.get(1)?,
        class_name: row.get(2)?,
        title: row.get(3)?,
        kind: row.get(4)?,
        description: row.get(5)?,
        due_at: row.get(6)?,
        late_hours: row.get(7)?,
        status: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        question_count: row.get(11)?,
        total_score: row.get(12)?,
        student_count: row.get(13)?,
        submitted_count: row.get(14)?,
        pending_grade: row.get(15)?,
    })
}

pub fn assignment_list(
    conn: &Connection,
    class_id: Option<&str>,
) -> Result<Vec<AssignmentRow>, String> {
    let mut sql = format!("{ASSIGNMENT_SELECT} where a.deleted_at is null");
    let mut args: Vec<String> = Vec::new();
    if let Some(id) = class_id.filter(|value| !value.is_empty()) {
        sql.push_str(" and a.class_id = ?1");
        args.push(id.to_string());
    }
    sql.push_str(" order by a.updated_at desc");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), map_assignment)
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn assignment_get(conn: &Connection, id: &str) -> Result<AssignmentRow, String> {
    conn.query_row(
        &format!("{ASSIGNMENT_SELECT} where a.id = ?1 and a.deleted_at is null"),
        [id],
        map_assignment,
    )
    .map_err(|_| "找不到这份作业".to_string())
}

pub fn assignment_save(conn: &mut Connection, input: &AssignmentInput) -> Result<String, String> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err("作业标题不能为空".into());
    }
    let kind = match input.kind.trim().to_uppercase().as_str() {
        "EXAM" => "EXAM",
        _ => "PRACTICE",
    };
    if input.questions.is_empty() {
        return Err("至少选一道题".into());
    }
    let class_exists: i64 = conn
        .query_row(
            "select count(*) from local_class where id = ?1 and deleted_at is null",
            [&input.class_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if class_exists == 0 {
        return Err("找不到这个班级".into());
    }

    let now = text::now();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let id = match input.id.as_deref().filter(|value| !value.is_empty()) {
        Some(id) => {
            let changed = tx
                .execute(
                    "update local_assignment set title = ?2, type = ?3, description = ?4,
                        due_at = ?5, late_hours = ?6, updated_at = ?7
                     where id = ?1 and deleted_at is null",
                    params![
                        id,
                        title,
                        kind,
                        input.description.trim(),
                        input.due_at,
                        input.late_hours.max(0),
                        now
                    ],
                )
                .map_err(|e| e.to_string())?;
            if changed == 0 {
                return Err("找不到这份作业".into());
            }
            id.to_string()
        }
        None => {
            let id = new_id();
            tx.execute(
                "insert into local_assignment
                    (id, class_id, title, type, description, due_at, late_hours, status, created_at, updated_at)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'DRAFT', ?8, ?8)",
                params![
                    id,
                    input.class_id,
                    title,
                    kind,
                    input.description.trim(),
                    input.due_at,
                    input.late_hours.max(0),
                    now
                ],
            )
            .map_err(|e| e.to_string())?;
            id
        }
    };

    // 组卷是整体替换：删掉的题不再计分，改分值也一并更新。
    tx.execute(
        "delete from local_assignment_question where assignment_id = ?1",
        [&id],
    )
    .map_err(|e| e.to_string())?;
    for (index, item) in input.questions.iter().enumerate() {
        tx.execute(
            "insert or replace into local_assignment_question (assignment_id, question_id, ord, score)
             values (?1, ?2, ?3, ?4)",
            params![id, item.question_id, index as i64, item.score.max(0.0)],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}

pub fn assignment_publish(conn: &Connection, id: &str) -> Result<(), String> {
    let questions: i64 = conn
        .query_row(
            "select count(*) from local_assignment_question where assignment_id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if questions == 0 {
        return Err("这份作业还没有题目，先选题再发布".into());
    }
    let changed = conn
        .execute(
            "update local_assignment set status = 'PUBLISHED', updated_at = ?2
             where id = ?1 and deleted_at is null and status = 'DRAFT'",
            params![id, text::now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("找不到草稿状态的作业".into());
    }
    Ok(())
}

pub fn assignment_close(conn: &Connection, id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "update local_assignment set status = 'CLOSED', updated_at = ?2
             where id = ?1 and deleted_at is null",
            params![id, text::now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("找不到这份作业".into());
    }
    Ok(())
}

pub fn assignment_reopen(conn: &Connection, id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "update local_assignment set status = 'PUBLISHED', updated_at = ?2
             where id = ?1 and deleted_at is null",
            params![id, text::now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("找不到这份作业".into());
    }
    Ok(())
}

pub fn assignment_delete(conn: &Connection, id: &str) -> Result<(), String> {
    let changed = conn
        .execute(
            "update local_assignment set deleted_at = ?2, updated_at = ?2 where id = ?1 and deleted_at is null",
            params![id, text::now()],
        )
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err("找不到这份作业".into());
    }
    Ok(())
}

pub fn assignment_questions(
    conn: &Connection,
    assignment_id: &str,
) -> Result<Vec<AssignmentQuestionRow>, String> {
    let pairs: Vec<(String, i64, f64)> = {
        let mut stmt = conn
            .prepare(
                "select question_id, ord, score from local_assignment_question
                 where assignment_id = ?1 order by ord asc",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([assignment_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.filter_map(Result::ok).collect()
    };
    let mut out = Vec::new();
    for (question_id, ord, score) in pairs {
        // 题目被彻底删掉时跳过，不让整份作业打不开。
        if let Ok(question) = repo::get(conn, &question_id) {
            out.push(AssignmentQuestionRow {
                ord,
                score,
                question,
            });
        }
    }
    Ok(out)
}

pub fn assignment_detail(conn: &Connection, id: &str) -> Result<AssignmentDetail, String> {
    let assignment = assignment_get(conn, id)?;
    let questions = assignment_questions(conn, id)?;
    let submissions = submission_rows(conn, &assignment)?;
    Ok(AssignmentDetail {
        assignment,
        questions,
        submissions,
    })
}

fn submission_rows(
    conn: &Connection,
    assignment: &AssignmentRow,
) -> Result<Vec<SubmissionRow>, String> {
    let members: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(
                "select s.id, s.name from local_class_member m
                   join local_student s on s.id = m.student_id
                  where m.class_id = ?1 and s.deleted_at is null
                  order by s.name collate nocase",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([&assignment.class_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.filter_map(Result::ok).collect()
    };

    let mut out = Vec::new();
    for (student_id, student_name) in members {
        let summary: Option<(String, String, f64, f64, i64, i64, i64)> = conn
            .query_row(
                "select submission_id, max(submitted_at), coalesce(sum(score), 0),
                        coalesce(sum(max_score), 0),
                        sum(case when correct is null then 1 else 0 end),
                        count(*), max(late)
                   from local_attempt
                  where assignment_id = ?1 and student_id = ?2 and superseded_at is null
                  group by submission_id
                  order by max(submitted_at) desc limit 1",
                params![assignment.id, student_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        row.get(5)?,
                        row.get::<_, Option<i64>>(6)?.unwrap_or(0),
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let photo_count: i64 = conn
            .query_row(
                "select count(*) from local_attempt_asset
                  where student_id = ?1 and deleted_at is null
                    and (assignment_id = ?2 or assignment_id is null)",
                params![student_id, assignment.id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        out.push(match summary {
            Some((submission_id, submitted_at, score, max_score, pending, _, late)) => {
                SubmissionRow {
                    student_id,
                    student_name,
                    submission_id: Some(submission_id),
                    submitted_at: Some(submitted_at),
                    late: late != 0,
                    score,
                    // 作业改过题后 max_score 以当前组卷为准，避免分母看着不对。
                    max_score: if assignment.total_score > 0.0 {
                        assignment.total_score
                    } else {
                        max_score
                    },
                    pending,
                    graded: pending == 0,
                    photo_count,
                }
            }
            None => SubmissionRow {
                student_id,
                student_name,
                submission_id: None,
                submitted_at: None,
                late: false,
                score: 0.0,
                max_score: assignment.total_score,
                pending: 0,
                graded: false,
                photo_count,
            },
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 作答与批改
// ---------------------------------------------------------------------------

/// 截止时间两种写法都认：带时区的 RFC3339（中心下发）和 `datetime-local` 的本地时间。
fn parse_due(raw: &str) -> Option<chrono::DateTime<chrono::Local>> {
    if let Ok(at) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(at.with_timezone(&chrono::Local));
    }
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M"))
        .ok()
        .and_then(|naive| naive.and_local_timezone(chrono::Local).single())
}

/// 客观题判分：单选比字母集合，多选比集合是否完全相同；填空题和解答题返回 None（留给老师）。
pub fn grade_answer(kind: &str, expected: &str, given: &str) -> Option<bool> {
    let pick = |raw: &str| -> Vec<char> {
        let mut letters: Vec<char> = raw
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        letters.sort_unstable();
        letters.dedup();
        letters
    };
    let expected = pick(expected);
    if expected.is_empty() {
        return None;
    }
    let given = pick(given);
    match text::normalize_type(kind) {
        Some("SINGLE") => Some(given.len() == 1 && given[0] == expected[0]),
        Some("MULTI") => Some(given == expected),
        _ => None,
    }
}

pub fn submit(conn: &mut Connection, input: &SubmitInput) -> Result<SubmitResult, String> {
    let assignment = assignment_get(conn, &input.assignment_id)?;
    if assignment.status == "DRAFT" {
        return Err("这份作业还没发布".into());
    }
    let member: i64 = conn
        .query_row(
            "select count(*) from local_class_member where class_id = ?1 and student_id = ?2",
            params![assignment.class_id, input.student_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if member == 0 {
        return Err("这个学生不在班里".into());
    }
    let questions = assignment_questions(conn, &input.assignment_id)?;
    if questions.is_empty() {
        return Err("这份作业没有题目".into());
    }

    let now = text::now();
    let late = assignment
        .due_at
        .as_deref()
        .and_then(parse_due)
        .map(|due| chrono::Local::now() > due)
        .unwrap_or(false);
    let answers: HashMap<&str, &AnswerInput> = input
        .answers
        .iter()
        .map(|item| (item.question_id.as_str(), item))
        .collect();

    let submission_id = new_id();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    // 重新交：旧的那次标成 superseded，成绩单只看最新一次。
    tx.execute(
        "update local_attempt set superseded_at = ?3
          where assignment_id = ?1 and student_id = ?2 and superseded_at is null",
        params![input.assignment_id, input.student_id, now],
    )
    .map_err(|e| e.to_string())?;

    let mut graded = 0;
    let mut pending = 0;
    let mut answered = 0;
    let mut score_total = 0.0;
    for row in &questions {
        let answer = answers
            .get(row.question.id.as_str())
            .map(|item| item.answer.trim().to_string())
            .unwrap_or_default();
        let duration = answers
            .get(row.question.id.as_str())
            .and_then(|item| item.duration_ms);
        let max_score = row.score;
        if !answer.is_empty() {
            answered += 1;
        }
        let judgement = if answer.is_empty() {
            Some(false)
        } else {
            grade_answer(&row.question.kind, &row.question.answer, &answer)
        };
        let (correct, score) = match judgement {
            Some(true) => {
                graded += 1;
                score_total += max_score;
                (Some(1i64), Some(max_score))
            }
            Some(false) => {
                graded += 1;
                (Some(0i64), Some(0.0))
            }
            None => {
                pending += 1;
                (None, None)
            }
        };
        let attempt_id = new_id();
        tx.execute(
            "insert into local_attempt
                (id, assignment_id, question_id, student_id, submission_id, answer, duration_ms,
                 submitted_at, correct, score, max_score, late)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                &attempt_id,
                input.assignment_id,
                row.question.id,
                input.student_id,
                &submission_id,
                answer,
                duration,
                &now,
                correct,
                score,
                max_score,
                late as i64
            ],
        )
        .map_err(|e| e.to_string())?;
        // 错题本跟着判分走：做错入本，做对把旧账结掉。待批的等老师给分时再处理。
        match correct {
            Some(0) => {
                insights::record_wrong(&tx, &input.student_id, &row.question.id, &attempt_id)
            }
            Some(_) => {
                insights::resolve_wrong(&tx, &input.student_id, &row.question.id, &attempt_id)
            }
            None => {}
        }
    }
    tx.execute(
        "update local_assignment set updated_at = ?2 where id = ?1",
        params![input.assignment_id, now],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;

    Ok(SubmitResult {
        submission_id,
        answered,
        graded,
        pending,
        score: score_total,
        max_score: assignment.total_score,
        late,
    })
}

pub fn grade(conn: &mut Connection, input: &GradeInput, teacher: &str) -> Result<(), String> {
    let (max_score, current_remark): (f64, String) = conn
        .query_row(
            "select max_score, remark from local_attempt where id = ?1",
            [&input.attempt_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "找不到这条作答".to_string())?;
    if !input.score.is_finite() || input.score < 0.0 || input.score > max_score + 1e-6 {
        return Err(format!("分数要在 0 到 {max_score} 之间"));
    }
    // 没显式给对错就按 60% 推导，和中心规则一致。
    let correct = input
        .correct
        .map(|value| if value { 1i64 } else { 0i64 })
        .unwrap_or(if input.score >= max_score * 0.6 { 1 } else { 0 });
    let remark = if input.remark.trim().is_empty() {
        current_remark
    } else {
        input.remark.trim().to_string()
    };
    conn.execute(
        "update local_attempt set score = ?2, correct = ?3, remark = ?4, graded_by = ?5, graded_at = ?6
         where id = ?1",
        params![input.attempt_id, input.score, correct, remark, teacher, text::now()],
    )
    .map_err(|e| e.to_string())?;
    // 主观题给完分才知道对错，错题本在这一步补写。
    if let Ok((student_id, question_id)) = conn.query_row(
        "select student_id, question_id from local_attempt where id = ?1",
        [&input.attempt_id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ) {
        if correct == 1 {
            insights::resolve_wrong(conn, &student_id, &question_id, &input.attempt_id);
        } else {
            insights::record_wrong(conn, &student_id, &question_id, &input.attempt_id);
        }
    }
    Ok(())
}

pub fn batch_grade(
    conn: &mut Connection,
    items: &[GradeInput],
    teacher: &str,
) -> Result<i64, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut done = 0;
    for item in items {
        let (max_score, remark): (f64, String) = tx
            .query_row(
                "select max_score, remark from local_attempt where id = ?1",
                [&item.attempt_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "找不到这条作答".to_string())?;
        if !item.score.is_finite() || item.score < 0.0 || item.score > max_score + 1e-6 {
            return Err(format!("分数要在 0 到 {max_score} 之间"));
        }
        let correct = item
            .correct
            .map(|value| if value { 1i64 } else { 0i64 })
            .unwrap_or(if item.score >= max_score * 0.6 { 1 } else { 0 });
        let next_remark = if item.remark.trim().is_empty() {
            remark
        } else {
            item.remark.trim().to_string()
        };
        tx.execute(
            "update local_attempt set score = ?2, correct = ?3, remark = ?4, graded_by = ?5, graded_at = ?6
             where id = ?1",
            params![item.attempt_id, item.score, correct, next_remark, teacher, text::now()],
        )
        .map_err(|e| e.to_string())?;
        done += 1;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(done)
}

pub fn attempt_sheet(
    conn: &Connection,
    assignment_id: &str,
    student_id: &str,
) -> Result<AttemptSheet, String> {
    let assignment = assignment_get(conn, assignment_id)?;
    let student = student_get(conn, student_id)?;
    let submission: Option<(String, String, i64)> = conn
        .query_row(
            "select submission_id, max(submitted_at), max(late) from local_attempt
              where assignment_id = ?1 and student_id = ?2 and superseded_at is null
              group by submission_id order by max(submitted_at) desc limit 1",
            params![assignment_id, student_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    if let Some((submission_id, _, _)) = &submission {
        let rows: Vec<String> = {
            let mut stmt = conn
                .prepare("select id from local_attempt where submission_id = ?1 order by rowid asc")
                .map_err(|e| e.to_string())?;
            let mapped = stmt
                .query_map([submission_id], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            mapped.filter_map(Result::ok).collect()
        };
        for attempt_id in rows {
            let row: Option<(String, String, Option<f64>, f64, Option<i64>, String, String, Option<String>)> =
                conn.query_row(
                    "select question_id, answer, score, max_score, correct, remark, graded_by, graded_at
                       from local_attempt where id = ?1",
                    [&attempt_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                        ))
                    },
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let Some((
                question_id,
                answer,
                score,
                max_score,
                correct,
                remark,
                graded_by,
                graded_at,
            )) = row
            else {
                continue;
            };
            let Ok(question) = repo::get(conn, &question_id) else {
                continue;
            };
            let photos = crate::grading::list_by_attempt(conn, &attempt_id).unwrap_or_default();
            items.push(AttemptItem {
                attempt_id,
                question,
                answer,
                score,
                max_score,
                correct: correct.map(|value| value != 0),
                remark,
                graded_by,
                graded_at,
                photos,
            });
        }
    }

    Ok(AttemptSheet {
        assignment,
        student,
        submission_id: submission.as_ref().map(|(id, _, _)| id.clone()),
        submitted_at: submission.as_ref().map(|(_, at, _)| at.clone()),
        late: submission.map(|(_, _, late)| late != 0).unwrap_or(false),
        items,
    })
}

/// 撤销一次提交，让学生可以重做（旧分数一并作废）。
pub fn attempt_clear(
    conn: &Connection,
    assignment_id: &str,
    student_id: &str,
) -> Result<i64, String> {
    let changed = conn
        .execute(
            "delete from local_attempt where assignment_id = ?1 and student_id = ?2",
            params![assignment_id, student_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(changed as i64)
}

/// 首页 / 班级页用的概览数字。
pub fn overview(conn: &Connection) -> Result<serde_json::Value, String> {
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap_or(0) };
    Ok(json!({
        "students": count("select count(*) from local_student where deleted_at is null and status = 'ACTIVE'"),
        "classes": count("select count(*) from local_class where deleted_at is null and status = 'ACTIVE'"),
        "assignments": count("select count(*) from local_assignment where deleted_at is null and status <> 'DRAFT'"),
        "pendingGrade": count(
            "select count(*) from local_attempt where superseded_at is null and correct is null"
        ),
    }))
}
