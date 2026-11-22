//! 本机存储盘点与清理。
//!
//! 为什么要分两档：
//!
//! - `tmp/` 放的是**过程图**：截图选区、PDF 页渲染、HEIC 转 PNG、多屏拼图。识别完、入题时
//!   会复制一份到 `assets/<题目 id>/`，原件就没人再用了，随时可以删。
//! - `assets/` 放的是**题目制图**：题干和解析里的 `![](nbasset:<id>)` 指着它，删了题目就缺图，
//!   导出和整库备份也靠它。所以「清理临时文件」绝不会碰这个目录。
//!
//! 唯一允许动 `assets/` 的是「无主图片」：文件在盘上、但 `question_asset` 里没有任何引用
//! （大多是导入中断或题目被彻底删除时漏下的）。这类清理也只在用户明确点按钮后执行。
//!
//! 两条保护措施贯穿全程：
//! 1. 只操作 `data_dir` 下的 `tmp` / `assets` / `backup` 三个子目录，不做任何跨目录删除；
//! 2. 最近 10 分钟内改过的文件一律跳过——识别任务和预览可能正拿着这个路径读。

use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 刚落盘的图可能正被识别任务或 PDF 预览读取，这个窗口内不删。
pub const GUARD: Duration = Duration::from_secs(10 * 60);

/// 备份包只保留最近几份，再老的由用户点「清理旧备份」删掉。
pub const KEEP_BACKUPS: usize = 3;

#[derive(Default, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub bytes: u64,
    pub files: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub data_dir: String,
    pub db_path: String,
    pub total: Bucket,
    pub db: Bucket,
    pub assets: Bucket,
    pub tmp: Bucket,
    pub backups: Bucket,
    pub other: Bucket,
    /// 题目图覆盖了多少道题（不是文件数：一道题可能有多张图）。
    pub assets_questions: u64,
    /// 因「太新」而被保护下来的临时文件数。
    pub tmp_recent: u64,
    /// 连拍队列里还没处理完、清理时必须留下的图。
    pub tmp_pinned: u64,
    pub orphan_files: u64,
    pub orphan_bytes: u64,
    pub disk_total: Option<u64>,
    pub disk_free: Option<u64>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    /// 删掉的条目数（目录按一项算）。
    pub removed: u64,
    pub freed_bytes: u64,
    /// 因为太新而留下的条目数。
    pub kept: u64,
    /// 还在连拍队列里等着识别的图，删了就得重拍，一律留着。
    pub pinned: u64,
    /// 删不动的条目数（权限、被占用等），不因此中断其余清理。
    pub errors: u64,
}

fn scan(path: &Path) -> Bucket {
    let mut bucket = Bucket::default();
    let Ok(entries) = std::fs::read_dir(path) else {
        return bucket;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // 不跟符号链接：用户可能把外接盘链进来，删了就不是「清理临时文件」了。
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            let child = scan(&entry.path());
            bucket.bytes += child.bytes;
            bucket.files += child.files;
        } else if kind.is_file() {
            if let Ok(meta) = entry.metadata() {
                bucket.bytes += meta.len();
                bucket.files += 1;
            }
        }
    }
    bucket
}

fn is_recent(path: &Path, guard: Duration) -> bool {
    if guard.is_zero() {
        return false;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    // 时间戳落在未来（时区/校时抖动）也按「新」处理，宁可少删。
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age < guard)
        .unwrap_or(true)
}

fn count_recent(path: &Path, guard: Duration, out: &mut u64) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            count_recent(&entry.path(), guard, out);
        } else if is_recent(&entry.path(), guard) {
            *out += 1;
        }
    }
}

fn db_bucket(db_path: &Path) -> Bucket {
    let mut bucket = Bucket::default();
    // WAL 模式下 -wal / -shm 也是真实占用，别只算主文件。
    for suffix in ["", "-wal", "-shm"] {
        let path = if suffix.is_empty() {
            db_path.to_path_buf()
        } else {
            PathBuf::from(format!("{}{suffix}", db_path.to_string_lossy()))
        };
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.is_file() {
                bucket.bytes += meta.len();
                bucket.files += 1;
            }
        }
    }
    bucket
}

/// 盘上实际存在的图片文件，路径相对 `assets/` 且统一用 `/`。
fn asset_files(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                walk(&entry.path(), root, out);
            } else if kind.is_file() {
                let path = entry.path();
                if relative(root, &path).is_some() {
                    out.push(path);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

fn relative(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    Some(
        rest.components()
            .map(|part| part.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

/// 数据库里登记过的图，这些是题目内容或学生作答照片，永远不算无主。
///
/// 注意：作答照片（`local_attempt_asset`）也在这张表里登记路径——漏了它，
/// 「清理无主图片」会把学生交上来的卷子照片当垃圾删掉。
fn referenced(conn: &Connection) -> Result<HashSet<String>, String> {
    let mut out = HashSet::new();
    for sql in [
        "select relative_path from question_asset",
        "select relative_path from local_attempt_asset where deleted_at is null",
    ] {
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        for row in rows.flatten() {
            out.insert(row);
        }
    }
    Ok(out)
}

#[cfg(target_os = "macos")]
fn disk_space(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let code = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if code != 0 {
        return None;
    }
    let total = stat.f_blocks as u64 * stat.f_frsize as u64;
    let free = stat.f_bavail as u64 * stat.f_frsize as u64;
    Some((total, free))
}

#[cfg(not(target_os = "macos"))]
fn disk_space(_path: &Path) -> Option<(u64, u64)> {
    // 其他平台先不报磁盘余量，免得引入额外依赖；存储明细本身照常可用。
    None
}

pub fn usage(conn: &Connection, data_dir: &Path, db_path: &Path) -> Result<Usage, String> {
    let total = scan(data_dir);
    let assets_root = data_dir.join("assets");
    let assets = scan(&assets_root);
    let tmp = scan(&data_dir.join("tmp"));
    let backups = scan(&data_dir.join("backup"));
    let db = db_bucket(db_path);

    let counted = Bucket {
        bytes: db.bytes + assets.bytes + tmp.bytes + backups.bytes,
        files: db.files + assets.files + tmp.files + backups.files,
    };
    let other = Bucket {
        bytes: total.bytes.saturating_sub(counted.bytes),
        files: total.files.saturating_sub(counted.files),
    };

    let assets_questions: u64 = conn
        .query_row(
            "select count(distinct question_id) from question_asset",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as u64;

    let mut tmp_recent = 0;
    count_recent(&data_dir.join("tmp"), GUARD, &mut tmp_recent);
    // 队列表只用于「哪些临时图还得留着」，读不到也不该让整个面板打不开。
    let tmp_pinned = pinned_tmp(conn)
        .unwrap_or_default()
        .iter()
        .filter(|path| path.exists())
        .count() as u64;

    let refs = referenced(conn)?;
    let mut orphan_files = 0;
    let mut orphan_bytes = 0;
    for path in asset_files(&assets_root) {
        let Some(rel) = relative(&assets_root, &path) else {
            continue;
        };
        // 课件 HTML、草稿图片和导入原稿都由课件库 / 导入任务管，不归无主图片管。
        if rel.starts_with("courseware/") {
            continue;
        }
        if refs.contains(&rel) || is_recent(&path, GUARD) {
            continue;
        }
        orphan_files += 1;
        orphan_bytes += std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    }

    let (disk_total, disk_free) = match disk_space(data_dir) {
        Some((total, free)) => (Some(total), Some(free)),
        None => (None, None),
    };

    Ok(Usage {
        data_dir: data_dir.to_string_lossy().to_string(),
        db_path: db_path.to_string_lossy().to_string(),
        total,
        db,
        assets,
        tmp,
        backups,
        other,
        assets_questions,
        tmp_recent,
        tmp_pinned,
        orphan_files,
        orphan_bytes,
        disk_total,
        disk_free,
    })
}

/// 连拍队列里 PENDING / RUNNING / FAILED 的图还在 `tmp/`，清掉就得重拍。
/// DONE 的项不用管：入题时已经复制到 `assets/`，原件可以删。
pub fn pinned_tmp(conn: &Connection) -> Result<HashSet<PathBuf>, String> {
    let mut stmt = conn
        .prepare(
            "select image_path from capture_queue where status in ('PENDING','RUNNING','FAILED')",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut out = HashSet::new();
    for row in rows.flatten() {
        out.insert(PathBuf::from(row));
    }
    Ok(out)
}

/// 清 `tmp/`：截图、PDF 页面渲染、格式转换和拼图的中间产物。
/// `pinned` 是队列里还要用的图，命中就跳过。
pub fn clean_tmp(
    data_dir: &Path,
    guard: Duration,
    pinned: &HashSet<PathBuf>,
) -> Result<CleanResult, String> {
    let dir = data_dir.join("tmp");
    let mut out = CleanResult::default();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        // 目录本来就没有，等于已经清干净。
        Err(_) => return Ok(out),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if pinned.contains(&path) {
            out.pinned += 1;
            continue;
        }
        if is_recent(&path, guard) {
            out.kept += 1;
            continue;
        }
        let bytes = if kind.is_dir() {
            scan(&path).bytes
        } else {
            entry.metadata().map(|meta| meta.len()).unwrap_or(0)
        };
        let result = if kind.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match result {
            Ok(()) => {
                out.removed += 1;
                out.freed_bytes += bytes;
            }
            Err(_) => out.errors += 1,
        }
    }
    Ok(out)
}

/// 清 `assets/` 里数据库没有引用的图，顺带收掉空目录。题库内容一张不动。
pub fn clean_orphan_assets(
    conn: &Connection,
    data_dir: &Path,
    guard: Duration,
) -> Result<CleanResult, String> {
    let root = data_dir.join("assets");
    let refs = referenced(conn)?;
    let mut out = CleanResult::default();
    for path in asset_files(&root) {
        let Some(rel) = relative(&root, &path) else {
            continue;
        };
        // 课件 HTML、图片与导入原稿由课件功能自己管生命周期，一律跳过。
        if rel.starts_with("courseware/") {
            continue;
        }
        // 作答照片同理：学生交上来的卷子照片由批改台管，不当孤儿图删。
        if rel.starts_with("submissions/") {
            continue;
        }
        if refs.contains(&rel) {
            continue;
        }
        if is_recent(&path, guard) {
            out.kept += 1;
            continue;
        }
        let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                out.removed += 1;
                out.freed_bytes += bytes;
            }
            Err(_) => out.errors += 1,
        }
    }
    prune_empty(&root);
    Ok(out)
}

/// 删掉 `assets/` 下的空题目目录（图删完就只剩壳）。
fn prune_empty(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        prune_empty(&path);
        if std::fs::read_dir(&path)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(&path);
        }
    }
}

/// 备份只留最近 `keep` 份，更老的删掉；按修改时间算新旧。
pub fn clean_backups(data_dir: &Path, keep: usize) -> Result<CleanResult, String> {
    let dir = data_dir.join("backup");
    let mut out = CleanResult::default();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(out);
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file() || kind.is_symlink() {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        files.push((
            meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            meta.len(),
            path,
        ));
    }
    // 新的在前，前 keep 份留下。
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, bytes, path) in files.into_iter().skip(keep) {
        match std::fs::remove_file(&path) {
            Ok(()) => {
                out.removed += 1;
                out.freed_bytes += bytes;
            }
            Err(_) => out.errors += 1,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nb-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("tmp")).unwrap();
        std::fs::create_dir_all(dir.join("assets").join("q1")).unwrap();
        std::fs::create_dir_all(dir.join("backup")).unwrap();
        dir
    }

    #[test]
    fn tmp_clean_respects_guard_window() {
        let dir = temp_root();
        std::fs::write(dir.join("tmp").join("a.png"), b"12345").unwrap();
        std::fs::write(dir.join("tmp").join("b.png"), b"1234567890").unwrap();

        // 保护窗口内一个都不删。
        let kept = clean_tmp(&dir, GUARD, &HashSet::new()).unwrap();
        assert_eq!(kept.removed, 0);
        assert_eq!(kept.kept, 2);

        // 窗口放到 0，才真的清掉，并报出释放字节。
        let freed = clean_tmp(&dir, Duration::ZERO, &HashSet::new()).unwrap();
        assert_eq!(freed.removed, 2);
        assert_eq!(freed.freed_bytes, 15);
        assert!(!dir.join("tmp").join("a.png").exists());
    }

    #[test]
    fn queue_images_are_pinned_even_without_guard() {
        let dir = temp_root();
        let queued = dir.join("tmp").join("capture-1.png");
        let spare = dir.join("tmp").join("capture-2.png");
        std::fs::write(&queued, b"queued").unwrap();
        std::fs::write(&spare, b"spare").unwrap();

        let pinned: HashSet<PathBuf> = [queued.clone()].into_iter().collect();
        let result = clean_tmp(&dir, Duration::ZERO, &pinned).unwrap();
        assert_eq!(result.removed, 1);
        assert_eq!(result.pinned, 1);
        assert!(queued.exists(), "队列里等识别的图不能删");
        assert!(!spare.exists());
    }

    #[test]
    fn assets_are_not_touched_by_tmp_clean() {
        let dir = temp_root();
        let figure = dir.join("assets").join("q1").join("figure.png");
        std::fs::write(&figure, b"figure").unwrap();
        clean_tmp(&dir, Duration::ZERO, &HashSet::new()).unwrap();
        assert!(figure.exists(), "题目制图不能跟着临时文件一起没");
    }

    #[test]
    fn orphan_clean_keeps_referenced_and_prunes_empty_dir() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "create table question_asset (id text primary key, question_id text, relative_path text);
             create table local_attempt_asset (id text primary key, relative_path text, deleted_at text);",
        )
        .unwrap();
        conn.execute(
            "insert into question_asset (id, question_id, relative_path) values ('a1','q1','q1/figure.png')",
            [],
        )
        .unwrap();

        let dir = temp_root();
        let kept = dir.join("assets").join("q1").join("figure.png");
        let orphan = dir.join("assets").join("q9").join("leftover.png");
        std::fs::create_dir_all(orphan.parent().unwrap()).unwrap();
        std::fs::write(&kept, b"keep").unwrap();
        std::fs::write(&orphan, b"orphan").unwrap();

        let result = clean_orphan_assets(&conn, &dir, Duration::ZERO).unwrap();
        assert_eq!(result.removed, 1);
        assert_eq!(result.freed_bytes, 6);
        assert!(kept.exists(), "被题目引用的图必须留着");
        assert!(!orphan.exists());
        assert!(!dir.join("assets").join("q9").exists(), "空目录要收掉");
        assert!(dir.join("assets").join("q1").exists());
    }

    #[test]
    fn orphan_clean_keeps_submission_photos() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "create table question_asset (id text primary key, question_id text, relative_path text);
             create table local_attempt_asset (id text primary key, relative_path text, deleted_at text);",
        )
        .unwrap();
        conn.execute(
            "insert into local_attempt_asset (id, relative_path, deleted_at) values ('s1','submissions/stu1/p1.png',null)",
            [],
        )
        .unwrap();
        let dir = temp_root();
        let photo = dir
            .join("assets")
            .join("submissions")
            .join("stu1")
            .join("p1.png");
        std::fs::create_dir_all(photo.parent().unwrap()).unwrap();
        std::fs::write(&photo, b"paper").unwrap();

        let result = clean_orphan_assets(&conn, &dir, Duration::ZERO).unwrap();
        assert_eq!(result.removed, 0);
        assert!(photo.exists(), "学生交上来的卷子照片不能被当孤儿删");
    }

    #[test]
    fn orphan_clean_keeps_courseware_images_and_import_originals() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "create table question_asset (id text primary key, question_id text, relative_path text);
             create table local_attempt_asset (id text primary key, relative_path text, deleted_at text);
             create table capture_queue (id text primary key, image_path text, status text);",
        )
        .unwrap();
        let dir = temp_root();
        let files = [
            "courseware/drafts/draft-1/图1.png",
            "courseware/imports/task-1/原稿.docx",
            "courseware/courseware-1.html",
        ];
        for relative in files {
            let path = dir.join("assets").join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"courseware").unwrap();
        }

        let report = usage(&conn, &dir, &dir.join("neuralkb.db")).unwrap();
        assert_eq!(report.orphan_files, 0, "课件资产不能计入无主图片");
        let result = clean_orphan_assets(&conn, &dir, Duration::ZERO).unwrap();
        assert_eq!(result.removed, 0);
        for relative in files {
            assert!(
                dir.join("assets").join(relative).exists(),
                "{relative} 必须保留"
            );
        }
    }

    #[test]
    fn backups_keep_newest_three() {
        let dir = temp_root();
        for index in 0..5 {
            std::fs::write(dir.join("backup").join(format!("b{index}.zip")), b"x").unwrap();
        }
        let result = clean_backups(&dir, 3).unwrap();
        assert_eq!(result.removed, 2);
        assert_eq!(std::fs::read_dir(dir.join("backup")).unwrap().count(), 3);
    }

    #[test]
    fn usage_splits_buckets_and_never_counts_outside_dir() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "create table question_asset (id text primary key, question_id text, relative_path text);
             create table local_attempt_asset (id text primary key, relative_path text, deleted_at text);
             create table capture_queue (id text primary key, image_path text, status text);",
        )
        .unwrap();
        let dir = temp_root();
        std::fs::write(dir.join("neuralkb.db"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("assets").join("q1").join("a.png"), vec![0u8; 40]).unwrap();
        std::fs::write(dir.join("tmp").join("t.png"), vec![0u8; 30]).unwrap();
        std::fs::write(dir.join("backup").join("b.zip"), vec![0u8; 20]).unwrap();
        std::fs::write(dir.join("stray.txt"), vec![0u8; 5]).unwrap();

        let report = usage(&conn, &dir, &dir.join("neuralkb.db")).unwrap();
        assert_eq!(report.db.bytes, 100);
        assert_eq!(report.assets.bytes, 40);
        assert_eq!(report.tmp.bytes, 30);
        assert_eq!(report.backups.bytes, 20);
        assert_eq!(report.other.bytes, 5);
        assert_eq!(report.total.bytes, 195);
        // 临时文件全是刚写的，属于受保护范围。
        assert_eq!(report.tmp_recent, 1);
    }
}
