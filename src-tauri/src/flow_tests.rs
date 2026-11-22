//! 端到端流程测试：不开窗口，直接把「冷启动 → 导入题目包 → 整批撤销 → 手工录题 → 检索」
//! 在临时目录里走一遍。命令层（`commands.rs`）只是这些函数的薄壳，验完这层，
//! README「手工验收清单」里第 1、4、7 条的数据链路就有了回归保护。

use std::path::PathBuf;

use rusqlite::Connection;

use crate::models::{QuestionInput, QuestionQuery};
use crate::{auth, db, import, repo};

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nb-flow-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("assets")).expect("建临时数据目录");
    dir
}

/// 模拟一次冷启动：建库、迁移、灌教材快照、准备检索表。
fn fresh() -> (Connection, PathBuf) {
    let dir = temp_dir();
    let mut conn = db::open(&dir.join("neuralkb.db")).expect("打开数据库");
    db::migrate(&mut conn).expect("执行迁移");
    db::seed_curriculum(&mut conn).expect("灌教材快照");
    db::ensure_search(&conn).expect("准备检索");
    (conn, dir)
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap_or(0)
}

fn first_knowledge_id(conn: &Connection) -> String {
    conn.query_row(
        "select id from knowledge_point order by subject_id, sort limit 1",
        [],
        |row| row.get(0),
    )
    .expect("种子教材里有知识点")
}

fn first_subject_id(conn: &Connection) -> String {
    conn.query_row("select id from subject order by sort limit 1", [], |row| {
        row.get(0)
    })
    .expect("种子教材里有学科")
}

fn write_test_png(path: &std::path::Path) {
    std::fs::write(path, include_bytes!("../icons/32x32.png")).expect("写测试原图");
}

fn insert_done_capture_job(
    conn: &Connection,
    id: &str,
    kind: &str,
    input: serde_json::Value,
    items: Vec<serde_json::Value>,
) {
    let result = serde_json::json!({"items": items});
    conn.execute(
        "insert into generation_job (id, kind, status, input_json, result_json, progress, created_at, updated_at)
         values (?1, ?2, 'DONE', ?3, ?4, 100, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        rusqlite::params![id, kind, input.to_string(), result.to_string()],
    )
    .expect("插入识别任务");
}

fn capture_draft_item(
    conn: &Connection,
    stem: &str,
    capture_path: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "type": "SOLVED",
        "stem": stem,
        "options": [],
        "answer": "略",
        "analysis": "略",
        "difficulty": 3,
        "knowledgeNames": [],
        "knowledgeIds": [],
        "subjectId": first_subject_id(conn),
        "warnings": [],
        "blocking": [],
        "issues": [],
        "source": null,
        "year": null,
        "capturePath": capture_path,
    })
}

fn case_pack(keyword: &str) -> String {
    let raw = include_str!("../../shared/question-format/cases.json");
    let value: serde_json::Value = serde_json::from_str(raw).expect("cases.json 是合法 JSON");
    let case = value["cases"]
        .as_array()
        .expect("cases 是数组")
        .iter()
        .find(|case| case["name"].as_str().unwrap_or("").contains(keyword))
        .unwrap_or_else(|| panic!("cases.json 里找不到含「{keyword}」的用例"));
    serde_json::to_string(&case["pack"]).expect("序列化用例包")
}

#[test]
fn local_account_signup_login_and_password_change() {
    let (conn, _dir) = fresh();

    // 冷启动没有账号。
    let s = auth::status(&conn, false).expect("读状态");
    assert!(!s.has_user);

    // 首次设置账号，立刻解锁；密码从不落到明文列。
    let created = auth::signup(&conn, "小明", "secret123", "").expect("设置账号");
    assert!(created.has_user);
    assert_eq!(created.username, "小明");
    assert_eq!(created.display_name, "小明");
    let hash: String = conn
        .query_row(
            "select password_hash from local_user where id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!hash.contains("secret123"));
    assert!(hash.starts_with("$argon2"));

    // 不能再设置第二个账号。
    assert!(auth::signup(&conn, "别人", "other123", "").is_err());

    // 错误密码登录失败，会话不解锁。
    let mut unlocked = false;
    assert!(auth::login(&conn, &mut unlocked, "小明", "wrongpass").is_err());
    assert!(!unlocked);

    // 正确密码登录成功。
    assert!(auth::login(&conn, &mut unlocked, "小明", "secret123").is_ok());
    assert!(unlocked);

    // 改密后旧密码失效、新密码可用。
    auth::change_password(&conn, &mut unlocked, "secret123", "newpass456").expect("改密");
    let mut fresh_session = false;
    assert!(auth::login(&conn, &mut fresh_session, "小明", "secret123").is_err());
    assert!(auth::login(&conn, &mut fresh_session, "小明", "newpass456").is_ok());
    assert!(fresh_session);
}

#[test]
fn cold_start_seeds_curriculum() {
    let (conn, _dir) = fresh();
    assert_eq!(count(&conn, "select count(*) from subject"), 9, "学科快照");
    assert!(
        count(&conn, "select count(*) from knowledge_point") >= 100,
        "知识点快照应灌入"
    );
    assert_eq!(
        count(&conn, "select count(*) from question"),
        0,
        "新库没有题目"
    );
    let version: i64 = conn
        .query_row("pragma user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        version,
        db::SCHEMA_VERSION,
        "迁移版本（含本地用户表、采集台、教学闭环、学情、课件、拍照批改、模型 Key 标记、截图附件清理、知识点统一、中心照片、试卷、学号与成绩、课件批量导入、制图用途路由）"
    );
}

#[test]
fn import_accepts_compact_options_then_undo_goes_to_trash() {
    let (mut conn, dir) = fresh();
    // 旧包只带 examPoints：那些名字当知识点解析。先造两条同名知识点验证能落库。
    let sid = first_subject_id(&conn);
    for (i, name) in ["质数", "数的整除"].iter().enumerate() {
        conn.execute(
            "insert or ignore into knowledge_point (id, subject_id, name, sort) values (?1, ?2, ?3, ?4)",
            rusqlite::params![format!("kp-legacy-{i}"), sid, name, i as i64],
        )
        .expect("造旧包知识点");
    }
    let pack = case_pack("紧凑选项");

    // 1. 预览：只有一道题，且判定为可导入。
    let preview = import::scan_text(&pack, &dir, &[]).expect("解析用例包");
    assert_eq!(preview.total, 1, "用例包里只有一题");
    assert_eq!(preview.ready, 1, "应可直接导入：{:?}", preview.items);
    let draft = &preview.items[0].draft;
    assert_eq!(draft.kind, "SINGLE", "中文题型「单选题」要归一");
    assert_eq!(draft.options.len(), 3, "A|4;B|5;C|6 要拆成三个选项");
    assert_eq!(draft.answer, "B");
    assert_eq!(
        draft.knowledge_paths.len(),
        2,
        "「质数;数的整除」要拆成两条知识点名"
    );

    // 2. 入库：题干、选项、知识点都要落库。
    let rows: Vec<usize> = preview
        .items
        .iter()
        .filter(|item| item.status == "READY")
        .map(|item| item.row_no)
        .collect();
    let result = import::commit(&mut conn, &dir, &preview, &rows, false).expect("导入入库");
    assert_eq!(result.imported, 1, "导入 1 题");
    assert_eq!(result.failed, 0);
    assert_eq!(
        count(
            &conn,
            "select count(*) from question where deleted_at is null"
        ),
        1
    );

    let question_id = result.question_ids[0].clone();
    let saved = repo::get(&conn, &question_id).expect("读回刚导入的题");
    assert_eq!(saved.options.len(), 3);
    assert_eq!(saved.knowledge.len(), 2, "旧包里的知识点名要落库");
    assert!(saved.stem.contains("质数"));
    assert_eq!(saved.origin, "IMPORT");

    // 3. 同一份包再导一次：题干指纹相同，应判重。
    let again = import::scan_text(&pack, &dir, &[]).expect("再解析一次");
    let existing = repo::fingerprints(&conn).expect("取指纹");
    let dup = import::scan_text(&pack, &dir, &existing).expect("带已有指纹再解析");
    assert_eq!(dup.duplicate, 1, "同题干应识别为重复：{:?}", dup.items);
    let _ = again;

    // 4. 撤销整批：题目进回收站，题库里消失，批次标记 UNDONE。
    let undone = import::undo(&mut conn, &result.batch_id).expect("撤销批次");
    assert_eq!(undone, 1, "撤销 1 题");
    assert_eq!(
        count(
            &conn,
            "select count(*) from question where deleted_at is null"
        ),
        0
    );
    assert_eq!(
        count(
            &conn,
            "select count(*) from question where deleted_at is not null"
        ),
        1
    );
    let status: String = conn
        .query_row(
            "select status from import_batch where id = ?1",
            [&result.batch_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "UNDONE");

    // 5. 回收站仍然能查到这道题，恢复后回到题库。
    let trashed = repo::list(
        &conn,
        &QuestionQuery {
            keyword: None,
            subject_id: None,
            kind: None,
            difficulty: None,
            knowledge_id: None,
            tag: None,
            source: None,
            needs_review: None,
            rejected: None,
            trashed: Some(true),
            uncategorized: None,
            sort: None,
            page: Some(1),
            size: Some(30),
        },
    )
    .expect("查回收站");
    assert_eq!(trashed.total, 1);
    repo::restore(&mut conn, &[question_id.clone()]).expect("恢复");
    assert_eq!(
        count(
            &conn,
            "select count(*) from question where deleted_at is null"
        ),
        1
    );
}

#[test]
fn duplicate_and_similar_rows_import_only_when_explicitly_allowed() {
    let (mut conn, dir) = fresh();
    let pack = case_pack("紧凑选项");
    let first = import::scan_text(&pack, &dir, &[]).expect("首次预览");
    let rows = [first.items[0].row_no];
    import::commit(&mut conn, &dir, &first, &rows, false).expect("首次导入");

    let existing = repo::fingerprints(&conn).expect("取指纹");
    let duplicate = import::scan_text(&pack, &dir, &existing).expect("重复题预览");
    assert_eq!(duplicate.items[0].status, "DUPLICATE");
    let rows = [duplicate.items[0].row_no];
    let skipped =
        import::commit(&mut conn, &dir, &duplicate, &rows, false).expect("默认跳过重复题");
    assert_eq!((skipped.imported, skipped.skipped), (0, 1));
    let forced = import::commit(&mut conn, &dir, &duplicate, &rows, true).expect("显式导入重复题");
    assert_eq!(forced.imported, 1);

    let mut value: serde_json::Value = serde_json::from_str(&pack).expect("解析原包");
    value["questions"][0]["stem"] = serde_json::json!("下列哪一个是质数呢？");
    let similar_text = serde_json::to_string(&value).expect("序列化相似题包");
    let existing = repo::fingerprints(&conn).expect("重新取指纹");
    let similar = import::scan_text(&similar_text, &dir, &existing).expect("相似题预览");
    assert_eq!(similar.items[0].status, "SIMILAR", "{:?}", similar.items);
    let rows = [similar.items[0].row_no];
    let skipped = import::commit(&mut conn, &dir, &similar, &rows, false).expect("默认跳过相似题");
    assert_eq!((skipped.imported, skipped.skipped), (0, 1));
    let forced = import::commit(&mut conn, &dir, &similar, &rows, true).expect("显式导入相似题");
    assert_eq!(forced.imported, 1);
    assert_eq!(
        count(
            &conn,
            "select count(*) from question where deleted_at is null"
        ),
        3
    );
}

#[test]
fn manual_question_is_searchable_offline() {
    let (mut conn, _dir) = fresh();
    let knowledge_id = first_knowledge_id(&conn);
    let id = repo::save(
        &mut conn,
        &QuestionInput {
            id: None,
            subject_id: None,
            kind: "解答".to_string(),
            stem: "若 $a=2$，求 $a\\times 3$ 的值。".to_string(),
            options: vec![],
            answer: "$6$".to_string(),
            analysis: "$2\\times 3=6$。".to_string(),
            difficulty: 2,
            tags: vec!["七年级".to_string()],
            knowledge_ids: vec![knowledge_id],
            source: Some("手工录入".to_string()),
            year: None,
            status: Some("PUBLISHED".to_string()),
            origin: Some("MANUAL".to_string()),
            needs_review: Some(false),
            review_note: None,
        },
    )
    .expect("保存手工题");

    // 题干里的单反斜杠要原样保留，不能被当成控制字符吃掉。
    let saved = repo::get(&conn, &id).expect("读回");
    assert!(
        saved.stem.contains("\\times"),
        "LaTeX 反斜杠被改写：{}",
        saved.stem
    );

    // 中英文混合短语（≥3 字）走 FTS，断网也在本机查得到。
    for keyword in ["七年级", "求", "的值"] {
        let page = repo::list(
            &conn,
            &QuestionQuery {
                keyword: Some(keyword.to_string()),
                subject_id: None,
                kind: None,
                difficulty: None,
                knowledge_id: None,
                tag: None,
                source: None,
                needs_review: None,
                rejected: None,
                trashed: None,
                uncategorized: None,
                sort: None,
                page: Some(1),
                size: Some(30),
            },
        )
        .expect("检索");
        assert!(
            page.items.iter().any(|item| item.id == id),
            "「{keyword}」没搜到刚存的题"
        );
    }

    let stats = repo::stats(&conn).expect("统计");
    assert_eq!(stats.total, 1);
    assert_eq!(stats.draft, 0);
}

fn reviewable_question(conn: &mut Connection) -> String {
    let knowledge_id = first_knowledge_id(conn);
    repo::save(
        conn,
        &QuestionInput {
            id: None,
            subject_id: None,
            kind: "解答".to_string(),
            stem: "待复核的题：求 $1+1$。".to_string(),
            options: vec![],
            answer: "$2$".to_string(),
            analysis: "$1+1=2$。".to_string(),
            difficulty: 3,
            tags: vec![],
            knowledge_ids: vec![knowledge_id],
            source: None,
            year: None,
            status: Some("DRAFT".to_string()),
            origin: Some("AI".to_string()),
            needs_review: Some(true),
            review_note: Some("AI 生成，待核对".to_string()),
        },
    )
    .expect("保存待复核题")
}

fn complete_choice_input(knowledge_id: &str, status: &str) -> QuestionInput {
    QuestionInput {
        id: None,
        subject_id: None,
        kind: "SINGLE".to_string(),
        stem: "正式入库门槛测试：1+1 等于几？".to_string(),
        options: vec![
            crate::models::OptionItem {
                key: "A".into(),
                text: "1".into(),
            },
            crate::models::OptionItem {
                key: "B".into(),
                text: "2".into(),
            },
        ],
        answer: "B".to_string(),
        analysis: "因为 $1+1=2$。".to_string(),
        difficulty: 1,
        tags: vec![],
        knowledge_ids: vec![knowledge_id.to_string()],
        source: None,
        year: None,
        status: Some(status.to_string()),
        origin: Some("MANUAL".to_string()),
        needs_review: Some(status == "DRAFT"),
        review_note: None,
    }
}

fn remove_required_field(input: &mut QuestionInput, field: &str) {
    match field {
        "answer" => input.answer.clear(),
        "analysis" => input.analysis.clear(),
        "knowledge" => input.knowledge_ids.clear(),
        _ => unreachable!(),
    }
}

#[test]
fn review_approve_publishes_and_clears_flag() {
    let (mut conn, _dir) = fresh();
    let id = reviewable_question(&mut conn);

    let changed = repo::review(&mut conn, &[id.clone()], "approve", None).expect("通过复核");
    assert_eq!(changed, 1);

    let after = repo::get(&conn, &id).expect("读回");
    assert_eq!(after.status, "PUBLISHED", "通过后应标记已发布");
    assert!(!after.needs_review, "通过后应移出待复核");
    assert!(after.review_note.is_none(), "通过后应清掉复核备注");

    let stats = repo::stats(&conn).expect("统计");
    assert_eq!(stats.needs_review, 0, "待复核计数应归零");
}

#[test]
fn direct_publish_and_review_each_require_all_three_fields() {
    let (mut conn, _dir) = fresh();
    let knowledge_id = first_knowledge_id(&conn);

    for field in ["answer", "analysis", "knowledge"] {
        let mut input = complete_choice_input(&knowledge_id, "PUBLISHED");
        remove_required_field(&mut input, field);
        assert!(
            repo::save(&mut conn, &input).is_err(),
            "直接发布缺 {field} 应失败"
        );

        let mut input = complete_choice_input(&knowledge_id, "DRAFT");
        remove_required_field(&mut input, field);
        let id = repo::save(&mut conn, &input).expect("缺项仍可存草稿");
        let error = repo::review(&mut conn, &[id.clone()], "approve", None)
            .expect_err(&format!("复核缺 {field} 应失败"));
        assert!(
            error.contains(&repo::short_id(&id)),
            "错误应指出具体题目（短编号）：{error}"
        );
        assert_eq!(repo::get(&conn, &id).expect("读回").status, "DRAFT");
    }

    let mut draft = complete_choice_input(&knowledge_id, "DRAFT");
    draft.answer.clear();
    draft.analysis.clear();
    draft.knowledge_ids.clear();
    let id = repo::save(&mut conn, &draft).expect("三项都缺也可存草稿");
    assert!(repo::get(&conn, &id).expect("读回草稿").needs_review);

    let published = repo::save(
        &mut conn,
        &complete_choice_input(&knowledge_id, "PUBLISHED"),
    )
    .expect("三项齐全可直接发布");
    assert_eq!(
        repo::get(&conn, &published).expect("读回正式题").status,
        "PUBLISHED"
    );
}

#[test]
fn review_batch_rolls_back_when_any_question_is_incomplete() {
    let (mut conn, _dir) = fresh();
    let knowledge_id = first_knowledge_id(&conn);
    let ready =
        repo::save(&mut conn, &complete_choice_input(&knowledge_id, "DRAFT")).expect("完整草稿");
    let mut incomplete = complete_choice_input(&knowledge_id, "DRAFT");
    incomplete.analysis.clear();
    let incomplete = repo::save(&mut conn, &incomplete).expect("不完整草稿");

    let error = repo::review(
        &mut conn,
        &[ready.clone(), incomplete.clone()],
        "approve",
        None,
    )
    .expect_err("任一题缺项，整批不发布");
    assert!(error.contains(&repo::short_id(&incomplete)));
    assert_eq!(
        repo::get(&conn, &ready).expect("读回完整草稿").status,
        "DRAFT"
    );
}

#[test]
fn rejected_question_goes_to_rejected_list_and_can_return_to_draft() {
    let (mut conn, _dir) = fresh();
    let knowledge_id = first_knowledge_id(&conn);
    let id = repo::save(&mut conn, &complete_choice_input(&knowledge_id, "DRAFT")).expect("草稿");
    repo::review(&mut conn, &[id.clone()], "reject", Some("解析太简略")).expect("驳回");

    let rejected_query = QuestionQuery {
        keyword: None,
        subject_id: None,
        kind: None,
        difficulty: None,
        knowledge_id: None,
        tag: None,
        source: None,
        needs_review: None,
        rejected: Some(true),
        trashed: None,
        uncategorized: None,
        sort: None,
        page: Some(1),
        size: Some(30),
    };
    let page = repo::list(&conn, &rejected_query).expect("驳回列表");
    assert_eq!(page.items.len(), 1, "驳回的题应进驳回列表");
    assert_eq!(page.items[0].id, id);
    assert_eq!(page.items[0].review_note.as_deref(), Some("解析太简略"));

    repo::review(&mut conn, &[id.clone()], "draft", None).expect("转回草稿");
    assert!(
        repo::list(&conn, &rejected_query)
            .expect("驳回列表")
            .items
            .is_empty(),
        "转回草稿后应离开驳回列表"
    );
    let back = repo::get(&conn, &id).expect("读回");
    assert!(back.needs_review, "转回草稿后要回到草稿箱");
    assert!(back.review_note.is_none(), "转回草稿要清掉驳回原因");
}

#[test]
fn published_choice_answer_must_match_actual_options() {
    let (mut conn, _dir) = fresh();
    let knowledge_id = first_knowledge_id(&conn);
    for (kind, answer) in [("SINGLE", "C"), ("SINGLE", "AB"), ("MULTI", "AC")] {
        let mut input = complete_choice_input(&knowledge_id, "PUBLISHED");
        input.kind = kind.to_string();
        input.answer = answer.to_string();
        assert!(
            repo::save(&mut conn, &input).is_err(),
            "{kind} 的答案 {answer} 应无效"
        );
    }

    let mut multi = complete_choice_input(&knowledge_id, "PUBLISHED");
    multi.kind = "MULTI".into();
    multi.answer = "A、B".into();
    repo::save(&mut conn, &multi).expect("多选答案分隔符可用");

    let mut draft = complete_choice_input(&knowledge_id, "DRAFT");
    draft.answer = "C".into();
    repo::save(&mut conn, &draft).expect("无效答案可留待草稿修正");
}

#[test]
fn review_reject_returns_to_draft_with_reason() {
    let (mut conn, _dir) = fresh();
    let id = reviewable_question(&mut conn);

    // 驳回必须有原因。
    assert!(repo::review(&mut conn, &[id.clone()], "reject", None).is_err());
    assert!(repo::review(&mut conn, &[id.clone()], "reject", Some("   ")).is_err());

    let changed =
        repo::review(&mut conn, &[id.clone()], "reject", Some("答案与解析不一致")).expect("驳回");
    assert_eq!(changed, 1);

    let after = repo::get(&conn, &id).expect("读回");
    assert_eq!(after.status, "DRAFT", "驳回后应退回草稿");
    assert!(!after.needs_review, "驳回后应移出待复核");
    assert_eq!(after.review_note.as_deref(), Some("答案与解析不一致"));
}

#[test]
fn review_skips_trashed_questions() {
    let (mut conn, _dir) = fresh();
    let id = reviewable_question(&mut conn);
    repo::soft_delete(&mut conn, &[id.clone()]).expect("移到回收站");

    let changed = repo::review(&mut conn, &[id.clone()], "approve", None).expect("复核");
    assert_eq!(changed, 0, "回收站的题不应被复核");
}

#[test]
fn backup_zip_contains_db_and_assets() {
    let (_conn, dir) = fresh();
    // 造一张图片附件，模拟有道题带图。
    let asset_dir = dir.join("assets").join("question-1");
    std::fs::create_dir_all(&asset_dir).unwrap();
    std::fs::write(asset_dir.join("1.png"), b"not-a-real-png-but-a-file").unwrap();

    let target = crate::commands::write_backup(&dir).expect("导出备份");
    assert!(target.exists(), "备份文件应落在 backup/ 下");
    assert_eq!(target.parent().unwrap().file_name().unwrap(), "backup");

    let archive = std::fs::File::open(&target).unwrap();
    let mut zip = zip::ZipArchive::new(archive).expect("备份是合法 ZIP");
    let names: Vec<String> = (0..zip.len())
        .map(|index| zip.by_index(index).unwrap().name().to_string())
        .collect();

    assert!(
        names.iter().any(|name| name == "neuralkb.db"),
        "缺数据库：{names:?}"
    );
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("assets/question-1/")),
        "缺图片附件：{names:?}"
    );
    assert!(
        !names.iter().any(|name| name.starts_with("backup/")),
        "备份不该把自己打进去：{names:?}"
    );
    assert!(
        !names
            .iter()
            .any(|name| name.ends_with("-wal") || name.ends_with("-shm")),
        "WAL 影子文件不该进包：{names:?}"
    );
}

fn write_zip(path: &std::path::Path, files: &[(&str, &[u8])]) {
    use std::io::Write;
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in files {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn read_zip(path: &std::path::Path) -> (Vec<String>, serde_json::Value) {
    let archive = std::fs::File::open(path).unwrap();
    let mut zip = zip::ZipArchive::new(archive).expect("是合法 ZIP");
    let names: Vec<String> = (0..zip.len())
        .map(|index| zip.by_index(index).unwrap().name().to_string())
        .collect();
    let raw = {
        use std::io::Read;
        let mut entry = zip
            .by_name("questions.json")
            .expect("包里应有 questions.json");
        let mut buffer = String::new();
        entry.read_to_string(&mut buffer).unwrap();
        buffer
    };
    (
        names,
        serde_json::from_str(&raw).expect("questions.json 是合法 JSON"),
    )
}

#[test]
fn zip_export_round_trips_images_and_knowledge() {
    // 1. 先造一个「带图 + 两个知识点」的包并导进来，得到真实的题库数据。
    let (mut conn, dir) = fresh();
    let png: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";
    let pack = r#"{"format":"neural-question-pack","version":1,"subject":"数学","questions":[{"type":"SOLVED","stem":"如图，求阴影面积。![](images/fig-1.png)","answer":"$12$","analysis":"矩形减圆。","difficulty":3,"knowledgePath":["初中数学 · 七年级上册","第一章 有理数","正数和负数"],"knowledgePaths":[["初中数学 · 七年级上册","第一章 有理数","正数和负数"],["初中数学 · 七年级上册","第一章 有理数","有理数的乘方"]],"images":[{"file":"images/fig-1.png","kind":"FIGURE","alt":"图1"}]}]}"#;
    let source = dir.join("in.zip");
    write_zip(
        &source,
        &[
            ("questions.json", pack.as_bytes()),
            ("images/fig-1.png", png),
        ],
    );

    let preview = import::scan(&source, &dir, &[]).expect("扫描带图包");
    assert_eq!(preview.ready, 1, "{:?}", preview.items);
    assert!(preview.items[0].images[0].exists, "包里的图应能找到");
    let rows: Vec<usize> = preview.items.iter().map(|item| item.row_no).collect();
    let result = import::commit(&mut conn, &dir, &preview, &rows, false).expect("导入");
    let id = result.question_ids[0].clone();

    let saved = repo::get(&conn, &id).expect("读回");
    assert_eq!(saved.knowledge.len(), 2, "两条 knowledgePaths 都要落库");
    assert_eq!(saved.assets.len(), 1);
    assert!(saved.stem.contains("nbasset:"), "库里题干用 nbasset 引用图");

    // 2. 导出带图 ZIP：图改名到 images/，题干同步改写，知识点全部带出。
    let out = dir.join("roundtrip.zip");
    let exported =
        crate::export::export_zip(&conn, &dir, std::slice::from_ref(&id), &out).expect("导出 ZIP");
    assert_eq!(exported.count, 1);
    assert_eq!(exported.images, 1, "应带出 1 张图");
    assert_eq!(exported.missing, 0);
    assert!(exported.bytes > 0);

    let (names, body) = read_zip(&out);
    assert!(names.iter().any(|name| name == "questions.json"));
    assert!(
        names.iter().any(|name| name == "images/1-1.png"),
        "图片应改名进包：{names:?}"
    );
    let question = &body["questions"][0];
    assert_eq!(
        question["stem"], "如图，求阴影面积。![图1](images/1-1.png)",
        "题干里的图引用要改写成包内路径，图注不能丢"
    );
    assert!(!question["stem"].as_str().unwrap().contains("nbasset:"));
    assert_eq!(question["images"][0]["kind"], "FIGURE");
    assert_eq!(question["images"][0]["alt"], "图1");
    assert_eq!(question["knowledgePaths"].as_array().unwrap().len(), 2);

    // 3. 导出的包原样再导入：图还在、两个知识点还在（不比原题少）。
    let again = import::scan(&out, &dir, &[]).expect("再导入自己导出的包");
    assert_eq!(again.ready, 1, "{:?}", again.items);
    assert!(again.items[0].images[0].exists, "回导时图片应能定位");
    let rows: Vec<usize> = again.items.iter().map(|item| item.row_no).collect();
    let second = import::commit(&mut conn, &dir, &again, &rows, false).expect("回导入库");
    let copy = repo::get(&conn, &second.question_ids[0]).expect("读回回导的题");
    assert_eq!(copy.knowledge.len(), 2, "知识点不能丢");
    assert_eq!(copy.assets.len(), 1, "图片不能丢");
    // 包里的 clientId 就是原题 id：回导是「覆盖」而不是新增一份。
    assert_eq!(second.question_ids[0], id, "同 id 覆盖");
    assert_eq!(
        count(
            &conn,
            "select count(*) from question where deleted_at is null"
        ),
        1,
        "回导不该多出一道题"
    );

    // 4. 单 JSON 导出：图带不走，占位成「【图】」并写 missingImages。
    let json_text = crate::export::pack_json(&conn, std::slice::from_ref(&id)).expect("导出 JSON");
    let parsed: serde_json::Value = serde_json::from_str(&json_text).unwrap();
    let item = &parsed["questions"][0];
    assert_eq!(
        item["stem"], "如图，求阴影面积。【图】",
        "JSON 里整段换成占位符，别留半截 markdown"
    );
    assert_eq!(item["missingImages"][0], "图1");
    assert!(
        item.get("images").is_none(),
        "JSON 里不该出现指向不存在文件的 images"
    );
}

#[test]
fn option_images_survive_import_and_export() {
    let (mut conn, dir) = fresh();
    let png: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";
    // 选项本身就是一张图：题干没有图，图片只出现在 options[0].text 里。
    // images 里写文件名、选项里写 images/ 前缀：按原名硬匹配会漏掉，必须按文件名兜底。
    let pack = r#"{"format":"neural-question-pack","version":1,"subject":"数学","questions":[{"type":"SINGLE","stem":"下面哪个图形是圆？","options":[{"key":"A","text":"![圆](images/opt-a.png)"},{"key":"B","text":"三角形"}],"answer":"A","analysis":"A 是圆。","difficulty":3,"knowledgePath":["初中数学 · 七年级上册","第一章 有理数","正数和负数"],"images":[{"file":"opt-a.png","kind":"FIGURE","alt":"圆"}]}]}"#;
    let source = dir.join("option.zip");
    write_zip(
        &source,
        &[
            ("questions.json", pack.as_bytes()),
            ("images/opt-a.png", png),
        ],
    );

    let preview = import::scan(&source, &dir, &[]).expect("扫描选项带图包");
    assert_eq!(preview.ready, 1, "{:?}", preview.items);
    assert!(
        !preview.items[0].images.is_empty(),
        "选项里的图也要进附件表：{:?}",
        preview.items[0]
    );
    let rows: Vec<usize> = preview.items.iter().map(|item| item.row_no).collect();
    let result = import::commit(&mut conn, &dir, &preview, &rows, false).expect("导入");
    let id = result.question_ids[0].clone();

    let saved = repo::get(&conn, &id).expect("读回");
    assert_eq!(saved.assets.len(), 1, "选项图应落成附件");
    assert!(
        saved.options[0].text.contains("nbasset:"),
        "选项里的图引用要改写成 nbasset：{}",
        saved.options[0].text
    );

    // 带图 ZIP 导出：选项里的 nbasset 也要改回包内路径，并在 images 里登记。
    let out = dir.join("option-roundtrip.zip");
    crate::export::export_zip(&conn, &dir, std::slice::from_ref(&id), &out).expect("导出 ZIP");
    let (names, body) = read_zip(&out);
    assert!(
        names.iter().any(|name| name == "images/1-1.png"),
        "{names:?}"
    );
    let option_text = body["questions"][0]["options"][0]["text"]
        .as_str()
        .unwrap_or("");
    assert_eq!(option_text, "![圆](images/1-1.png)");
    assert_eq!(body["questions"][0]["images"][0]["alt"], "圆");

    // 回导：选项图仍然是附件，不是一段指向不存在路径的 markdown。
    let again = import::scan(&out, &dir, &[]).expect("回导选项图包");
    let rows: Vec<usize> = again.items.iter().map(|item| item.row_no).collect();
    let second = import::commit(&mut conn, &dir, &again, &rows, false).expect("回导");
    let copy = repo::get(&conn, &second.question_ids[0]).expect("读回回导题");
    assert_eq!(copy.assets.len(), 1);
    assert!(copy.options[0].text.contains("nbasset:"));

    // 单 JSON 导出：选项图换成占位符，图注进 missingImages。
    let json_text = crate::export::pack_json(&conn, std::slice::from_ref(&id)).expect("导出 JSON");
    let parsed: serde_json::Value = serde_json::from_str(&json_text).unwrap();
    assert_eq!(parsed["questions"][0]["options"][0]["text"], "【图】");
    assert_eq!(parsed["questions"][0]["missingImages"][0], "圆");
}

// ---------------------------------------------------------------------------
// 采集台：来源记忆 / 连拍队列 / 题库按来源筛选
// ---------------------------------------------------------------------------

fn save_with_source(conn: &mut Connection, stem: &str, source: Option<&str>) -> String {
    repo::save(
        conn,
        &QuestionInput {
            id: None,
            subject_id: None,
            kind: "解答".to_string(),
            stem: stem.to_string(),
            options: vec![],
            answer: "略".to_string(),
            analysis: String::new(),
            difficulty: 3,
            tags: vec![],
            knowledge_ids: vec![],
            source: source.map(|value| value.to_string()),
            year: None,
            status: Some("DRAFT".to_string()),
            origin: Some("CAPTURE".to_string()),
            needs_review: Some(true),
            review_note: None,
        },
    )
    .expect("保存题目")
}

#[test]
fn capture_source_memory_accumulates_and_can_be_forgotten() {
    let (conn, _dir) = fresh();
    let first =
        crate::capture_store::remember(&conn, "https://example.com/paper/1", "").expect("记来源");
    let second = crate::capture_store::remember(&conn, "https://example.com/paper/1", "例卷")
        .expect("再记一次");
    assert_eq!(first, second, "同一网址应复用同一条来源");

    let list = crate::capture_store::sources(&conn).expect("列来源");
    assert_eq!(list.len(), 1, "同网址只留一条");
    assert_eq!(list[0].capture_count, 2, "重复采集应累加次数");
    assert_eq!(list[0].label, "例卷", "后一次给的标签应覆盖旧值");

    crate::capture_store::star(&conn, &first, true).expect("置顶");
    assert!(crate::capture_store::sources(&conn).expect("列来源")[0].starred);

    crate::capture_store::forget(&conn, &first).expect("忘记来源");
    assert!(
        crate::capture_store::sources(&conn)
            .expect("列来源")
            .is_empty(),
        "忘记后不应再出现"
    );
}

#[test]
fn capture_queue_runs_through_its_states() {
    let (conn, _dir) = fresh();
    let source = crate::capture_store::remember(&conn, "https://a.test/x", "A 站").expect("记来源");
    let item =
        crate::capture_store::enqueue(&conn, "/tmp/a.png", Some(&source), "https://a.test/x")
            .expect("入队");
    assert_eq!(item.status, "PENDING");
    assert_eq!(item.source_id.as_deref(), Some(source.as_str()));

    crate::capture_store::mark(&conn, &item.id, "RUNNING", Some("job-1"), "").expect("标运行中");
    assert_eq!(
        crate::capture_store::get(&conn, &item.id)
            .expect("读回")
            .status,
        "RUNNING"
    );

    crate::capture_store::mark(&conn, &item.id, "FAILED", Some("job-1"), "模型超时")
        .expect("标失败");
    let failed = crate::capture_store::get(&conn, &item.id).expect("读回");
    assert_eq!(failed.status, "FAILED");
    assert_eq!(failed.error, "模型超时");

    crate::capture_store::retry(&conn, &item.id).expect("重试");
    assert_eq!(
        crate::capture_store::get(&conn, &item.id)
            .expect("读回")
            .status,
        "PENDING",
        "重试应回到待识别"
    );

    crate::capture_store::set_item_count(&conn, &item.id, 3).expect("标记完成");
    let done = crate::capture_store::get(&conn, &item.id).expect("读回");
    assert_eq!(done.status, "DONE");
    assert_eq!(done.item_count, 3, "应记住这张图认出几道题");

    assert_eq!(crate::capture_store::clear_done(&conn).expect("清理"), 1);
    assert!(crate::capture_store::list(&conn)
        .expect("列队列")
        .is_empty());
}

#[test]
fn question_list_filters_by_capture_source() {
    let (mut conn, _dir) = fresh();
    save_with_source(&mut conn, "甲卷第一题", Some("https://a.test/paper/1"));
    save_with_source(&mut conn, "乙卷第一题", Some("https://b.test/paper/2"));
    save_with_source(&mut conn, "手录的题", None);

    let counts = repo::sources(&conn).expect("来源统计");
    assert_eq!(counts.len(), 2, "没来源的题不计入");
    assert_eq!(
        counts
            .iter()
            .find(|row| row.source == "https://a.test/paper/1")
            .expect("A 来源")
            .count,
        1
    );

    let page = repo::list(
        &conn,
        &QuestionQuery {
            source: Some("https://a.test/paper/1".to_string()),
            needs_review: Some(true),
            ..Default::default()
        },
    )
    .expect("按来源筛选");
    assert_eq!(page.total, 1);
    assert_eq!(page.items[0].stem, "甲卷第一题");

    let all = repo::list(
        &conn,
        &QuestionQuery {
            needs_review: Some(true),
            ..Default::default()
        },
    )
    .expect("不筛选");
    assert_eq!(all.total, 3);
}

/// 中心下发：本地没见过的题要落库，同一道题再下发只更新、不重复；
/// 本地有未推送的改动时不能被一次拉取盖掉。
#[test]
fn desktop_pull_imports_then_updates_without_duplicates() {
    let (conn, _dir) = fresh();
    let server_id = uuid::Uuid::new_v4().to_string();
    let item = serde_json::json!({
        "serverId": server_id,
        "type": "SINGLE",
        "stem": "中心来的题：1+1 等于几？",
        "options": [{"key": "A", "text": "1"}, {"key": "B", "text": "2"}],
        "answer": "B",
        "analysis": "常识。",
        "difficulty": 1,
        "subjectName": "数学",
        "tags": ["同步", "冒烟"],
        "examPoints": ["正数和负数"],
        "deleted": false
    });

    let applied =
        crate::sync::apply_pull_items(&conn, std::slice::from_ref(&item)).expect("应用下发");
    assert_eq!(applied, 1);
    assert_eq!(
        count(
            &conn,
            &format!("select count(*) from question where id = '{server_id}'")
        ),
        1
    );
    assert_eq!(
        count(
            &conn,
            &format!("select count(*) from question_tag where question_id = '{server_id}'")
        ),
        2,
        "只保留自由标签，旧考点字符串不再落成标签"
    );
    assert_eq!(
        count(
            &conn,
            &format!("select count(*) from question_knowledge where question_id = '{server_id}'")
        ),
        1,
        "旧考点字符串按知识点名解析后挂到 question_knowledge"
    );
    assert_eq!(
        count(
            &conn,
            &format!("select count(*) from sync_link where server_id = '{server_id}'")
        ),
        1
    );
    assert_eq!(
        count(
            &conn,
            &format!(
                "select count(*) from question where id = '{server_id}' and subject_id is not null"
            )
        ),
        1,
        "学科对不上也要落第一个学科，不能留空"
    );

    // 同一道题再下发（题干改了）：仍然只有一行，内容更新
    let mut updated = item.clone();
    updated["stem"] = serde_json::json!("中心来的题（改）：1+1 等于几？");
    updated["tags"] = serde_json::json!(["同步"]);
    assert_eq!(
        crate::sync::apply_pull_items(&conn, std::slice::from_ref(&updated)).expect("再次下发"),
        1
    );
    assert_eq!(count(&conn, "select count(*) from question"), 1);
    let stem: String = conn
        .query_row("select stem from question limit 1", [], |row| row.get(0))
        .expect("取题干");
    assert_eq!(stem, "中心来的题（改）：1+1 等于几？");
    assert_eq!(
        count(
            &conn,
            &format!("select count(*) from question_tag where question_id = '{server_id}'")
        ),
        1,
        "标签按中心快照整体替换，只剩自由标签"
    );

    // 本地改了还没推：这一轮下发跳过，别把老师的改动冲掉
    conn.execute(
        "insert into sync_outbox (id, entity_type, entity_id, op, payload_json, created_at)
         values ('outbox-1', 'question', ?1, 'UPDATE', '{}', datetime('now'))",
        [&server_id],
    )
    .expect("排一条待推送");
    let mut third = item.clone();
    third["stem"] = serde_json::json!("中心又改了一次");
    crate::sync::apply_pull_items(&conn, std::slice::from_ref(&third)).expect("第三次下发");
    let stem: String = conn
        .query_row("select stem from question limit 1", [], |row| row.get(0))
        .expect("取题干");
    assert_eq!(stem, "中心来的题（改）：1+1 等于几？", "待推送期间不被覆盖");
}

// ---------------------------------------------------------------------------
// 教学闭环：名单导入 → 建班 → 组卷 → 提交判分 → 批改
// ---------------------------------------------------------------------------

fn single_choice(conn: &mut Connection, stem: &str, answer: &str) -> String {
    let knowledge_id = first_knowledge_id(conn);
    repo::save(
        conn,
        &QuestionInput {
            id: None,
            subject_id: None,
            kind: "单选题".to_string(),
            stem: stem.to_string(),
            options: vec![
                crate::models::OptionItem {
                    key: "A".into(),
                    text: "1".into(),
                },
                crate::models::OptionItem {
                    key: "B".into(),
                    text: "2".into(),
                },
                crate::models::OptionItem {
                    key: "C".into(),
                    text: "3".into(),
                },
            ],
            answer: answer.to_string(),
            analysis: "核对选项。".to_string(),
            difficulty: 1,
            tags: vec![],
            knowledge_ids: vec![knowledge_id],
            source: None,
            year: None,
            status: Some("NORMAL".to_string()),
            origin: Some("MANUAL".to_string()),
            needs_review: Some(false),
            review_note: None,
        },
    )
    .expect("保存选择题")
}

#[test]
fn roster_import_builds_class_and_soft_delete_hides_students() {
    let (mut conn, _dir) = fresh();
    let text = "姓名,年级,备注\n张三,七年级,数学课代表\n李四,七年级\n张三,七年级\n";

    let preview = crate::teaching::student_import_preview(&conn, text).expect("预览名单");
    assert_eq!(preview.total, 3, "表头不算，三行都读到");
    assert_eq!(preview.duplicates, 1, "同名同年级算重复");
    assert!(preview.rows[0].note.contains("数学课代表"));

    let result = crate::teaching::student_import_commit(&mut conn, text).expect("导入名单");
    assert_eq!(result.created, 2);
    assert_eq!(result.skipped, 1);
    assert_eq!(count(&conn, "select count(*) from local_student"), 2);

    let class = crate::teaching::class_save(
        &conn,
        &crate::teaching::ClassInput {
            id: None,
            name: "七年级一班".into(),
            subject_id: None,
            term: "2026 秋".into(),
        },
    )
    .expect("建班");
    let row = crate::teaching::class_get(&conn, &class).expect("读班");
    assert_eq!(row.join_code.len(), 6, "邀请码 6 位");
    assert!(row
        .join_code
        .chars()
        .all(|c| "ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(c)));

    let students = crate::teaching::student_list(&conn, None, None).expect("学生列表");
    let ids: Vec<String> = students.iter().map(|s| s.id.clone()).collect();
    assert_eq!(
        crate::teaching::class_add_members(&mut conn, &class, &ids).expect("加成员"),
        2
    );
    assert_eq!(
        crate::teaching::class_add_members(&mut conn, &class, &ids).expect("重复加不翻倍"),
        0
    );

    let list = crate::teaching::class_list(&conn).expect("班级列表");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].member_count, 2);

    let found = crate::teaching::student_list(&conn, Some("张三"), None).expect("搜索");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].class_count, 1);

    // 软删除：列表和班级成员里都看不到，但行还在（可追溯）。
    let lisi = students
        .iter()
        .find(|s| s.name == "李四")
        .expect("李四")
        .id
        .clone();
    crate::teaching::student_delete(&conn, &lisi).expect("软删除");
    assert_eq!(
        crate::teaching::student_list(&conn, None, None)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        crate::teaching::class_members(&conn, &class).unwrap().len(),
        1
    );
    assert_eq!(
        count(&conn, "select count(*) from local_student"),
        2,
        "不是硬删"
    );
}

#[test]
fn assignment_submit_auto_grades_objective_and_teacher_finishes_subjective() {
    let (mut conn, _dir) = fresh();
    let choice = single_choice(&mut conn, "1+1 等于几？", "B");
    let knowledge_id = first_knowledge_id(&conn);
    let solved = repo::save(
        &mut conn,
        &QuestionInput {
            id: None,
            subject_id: None,
            kind: "解答".into(),
            stem: "写出你的推导过程。".into(),
            options: vec![],
            answer: "略".into(),
            analysis: "写出关键推导步骤。".into(),
            difficulty: 3,
            tags: vec![],
            knowledge_ids: vec![knowledge_id],
            source: None,
            year: None,
            status: Some("NORMAL".into()),
            origin: Some("MANUAL".into()),
            needs_review: Some(false),
            review_note: None,
        },
    )
    .expect("保存解答题");

    let text = "王五,八年级\n赵六,八年级\n";
    crate::teaching::student_import_commit(&mut conn, text).expect("导入");
    let class = crate::teaching::class_save(
        &conn,
        &crate::teaching::ClassInput {
            id: None,
            name: "八年级二班".into(),
            subject_id: None,
            term: String::new(),
        },
    )
    .expect("建班");
    let students = crate::teaching::student_list(&conn, None, None).expect("学生");
    let ids: Vec<String> = students.iter().map(|s| s.id.clone()).collect();
    crate::teaching::class_add_members(&mut conn, &class, &ids).expect("加成员");
    let wang = students
        .iter()
        .find(|s| s.name == "王五")
        .unwrap()
        .id
        .clone();

    let assignment = crate::teaching::assignment_save(
        &mut conn,
        &crate::teaching::AssignmentInput {
            id: None,
            class_id: class.clone(),
            title: "第一次作业".into(),
            kind: "PRACTICE".into(),
            description: String::new(),
            due_at: None,
            late_hours: 0,
            questions: vec![
                crate::teaching::AssignmentQuestionInput {
                    question_id: choice.clone(),
                    score: 2.0,
                },
                crate::teaching::AssignmentQuestionInput {
                    question_id: solved.clone(),
                    score: 3.0,
                },
            ],
        },
    )
    .expect("建作业");

    // 草稿状态不能交。
    assert!(crate::teaching::submit(
        &mut conn,
        &crate::teaching::SubmitInput {
            assignment_id: assignment.clone(),
            student_id: wang.clone(),
            answers: vec![],
        }
    )
    .is_err());
    crate::teaching::assignment_publish(&conn, &assignment).expect("发布");

    let submitted = crate::teaching::submit(
        &mut conn,
        &crate::teaching::SubmitInput {
            assignment_id: assignment.clone(),
            student_id: wang.clone(),
            answers: vec![
                crate::teaching::AnswerInput {
                    question_id: choice.clone(),
                    answer: "b".into(),
                    duration_ms: Some(9000),
                },
                crate::teaching::AnswerInput {
                    question_id: solved.clone(),
                    answer: "因为 1+1=2".into(),
                    duration_ms: Some(60000),
                },
            ],
        },
    )
    .expect("提交");
    assert_eq!(submitted.answered, 2);
    assert_eq!(submitted.graded, 1, "单选自动判");
    assert_eq!(submitted.pending, 1, "解答题待批");
    assert_eq!(submitted.score, 2.0);
    assert_eq!(submitted.max_score, 5.0);

    let sheet = crate::teaching::attempt_sheet(&conn, &assignment, &wang).expect("作答详情");
    assert_eq!(sheet.items.len(), 2);
    assert_eq!(sheet.items[0].correct, Some(true), "小写 b 也应判对");
    assert_eq!(sheet.items[1].correct, None);
    let solved_attempt = sheet.items[1].attempt_id.clone();

    // 分数越界要挡住。
    assert!(crate::teaching::grade(
        &mut conn,
        &crate::teaching::GradeInput {
            attempt_id: solved_attempt.clone(),
            score: 9.0,
            correct: None,
            remark: String::new(),
        },
        "老师"
    )
    .is_err());

    crate::teaching::grade(
        &mut conn,
        &crate::teaching::GradeInput {
            attempt_id: solved_attempt,
            score: 2.5,
            correct: None,
            remark: "思路对，步骤再写全".to_string(),
        },
        "李老师",
    )
    .expect("批改");

    let detail = crate::teaching::assignment_detail(&conn, &assignment).expect("作业详情");
    let mine = detail
        .submissions
        .iter()
        .find(|row| row.student_id == wang)
        .expect("成绩单有王五");
    assert_eq!(mine.pending, 0);
    assert!(mine.graded);
    assert_eq!(mine.score, 4.5);
    assert_eq!(mine.max_score, 5.0);
    assert_eq!(
        detail
            .submissions
            .iter()
            .filter(|row| row.submitted_at.is_none())
            .count(),
        1,
        "没交的那位仍然列出来"
    );

    // 重交：旧记录作废，成绩单只算最新一次。
    crate::teaching::attempt_clear(&conn, &assignment, &wang).expect("撤销提交");
    let sheet = crate::teaching::attempt_sheet(&conn, &assignment, &wang).expect("重读");
    assert!(sheet.items.is_empty());
    assert_eq!(count(&conn, "select count(*) from local_attempt"), 0);
}

#[test]
fn grade_answer_handles_choice_normalization() {
    use crate::teaching::grade_answer;
    assert_eq!(grade_answer("SINGLE", "B", "b"), Some(true));
    assert_eq!(grade_answer("SINGLE", "B", "B、"), Some(true));
    assert_eq!(grade_answer("SINGLE", "B", "C"), Some(false));
    assert_eq!(grade_answer("MULTI", "AC", "CA"), Some(true));
    assert_eq!(grade_answer("MULTI", "AC", "A"), Some(false));
    assert_eq!(grade_answer("FILL", "2", "2"), None, "填空留给老师");
    assert_eq!(grade_answer("SOLVED", "略", "随便写"), None);
}

/// 预览 → 就地改 → 存草稿：缺答案的题以前只能丢弃，现在补上答案就能存成草稿，
/// 而且存下的是 `DRAFT + needs_review`，复核通过才算正式入库。
#[test]
fn preview_item_can_be_fixed_then_saved_as_draft() {
    let (mut conn, dir) = fresh();
    let subject_id: String = conn
        .query_row("select id from subject order by sort limit 1", [], |row| {
            row.get(0)
        })
        .expect("有学科");
    let point_id: String = conn
        .query_row(
            "select id from knowledge_point where subject_id = ?1 limit 1",
            [&subject_id],
            |row| row.get(0),
        )
        .expect("有知识点");

    let item = serde_json::json!({
        "type": "SOLVED",
        "stem": "解方程 $x+1=2$",
        "options": [],
        "answer": "",
        "analysis": "移项得 $x=1$",
        "difficulty": 2,
        "knowledgeNames": [],
        "knowledgeIds": [point_id],
        "subjectId": subject_id,
        "warnings": [],
        "blocking": [],
        "issues": ["缺答案"],
        "source": null,
        "year": null,
        "confidence": 0.9
    });
    let result = serde_json::json!({"items": [item], "subjectName": "数学"});
    conn.execute(
        "insert into generation_job (id, kind, status, input_json, result_json, progress, created_at, updated_at)
         values ('job-fix', 'generate', 'DONE', '{}', ?1, 100, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        [result.to_string()],
    )
    .expect("插入任务");

    // 补答案：待补清单当场清空。
    let updated =
        crate::jobs::update_item(&conn, "job-fix", 0, &serde_json::json!({"answer": "$x=1$"}))
            .expect("改预览");
    assert!(updated["issues"].as_array().expect("issues").is_empty());
    assert_eq!(updated["answer"], "$x=1$");

    // 解答题带选项会被清掉，避免「解答题还挂 A/B/C/D」。
    let cleaned = crate::jobs::update_item(
        &conn,
        "job-fix",
        0,
        &serde_json::json!({"type": "SOLVED", "options": [{"key": "A", "text": "1"}]}),
    )
    .expect("改题型");
    assert!(cleaned["options"].as_array().expect("options").is_empty());

    let saved = crate::jobs::commit(&mut conn, &dir, "job-fix", &[0]).expect("存草稿");
    assert_eq!(saved["imported"], 1);
    assert_eq!(saved["needsWork"], 0, "答案补齐了，不算待补");
    let (status, needs_review): (String, i64) = conn
        .query_row("select status, needs_review from question", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("读到题目");
    assert_eq!(status, "DRAFT", "存下来的是草稿，不是直接发布");
    assert_eq!(needs_review, 1, "草稿要进草稿箱等复核");

    // 已经存过草稿的任务不允许再改，避免「改了但没生效」。
    assert!(
        crate::jobs::update_item(&conn, "job-fix", 0, &serde_json::json!({"answer": "x"})).is_err()
    );
}

/// 缺答案 / 解析 / 知识点不再拦人：草稿箱本来就是给半成品待的地方。
/// 只有连题干都没认出来（blocking 非空）才真的进不去。
#[test]
fn missing_answer_does_not_block_draft_but_blank_stem_does() {
    let (mut conn, dir) = fresh();
    let subject_id: String = conn
        .query_row("select id from subject order by sort limit 1", [], |row| {
            row.get(0)
        })
        .expect("有学科");

    let half_done = serde_json::json!({
        "type": "SOLVED",
        "stem": "证明勾股定理",
        "options": [],
        "answer": "",
        "analysis": "",
        "difficulty": 3,
        "knowledgeNames": [],
        "knowledgeIds": [],
        "subjectId": subject_id,
        "warnings": [],
        "blocking": [],
        "issues": ["缺答案", "缺解析", "没匹配到知识点（题库按知识点归章）"],
        "source": null,
        "year": null
    });
    let blank_stem = serde_json::json!({
        "type": "SOLVED",
        "stem": "",
        "options": [],
        "answer": "略",
        "analysis": "略",
        "difficulty": 3,
        "knowledgeNames": [],
        "knowledgeIds": [],
        "subjectId": subject_id,
        "warnings": [],
        "blocking": ["缺少题干"],
        "issues": [],
        "source": null,
        "year": null
    });
    let result = serde_json::json!({"items": [half_done, blank_stem]});
    conn.execute(
        "insert into generation_job (id, kind, status, input_json, result_json, progress, created_at, updated_at)
         values ('job-half', 'capture_text', 'DONE', '{}', ?1, 100, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        [result.to_string()],
    )
    .expect("插入任务");

    let saved = crate::jobs::commit(&mut conn, &dir, "job-half", &[0, 1]).expect("存草稿");
    assert_eq!(saved["imported"], 1, "缺答案的那道照样进草稿箱");
    assert_eq!(saved["skipped"], 1, "空题干的那道被拦下");
    assert_eq!(saved["needsWork"], 1, "统计出有 1 道待补");
    let stored: String = conn
        .query_row("select stem from question", [], |row| row.get(0))
        .expect("读到题目");
    assert_eq!(stored, "证明勾股定理");
}

#[test]
fn capture_missing_source_blocks_save_and_can_be_retried() {
    let (mut conn, dir) = fresh();
    let source = dir.join("missing.png");
    let source_text = source.to_string_lossy().to_string();
    insert_done_capture_job(
        &conn,
        "job-missing-capture",
        "capture",
        serde_json::json!({"imagePath": source_text}),
        vec![capture_draft_item(&conn, "原图稍后补齐", None)],
    );

    let error = crate::jobs::commit(&mut conn, &dir, "job-missing-capture", &[0])
        .expect_err("原图不存在时整批不能写题");
    assert!(
        error.contains("第 1 条") && error.contains("原图不存在"),
        "{error}"
    );
    assert_eq!(count(&conn, "select count(*) from question"), 0);
    assert_eq!(count(&conn, "select count(*) from question_asset"), 0);
    let status: String = conn
        .query_row(
            "select status from generation_job where id = 'job-missing-capture'",
            [],
            |row| row.get(0),
        )
        .expect("读任务状态");
    assert_eq!(status, "DONE", "前置失败不能消费任务");

    write_test_png(&source);
    let saved = crate::jobs::commit(&mut conn, &dir, "job-missing-capture", &[0])
        .expect("原图补齐后可以重试");
    assert_eq!(saved["imported"], 1);
    assert_eq!(saved["skipped"], 0);
    assert!(saved["problems"].as_array().expect("problems").is_empty());
    assert_eq!(count(&conn, "select count(*) from question"), 1);
    assert_eq!(
        count(
            &conn,
            "select count(*) from question_asset where kind = 'CAPTURE'"
        ),
        0,
        "截图是临时文件，不能跟题目绑定"
    );
}

#[test]
fn capture_batch_bad_source_blocks_every_question_and_can_be_retried() {
    let (mut conn, dir) = fresh();
    let first = dir.join("first.png");
    let bad = dir.join("bad.png");
    write_test_png(&first);
    std::fs::write(&bad, b"this is not an image").expect("写坏原图");
    let first_text = first.to_string_lossy().to_string();
    let bad_text = bad.to_string_lossy().to_string();
    insert_done_capture_job(
        &conn,
        "job-bad-capture",
        "capture_batch",
        serde_json::json!({}),
        vec![
            capture_draft_item(&conn, "第一题", Some(&first_text)),
            capture_draft_item(&conn, "第二题", Some(&bad_text)),
        ],
    );

    let error = crate::jobs::commit(&mut conn, &dir, "job-bad-capture", &[0, 1])
        .expect_err("第二张不是图片时整批失败");
    assert!(
        error.contains("第 2 条") && error.contains("不是支持的"),
        "{error}"
    );
    assert_eq!(
        count(&conn, "select count(*) from question"),
        0,
        "第一张也不能先落库"
    );
    assert_eq!(count(&conn, "select count(*) from question_asset"), 0);
    let status: String = conn
        .query_row(
            "select status from generation_job where id = 'job-bad-capture'",
            [],
            |row| row.get(0),
        )
        .expect("读任务状态");
    assert_eq!(status, "DONE");

    write_test_png(&bad);
    let saved = crate::jobs::commit(&mut conn, &dir, "job-bad-capture", &[0, 1])
        .expect("坏图替换后可以整批重试");
    assert_eq!(saved["imported"], 2);
    assert_eq!(saved["skipped"], 0);
    assert_eq!(count(&conn, "select count(*) from question"), 2);
    assert_eq!(
        count(
            &conn,
            "select count(*) from question_asset where kind = 'CAPTURE'"
        ),
        0,
        "批量任务也不绑定截图"
    );
}

#[test]
fn text_and_structured_web_jobs_do_not_require_source_images() {
    let (mut conn, dir) = fresh();
    for (id, kind) in [
        ("job-plain-text", "capture_text"),
        ("job-web-dom", "capture_web"),
    ] {
        insert_done_capture_job(
            &conn,
            id,
            kind,
            serde_json::json!({}),
            vec![capture_draft_item(&conn, "无原图也能存草稿", None)],
        );
        let saved = crate::jobs::commit(&mut conn, &dir, id, &[0]).expect("无原图任务正常保存");
        assert_eq!(saved["imported"], 1);
    }
    assert_eq!(count(&conn, "select count(*) from question"), 2);
    assert_eq!(count(&conn, "select count(*) from question_asset"), 0);
}

#[test]
fn database_snapshot_opens_standalone_with_data_and_version() {
    let (mut conn, dir) = fresh();
    let knowledge_id = first_knowledge_id(&conn);
    repo::save(
        &mut conn,
        &QuestionInput {
            id: None,
            subject_id: None,
            kind: "解答".to_string(),
            stem: "快照里应该有这道题。".to_string(),
            options: vec![],
            answer: "$1$".to_string(),
            analysis: "答案为 $1$。".to_string(),
            difficulty: 1,
            tags: vec![],
            knowledge_ids: vec![knowledge_id],
            source: None,
            year: None,
            status: Some("NORMAL".to_string()),
            origin: Some("MANUAL".to_string()),
            needs_review: Some(false),
            review_note: None,
        },
    )
    .expect("保存题目");

    let target = dir.join("snapshot.db");
    // Navicat / DB Browser 打开的就是这种文件：单文件、无 WAL 依赖。
    conn.execute("vacuum into ?1", [target.to_string_lossy().to_string()])
        .expect("导出快照");
    assert!(target.exists());

    let copy = Connection::open(&target).expect("快照能被独立打开");
    let questions: i64 = copy
        .query_row("select count(*) from question", [], |row| row.get(0))
        .unwrap();
    assert_eq!(questions, 1);
    let version: i64 = copy
        .query_row("pragma user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, db::SCHEMA_VERSION, "快照要保留迁移版本");
    // 快照是普通库，不应该再带 -wal / -shm。
    assert!(!dir.join("snapshot.db-wal").exists());
}

/// 知识点 id：取种子里第一个带知识点的学科的前两个。
fn two_knowledge_points(conn: &Connection) -> (String, String) {
    let mut stmt = conn
        .prepare("select id from knowledge_point order by subject_id, sort limit 2")
        .expect("查知识点");
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(ids.len(), 2, "种子教材里至少要有两个知识点");
    (ids[0].clone(), ids[1].clone())
}

#[test]
fn insights_matrix_and_wrong_book_follow_grading() {
    use crate::teaching::{
        self, AnswerInput, AssignmentInput, AssignmentQuestionInput, ClassInput, GradeInput,
        SubmitInput,
    };

    let (mut conn, _dir) = fresh();
    let (kp1, kp2) = two_knowledge_points(&conn);
    let mut make = |stem: &str, kind: &str, answer: &str, kp: &str| {
        repo::save(
            &mut conn,
            &QuestionInput {
                id: None,
                subject_id: None,
                kind: kind.to_string(),
                stem: stem.to_string(),
                options: vec![
                    crate::models::OptionItem {
                        key: "A".into(),
                        text: "1".into(),
                    },
                    crate::models::OptionItem {
                        key: "B".into(),
                        text: "2".into(),
                    },
                ],
                answer: answer.to_string(),
                analysis: "核对计算过程。".to_string(),
                difficulty: 1,
                tags: vec![],
                knowledge_ids: vec![kp.to_string()],
                source: None,
                year: None,
                status: Some("NORMAL".to_string()),
                origin: Some("MANUAL".to_string()),
                needs_review: Some(false),
                review_note: None,
            },
        )
        .expect("保存题目")
    };
    let right = make("1+1=?", "单选题", "B", &kp1);
    let wrong = make("2+2=?", "单选题", "B", &kp1);
    let solved = make("写出推导。", "解答", "略", &kp2);

    teaching::student_import_commit(&mut conn, "甲同学,八年级\n乙同学,八年级\n").expect("导入");
    let class = teaching::class_save(
        &conn,
        &ClassInput {
            id: None,
            name: "八年级一班".into(),
            subject_id: None,
            term: String::new(),
        },
    )
    .expect("建班");
    let students = teaching::student_list(&conn, None, None).expect("学生");
    let ids: Vec<String> = students.iter().map(|s| s.id.clone()).collect();
    teaching::class_add_members(&mut conn, &class, &ids).expect("加成员");
    let jia = students
        .iter()
        .find(|s| s.name == "甲同学")
        .unwrap()
        .id
        .clone();

    let assignment = teaching::assignment_save(
        &mut conn,
        &AssignmentInput {
            id: None,
            class_id: class.clone(),
            title: "学情作业".into(),
            kind: "PRACTICE".into(),
            description: String::new(),
            due_at: None,
            late_hours: 0,
            questions: vec![
                AssignmentQuestionInput {
                    question_id: right.clone(),
                    score: 2.0,
                },
                AssignmentQuestionInput {
                    question_id: wrong.clone(),
                    score: 2.0,
                },
                AssignmentQuestionInput {
                    question_id: solved.clone(),
                    score: 2.0,
                },
            ],
        },
    )
    .expect("建作业");
    teaching::assignment_publish(&conn, &assignment).expect("发布");

    let result = teaching::submit(
        &mut conn,
        &SubmitInput {
            assignment_id: assignment.clone(),
            student_id: jia.clone(),
            answers: vec![
                AnswerInput {
                    question_id: right.clone(),
                    answer: "B".into(),
                    duration_ms: None,
                },
                AnswerInput {
                    question_id: wrong.clone(),
                    answer: "A".into(),
                    duration_ms: None,
                },
                AnswerInput {
                    question_id: solved.clone(),
                    answer: "先移项".into(),
                    duration_ms: None,
                },
            ],
        },
    )
    .expect("提交");
    assert_eq!(result.graded, 2);
    assert_eq!(result.pending, 1, "解答题待批");

    // 错题本：做错的入本、做对的不入本。
    let book = crate::insights::wrong_list(&conn, &jia, None).expect("错题本");
    assert_eq!(book.len(), 1);
    assert_eq!(book[0].question_id, wrong);
    assert_eq!(book[0].state, "NEW");
    assert!(!book[0].knowledge.is_empty(), "错题要带知识点名");

    // 班级矩阵：两个知识点、两个学生；待批 1 题不计入掌握度。
    let class_view = crate::insights::class_insight(&conn, &class).expect("班级学情");
    assert_eq!(class_view.class_name, "八年级一班");
    assert_eq!(class_view.kps.len(), 2);
    assert_eq!(class_view.students.len(), 2);
    assert_eq!(class_view.pending_grade, 1);
    assert_eq!(class_view.graded_total, 2);
    let jia_row = class_view.students.iter().find(|s| s.id == jia).unwrap();
    // 对一题（难度 1 → 0.80）错一题（0.05）：(0.80+0.05)/2 = 42.5 → 43
    assert_eq!(jia_row.mastery, 43, "掌握度按统一公式算");
    assert_eq!(jia_row.answered, 2);
    assert_eq!(jia_row.correct, 1);
    assert_eq!(jia_row.status, "LEARNING");
    let you = class_view.students.iter().find(|s| s.id != jia).unwrap();
    assert_eq!(you.mastery, 0, "没交作业的学生是未开始");
    assert_eq!(
        you.cells.iter().filter(|v| **v < 0).count(),
        2,
        "没作答的格子是 -1"
    );
    assert_eq!(class_view.common_wrong.len(), 1, "共性错题只有那一题");
    assert_eq!(class_view.common_wrong[0].rate, 100);

    // 批完主观题：掌握度按两对一错重算。
    let sheet = teaching::attempt_sheet(&conn, &assignment, &jia).expect("成绩单");
    let pending_item = sheet
        .items
        .iter()
        .find(|item| item.correct.is_none())
        .expect("有待批项");
    teaching::grade(
        &mut conn,
        &GradeInput {
            attempt_id: pending_item.attempt_id.clone(),
            score: 2.0,
            correct: None,
            remark: "思路对".into(),
        },
        "老师",
    )
    .expect("批改");
    let after = crate::insights::student_insight(&conn, &jia).expect("学生学情");
    assert_eq!(after.answered, 3);
    assert_eq!(after.pending, 0);
    assert_eq!(after.trend.len(), 8);
    assert_eq!(after.trend[7].answered, 3);
    assert_eq!(after.wrong_count, 1, "错题还没结案");
    assert_eq!(after.recent.len(), 1);
    assert!(
        after.score_rate > 0.6 && after.score_rate < 1.0,
        "得分率 4/6"
    );

    // 复习一次：状态进 REVIEWING、复习次数 +1、下次复习时间往后排。
    let row = crate::insights::wrong_list(&conn, &jia, Some("NEW")).expect("按状态筛");
    assert_eq!(row.len(), 1);
    crate::insights::wrong_review(
        &conn,
        &crate::insights::ReviewInput {
            id: row[0].id.clone(),
            state: "reviewing".into(),
            note: "讲过一遍".into(),
        },
    )
    .expect("标记复习");
    let row = crate::insights::wrong_list(&conn, &jia, Some("REVIEWING")).expect("复习中");
    assert_eq!(row.len(), 1);
    assert_eq!(row[0].review_count, 1);
    assert_eq!(row[0].note, "讲过一遍");
    assert!(row[0].next_review_at.is_some());

    // 重交且做对：错题结案，不再出现在待复习里。
    teaching::submit(
        &mut conn,
        &SubmitInput {
            assignment_id: assignment.clone(),
            student_id: jia.clone(),
            answers: vec![
                AnswerInput {
                    question_id: right,
                    answer: "B".into(),
                    duration_ms: None,
                },
                AnswerInput {
                    question_id: wrong.clone(),
                    answer: "B".into(),
                    duration_ms: None,
                },
                AnswerInput {
                    question_id: solved,
                    answer: "同上".into(),
                    duration_ms: None,
                },
            ],
        },
    )
    .expect("重交");
    let book = crate::insights::wrong_list(&conn, &jia, None).expect("错题本");
    assert_eq!(book.len(), 1);
    assert_eq!(book[0].state, "MASTERED", "做对了就结案，记录留着");
    assert_eq!(book[0].wrong_count, 1);
    assert_eq!(crate::insights::due_review(&conn, Some(&jia)).unwrap(), 0);

    crate::insights::wrong_remove(&conn, &book[0].id).expect("删掉");
    assert!(crate::insights::wrong_list(&conn, &jia, None)
        .unwrap()
        .is_empty());
}

/// 拍照批改整条链路：纸质卷子拍照 → 切题挂到题 → 给分 → 学情与错题本跟着更新。
#[test]
fn photo_grading_flow_cuts_pages_grades_and_updates_insights() {
    use crate::grading;
    use crate::teaching::{self, AssignmentInput, AssignmentQuestionInput, ClassInput, GradeInput};

    let (mut conn, dir) = fresh();
    let (kp1, _kp2) = two_knowledge_points(&conn);

    let question = repo::save(
        &mut conn,
        &QuestionInput {
            id: None,
            subject_id: None,
            kind: "解答".to_string(),
            stem: "求阴影面积。".to_string(),
            options: vec![],
            answer: "12".to_string(),
            analysis: "由图形面积公式计算。".to_string(),
            difficulty: 3,
            tags: vec![],
            knowledge_ids: vec![kp1.clone()],
            source: None,
            year: None,
            status: Some("NORMAL".to_string()),
            origin: Some("MANUAL".to_string()),
            needs_review: Some(false),
            review_note: None,
        },
    )
    .expect("保存题目");

    teaching::student_import_commit(&mut conn, "乙同学,八年级\n").expect("导入学生");
    let class = teaching::class_save(
        &conn,
        &ClassInput {
            id: None,
            name: "八年级二班".into(),
            subject_id: None,
            term: String::new(),
        },
    )
    .expect("建班");
    let students = teaching::student_list(&conn, None, None).expect("学生");
    let yi = students[0].id.clone();
    teaching::class_add_members(&mut conn, &class, &[yi.clone()]).expect("加成员");

    let assignment = teaching::assignment_save(
        &mut conn,
        &AssignmentInput {
            id: None,
            class_id: class.clone(),
            title: "纸质作业".into(),
            kind: "PRACTICE".into(),
            description: String::new(),
            due_at: None,
            late_hours: 0,
            questions: vec![AssignmentQuestionInput {
                question_id: question.clone(),
                score: 10.0,
            }],
        },
    )
    .expect("建作业");
    teaching::assignment_publish(&conn, &assignment).expect("发布");

    // 学生没在线交卷：代建一次空提交。
    let submission = grading::paper_submit(&mut conn, &assignment, &yi).expect("代建提交");
    assert!(!submission.is_empty());

    // 交上来一张整页照片。
    let mut img = image::RgbImage::new(300, 200);
    for y in 30..90 {
        for x in 20..280 {
            img.put_pixel(x, y, image::Rgb([20, 20, 20]));
        }
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("编码 PNG");
    let data_url = format!(
        "data:image/png;base64,{}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, buf.into_inner())
    );
    let page = grading::store_page(
        &conn,
        &dir,
        &grading::PhotoInput {
            student_id: yi.clone(),
            assignment_id: Some(assignment.clone()),
            data_url,
        },
    )
    .expect("存整页照片");
    assert_eq!(page.kind, "PAGE");
    assert!(dir.join("assets").join(&page.relative_path).exists());

    // 自动切题能找到块。
    let regions = grading::detect_regions(&conn, &dir, &page.id).expect("检测题块");
    assert!(!regions.is_empty(), "整页照片上要能找出题块");

    // 切一块挂到那道题。
    let sheet = teaching::attempt_sheet(&conn, &assignment, &yi).expect("成绩单");
    assert_eq!(sheet.items.len(), 1);
    let attempt_id = sheet.items[0].attempt_id.clone();
    let block = grading::crop_from_page(
        &conn,
        &dir,
        &grading::CropRequest {
            page_asset_id: page.id.clone(),
            region: regions[0].clone(),
            attempt_id: Some(attempt_id.clone()),
        },
    )
    .expect("切题");
    assert_eq!(block.attempt_id.as_deref(), Some(attempt_id.as_str()));
    assert_eq!(block.page_asset_id.as_deref(), Some(page.id.as_str()));

    // 成绩单里这道题要能看到照片。
    let sheet = teaching::attempt_sheet(&conn, &assignment, &yi).expect("成绩单");
    assert_eq!(sheet.items[0].photos.len(), 1);
    assert_eq!(sheet.items[0].photos[0].id, block.id);

    // 提交列表里带照片数与待批数。
    let detail = teaching::assignment_detail(&conn, &assignment).expect("作业详情");
    let row = detail
        .submissions
        .iter()
        .find(|r| r.student_id == yi)
        .expect("提交行");
    assert_eq!(row.photo_count, 2, "整页 + 题块都算");
    assert_eq!(row.pending, 1, "还没给分");

    // 给分：做错。错题本与学情要跟着更新。
    teaching::grade(
        &mut conn,
        &GradeInput {
            attempt_id: attempt_id.clone(),
            score: 3.0,
            correct: Some(false),
            remark: "算错了".into(),
        },
        "老师",
    )
    .expect("批改");

    let book = crate::insights::wrong_list(&conn, &yi, None).expect("错题本");
    assert_eq!(book.len(), 1, "打错分要进错题本");
    assert_eq!(book[0].question_id, question);

    let after = crate::insights::student_insight(&conn, &yi).expect("学情");
    assert_eq!(after.answered, 1);
    assert_eq!(after.correct, 0);
    assert!(after.overall < 20, "做错掌握度很低，实际 {}", after.overall);

    // 改判做对：错题结案，掌握度上去。
    teaching::grade(
        &mut conn,
        &GradeInput {
            attempt_id: attempt_id.clone(),
            score: 9.0,
            correct: Some(true),
            remark: "复核后给分".into(),
        },
        "老师",
    )
    .expect("改判");
    let book = crate::insights::wrong_list(&conn, &yi, None).expect("错题本");
    assert_eq!(book.len(), 1);
    assert_eq!(book[0].state, "MASTERED", "改判对了就结案");
    let after = crate::insights::student_insight(&conn, &yi).expect("学情");
    assert_eq!(after.correct, 1);
    assert!(after.overall > 70, "做对掌握度要高，实际 {}", after.overall);

    // 删掉照片后，成绩单里就看不到图了。
    grading::remove(&conn, &block.id).expect("删照片");
    let sheet = teaching::attempt_sheet(&conn, &assignment, &yi).expect("成绩单");
    assert!(sheet.items[0].photos.is_empty(), "删掉的照片不再出现");
    // 整页还在（软删只针对被删的那一张）。
    assert!(grading::list(&conn, &yi, Some(&assignment)).unwrap().len() >= 1);
}

// ---------------------------------------------------------------------------
// 旧考点并入知识点：迁移、列表互斥、入库门槛
// ---------------------------------------------------------------------------

/// 造一个只有 0001–0009 的 v9 库，用来验证 0010 迁移。
fn fresh_v9() -> Connection {
    let conn = Connection::open_in_memory().expect("打开内存库");
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    for sql in [
        include_str!("../migrations/0001_init.sql"),
        include_str!("../migrations/0002_local_user.sql"),
        include_str!("../migrations/0003_capture.sql"),
        include_str!("../migrations/0004_teaching.sql"),
        include_str!("../migrations/0005_insights.sql"),
        include_str!("../migrations/0006_courseware.sql"),
        include_str!("../migrations/0007_grading_photos.sql"),
        include_str!("../migrations/0008_llm_model_has_key.sql"),
        include_str!("../migrations/0009_drop_capture_assets.sql"),
    ] {
        conn.execute_batch(sql).expect("执行 v9 之前的迁移");
    }
    conn.pragma_update(None, "user_version", 9).unwrap();
    conn
}

#[test]
fn migration_0010_merges_exam_points_into_knowledge() {
    let mut conn = fresh_v9();
    conn.execute_batch(
        "insert into subject (id, code, name, sort) values ('subj-MATH','MATH','数学',0);
         insert into knowledge_point (id, subject_id, name, sort)
           values ('kp-existing','subj-MATH','已有知识点',0);
         insert into question (id, subject_id, type, stem, options_json, status, created_at, updated_at)
           values ('q-new','subj-MATH','SINGLE','新考点','[]','DRAFT','2026-01-01','2026-01-01'),
                  ('q-reuse','subj-MATH','SINGLE','复用同名','[]','DRAFT','2026-01-01','2026-01-01'),
                  ('q-null',null,'SINGLE','没学科的题','[]','DRAFT','2026-01-01','2026-01-01');
         insert into question_tag (question_id, tag, kind) values
           ('q-new','全新考点','EXAM_POINT'),
           ('q-reuse','已有知识点','EXAM_POINT'),
           ('q-null','空学科考点','EXAM_POINT');",
    )
    .expect("造 v9 的旧数据");

    db::migrate(&mut conn).expect("执行 0010 迁移");

    let version: i64 = conn
        .query_row("pragma user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        version,
        db::SCHEMA_VERSION,
        "0010 之后还会接着跑 0011–0015（中心照片、试卷、学号与成绩、课件批量导入、制图用途路由）"
    );
    assert_eq!(
        count(
            &conn,
            "select count(*) from question_tag where kind = 'EXAM_POINT'"
        ),
        0,
        "旧标签要清干净"
    );
    assert!(
        conn.query_row("select count(*) from exam_point", [], |row| {
            row.get::<_, i64>(0)
        })
        .is_err(),
        "exam_point 表应被删除"
    );
    assert_eq!(
        count(&conn, "select count(*) from question_knowledge"),
        3,
        "三道题各挂上一个知识点"
    );
    let name: String = conn
        .query_row(
            "select kp.name from question_knowledge qk
             join knowledge_point kp on kp.id = qk.knowledge_point_id
             where qk.question_id = 'q-new'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name, "全新考点", "新建的知识点用标签名");
    let reused: String = conn
        .query_row(
            "select qk.knowledge_point_id from question_knowledge qk
             where qk.question_id = 'q-reuse'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reused, "kp-existing", "同名知识点要复用而不是再建一个");
    let null_subject: String = conn
        .query_row(
            "select kp.subject_id from question_knowledge qk
             join knowledge_point kp on kp.id = qk.knowledge_point_id
             where qk.question_id = 'q-null'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        null_subject, "subj-MATH",
        "题目没学科时挂到排序最前的真实学科"
    );
}

#[test]
fn question_lists_are_mutually_exclusive_and_uncategorized_filters() {
    let (mut conn, _dir) = fresh();
    let kp = first_knowledge_id(&conn);

    let published = repo::save(&mut conn, &complete_choice_input(&kp, "PUBLISHED")).unwrap();

    let mut draft = complete_choice_input(&kp, "DRAFT");
    draft.knowledge_ids.clear();
    let draft_id = repo::save(&mut conn, &draft).unwrap();

    let mut rejected = complete_choice_input(&kp, "DRAFT");
    rejected.knowledge_ids.clear();
    rejected.needs_review = Some(false);
    rejected.review_note = Some("不合格".into());
    let rejected_id = repo::save(&mut conn, &rejected).unwrap();

    // 默认题库列表：只有正式题，草稿与驳回都不掺进来。
    let main = repo::list(&conn, &QuestionQuery::default()).unwrap();
    assert_eq!(main.total, 1);
    assert_eq!(main.items[0].id, published);

    let drafts = repo::list(
        &conn,
        &QuestionQuery {
            needs_review: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(drafts.total, 1);
    assert_eq!(drafts.items[0].id, draft_id);

    let rejected_list = repo::list(
        &conn,
        &QuestionQuery {
            rejected: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(rejected_list.total, 1);
    assert_eq!(rejected_list.items[0].id, rejected_id);

    // 批量归类：草稿箱里只看没挂知识点的那道。
    let uncategorized = repo::list(
        &conn,
        &QuestionQuery {
            needs_review: Some(true),
            uncategorized: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(uncategorized.total, 1);
    assert_eq!(uncategorized.items[0].id, draft_id);

    // 题库列表里的 uncategorized：正式题有知识点，结果为空。
    let uncategorized_main = repo::list(
        &conn,
        &QuestionQuery {
            uncategorized: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(uncategorized_main.total, 0);
}

#[test]
fn publication_issues_only_reports_missing_knowledge() {
    let (mut conn, _dir) = fresh();
    let kp = first_knowledge_id(&conn);

    let mut draft = complete_choice_input(&kp, "DRAFT");
    draft.knowledge_ids.clear();
    let id = repo::save(&mut conn, &draft).unwrap();
    let issues = repo::get(&conn, &id).unwrap().issues;
    assert!(issues.contains(&"缺知识点".to_string()));
    assert!(
        !issues.iter().any(|issue| issue.contains("考察点")),
        "不再有独立的考点门槛：{issues:?}"
    );

    let mut published = complete_choice_input(&kp, "PUBLISHED");
    published.knowledge_ids.clear();
    let error = repo::save(&mut conn, &published).unwrap_err();
    assert!(error.contains("缺知识点"));
    assert!(!error.contains("考察点"), "报错里不该再出现：{error}");
}

// ---------------------------------------------------------------------------
// 试卷（0012）：建表、组卷、统一分值、差额校验、快照冻结
// ---------------------------------------------------------------------------

fn paper_create_custom(conn: &mut Connection, title: &str) -> crate::paper::PaperDetail {
    crate::paper::create(
        conn,
        &crate::paper::PaperCreateInput {
            title: title.to_string(),
            score_scheme: "CUSTOM".to_string(),
            ..Default::default()
        },
    )
    .expect("建试卷")
}

/// 建一份 100 分自定义卷，把唯一大题设为每题 50 分，再加入两道已入库的题。
fn balanced_paper(conn: &mut Connection) -> (crate::paper::PaperDetail, String, String) {
    let kp = first_knowledge_id(conn);
    let mut q1 = complete_choice_input(&kp, "PUBLISHED");
    q1.stem = "试卷测试题一：$1+1=$？".to_string();
    let id1 = repo::save(conn, &q1).unwrap();
    let mut q2 = complete_choice_input(&kp, "PUBLISHED");
    q2.stem = "试卷测试题二：$2+2=$？".to_string();
    let id2 = repo::save(conn, &q2).unwrap();

    let paper = paper_create_custom(conn, "单元测验");
    let section = paper.sections[0].id.clone();
    crate::paper::save_section(
        conn,
        &paper.id,
        &crate::paper::PaperSectionInput {
            id: Some(section.clone()),
            title: "一、解答题".to_string(),
            score_mode: "UNIFORM".to_string(),
            unit_score: Some(50.0),
            ..Default::default()
        },
    )
    .expect("改大题分值");
    crate::paper::add_questions(conn, &paper.id, &section, &[id1.clone(), id2.clone()])
        .expect("加入题目");
    (paper, id1, id2)
}

#[test]
fn paper_migration_creates_tables_without_touching_questions() {
    let (conn, _dir) = fresh();
    let version: i64 = conn
        .query_row("pragma user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        version,
        db::SCHEMA_VERSION,
        "迁移全部跑完后版本号应等于迁移文件数"
    );
    for table in [
        "local_paper",
        "local_paper_section",
        "local_paper_question",
        "local_paper_template",
        "local_paper_print",
    ] {
        let found: i64 = conn
            .query_row(
                "select count(*) from sqlite_master where type = 'table' and name = ?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "缺表 {table}");
    }
    // 旧数据不受影响：迁移前建的题还在。
    let kp = first_knowledge_id(&conn);
    let mut conn = conn;
    let id = repo::save(&mut conn, &complete_choice_input(&kp, "PUBLISHED")).unwrap();
    assert_eq!(repo::get(&conn, &id).unwrap().id, id);
}

#[test]
fn paper_create_uses_scheme_structure_and_total() {
    let (mut conn, _dir) = fresh();
    let paper = crate::paper::create(
        &mut conn,
        &crate::paper::PaperCreateInput {
            title: "期中卷".to_string(),
            score_scheme: "XGK1_MATH".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(paper.total_score, 150.0);
    assert_eq!(paper.duration_min, 120);
    assert_eq!(paper.sections.len(), 4);
    assert_eq!(paper.sections[0].unit_score, 5.0);
    assert_eq!(paper.sections[3].score_mode, "PER_ITEM");
    assert_eq!(paper.layout["paperSize"], "A4");
    assert_eq!(paper.layout["answerKeyPage"], true);
}

#[test]
fn paper_uniform_score_rescores_every_question() {
    let (mut conn, _dir) = fresh();
    let (paper, _, _) = balanced_paper(&mut conn);
    let section = paper.sections[0].id.clone();

    let fresh_paper = crate::paper::get(&conn, &paper.id).unwrap();
    assert_eq!(fresh_paper.assigned_score, 100.0);
    assert_eq!(fresh_paper.sections[0].score_total, 100.0);
    assert!(fresh_paper.sections[0].note.contains("每小题 50 分"));

    let updated = crate::paper::save_section(
        &mut conn,
        &paper.id,
        &crate::paper::PaperSectionInput {
            id: Some(section),
            title: "一、解答题".to_string(),
            score_mode: "UNIFORM".to_string(),
            unit_score: Some(30.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(updated.assigned_score, 60.0);
    assert!(updated.sections[0].note.contains("每小题 30 分，共 60 分"));
}

#[test]
fn paper_finalize_rejects_unbalanced_scores_then_accepts() {
    let (mut conn, _dir) = fresh();
    let paper = paper_create_custom(&mut conn, "分值校验卷");
    let section = paper.sections[0].id.clone();
    let kp = first_knowledge_id(&conn);
    let id1 = repo::save(&mut conn, &complete_choice_input(&kp, "PUBLISHED")).unwrap();
    crate::paper::save_section(
        &mut conn,
        &paper.id,
        &crate::paper::PaperSectionInput {
            id: Some(section.clone()),
            title: "一、解答题".to_string(),
            score_mode: "UNIFORM".to_string(),
            unit_score: Some(40.0),
            ..Default::default()
        },
    )
    .unwrap();
    crate::paper::add_questions(&mut conn, &paper.id, &section, &[id1]).unwrap();

    let check = crate::paper::score_check(&conn, &paper.id).unwrap();
    assert_eq!(check.total, 100.0);
    assert_eq!(check.assigned, 40.0);
    assert_eq!(check.delta, 60.0);
    assert!(!check.balanced);

    let error = crate::paper::finalize(&mut conn, &paper.id).unwrap_err();
    assert!(error.contains("分值不平"), "{error}");
    assert!(error.contains("还差"), "{error}");

    // 补齐差额才能定稿。
    crate::paper::save_section(
        &mut conn,
        &paper.id,
        &crate::paper::PaperSectionInput {
            id: Some(section),
            title: "一、解答题".to_string(),
            score_mode: "UNIFORM".to_string(),
            unit_score: Some(100.0),
            ..Default::default()
        },
    )
    .unwrap();
    let done = crate::paper::finalize(&mut conn, &paper.id).unwrap();
    assert_eq!(done.status, "FINAL");
    assert!(done.finalized_at.is_some());

    // 定稿后只读：改总分、加题都被挡。
    assert!(crate::paper::set_total(&mut conn, &paper.id, 120.0, true).is_err());
    assert!(crate::paper::add_questions(&mut conn, &paper.id, &paper.sections[0].id, &[]).is_err());
}

#[test]
fn paper_per_item_adjustment_switches_section_mode() {
    let (mut conn, _dir) = fresh();
    let (paper, _, _) = balanced_paper(&mut conn);
    let section = paper.sections[0].id.clone();
    let updated = crate::paper::set_scores(
        &mut conn,
        &paper.id,
        &[crate::paper::ScoreItem {
            section_id: section.clone(),
            ord: 0,
            score: 45.0,
        }],
    )
    .unwrap();
    assert_eq!(updated.sections[0].score_mode, "PER_ITEM");
    assert_eq!(updated.assigned_score, 95.0);
    assert!(updated.sections[0].note.contains("共 95 分"));
}

#[test]
fn paper_snapshot_survives_question_edit_and_duplicate_copies_it() {
    let (mut conn, _dir) = fresh();
    let (paper, qid, _) = balanced_paper(&mut conn);
    let before = crate::paper::get(&conn, &paper.id).unwrap();
    let stem_before = before.sections[0].questions[0]
        .snapshot
        .get("stem")
        .and_then(|value| value.as_str())
        .unwrap()
        .to_string();

    // 改题库里的题：卷内快照不变。
    let kp = first_knowledge_id(&conn);
    let mut edit = complete_choice_input(&kp, "PUBLISHED");
    edit.id = Some(qid);
    edit.stem = "题库已经改过的题干".to_string();
    repo::save(&mut conn, &edit).unwrap();
    let after = crate::paper::get(&conn, &paper.id).unwrap();
    assert_eq!(
        after.sections[0].questions[0]
            .snapshot
            .get("stem")
            .and_then(|value| value.as_str())
            .unwrap(),
        stem_before
    );

    // 复制：草稿态、题目与分值都在。
    let copy = crate::paper::duplicate(&mut conn, &paper.id).unwrap();
    assert!(copy.title.ends_with("（副本）"));
    assert_eq!(copy.status, "DRAFT");
    assert_eq!(copy.question_count, 2);
    assert_eq!(copy.assigned_score, 100.0);

    // 软删后进回收站，恢复后回到列表。
    crate::paper::soft_delete(&conn, &copy.id, true).unwrap();
    let trashed = crate::paper::list(
        &conn,
        &crate::paper::PaperQuery {
            trashed: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(trashed.items.iter().any(|item| item.id == copy.id));
    let normal = crate::paper::list(&conn, &crate::paper::PaperQuery::default()).unwrap();
    assert!(!normal.items.iter().any(|item| item.id == copy.id));
    crate::paper::soft_delete(&conn, &copy.id, false).unwrap();
    let restored = crate::paper::list(&conn, &crate::paper::PaperQuery::default()).unwrap();
    assert!(restored.items.iter().any(|item| item.id == copy.id));
}

#[test]
fn paper_archive_hides_from_default_list() {
    let (mut conn, _dir) = fresh();
    let (paper, _, _) = balanced_paper(&mut conn);
    crate::paper::set_status(&mut conn, &paper.id, "ARCHIVED").unwrap();
    let normal = crate::paper::list(&conn, &crate::paper::PaperQuery::default()).unwrap();
    assert!(normal.items.is_empty(), "归档卷默认不出现在列表");
    let archived = crate::paper::list(
        &conn,
        &crate::paper::PaperQuery {
            status: Some("ARCHIVED".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(archived.total, 1);
}

/// 定稿之后试卷是只读的：改题名、套方案都要先「重新打开」。
/// 这条挡的是「以为自己在改草稿、其实已经把定稿卷改了」。
#[test]
fn paper_update_and_scheme_are_read_only_after_finalize() {
    let (mut conn, _dir) = fresh();
    let (paper, _, _) = balanced_paper(&mut conn);
    crate::paper::finalize(&mut conn, &paper.id).expect("配平后能定稿");

    let error = crate::paper::update(
        &mut conn,
        &paper.id,
        &crate::paper::PaperPatch {
            title: Some("偷偷改个名".to_string()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("只读"), "{error}");
    assert!(error.contains("重新打开"), "{error}");

    let error = crate::paper::apply_scheme(&mut conn, &paper.id, "GENERAL_100").unwrap_err();
    assert!(error.contains("定稿"), "{error}");

    // 重新打开后恢复可编辑。
    crate::paper::set_status(&mut conn, &paper.id, "DRAFT").unwrap();
    let updated = crate::paper::update(
        &mut conn,
        &paper.id,
        &crate::paper::PaperPatch {
            title: Some("重新打开后的卷".to_string()),
            duration_min: Some(100),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(updated.title, "重新打开后的卷");
    assert_eq!(updated.duration_min, 100);
    assert_eq!(updated.status, "DRAFT");
    assert!(updated.finalized_at.is_none(), "重新打开要清掉定稿时间");
}

/// 改总分是一次「全部重算」的危险动作：没有确认标记就拒绝，0 分也拒绝。
#[test]
fn paper_set_total_requires_confirm_and_positive_score() {
    let (mut conn, _dir) = fresh();
    let (paper, _, _) = balanced_paper(&mut conn);

    let error = crate::paper::set_total(&mut conn, &paper.id, 120.0, false).unwrap_err();
    assert!(error.contains("确认"), "{error}");
    let error = crate::paper::set_total(&mut conn, &paper.id, 0.0, true).unwrap_err();
    assert!(error.contains("大于 0"), "{error}");

    let updated = crate::paper::set_total(&mut conn, &paper.id, 120.0, true).unwrap();
    assert_eq!(updated.total_score, 120.0);
    // 改总分不悄悄改题分，只把差额暴露出来。
    assert_eq!(updated.assigned_score, 100.0);
    let check = crate::paper::score_check(&conn, &paper.id).unwrap();
    assert_eq!(check.delta, 20.0);
    assert!(!check.balanced);
}

/// 套方案两条路：空卷按方案补齐大题结构；已有大题按「题型优先、标题兜底」对齐分值口径。
#[test]
fn paper_apply_scheme_rebuilds_empty_and_aligns_existing() {
    let (mut conn, _dir) = fresh();

    // 路一：把自定义卷唯一的大题删掉，套新高考Ⅰ卷 → 4 个大题、总分 150。
    let empty = paper_create_custom(&mut conn, "空卷");
    let section = empty.sections[0].id.clone();
    crate::paper::delete_section(&mut conn, &empty.id, &section).unwrap();
    let rebuilt = crate::paper::apply_scheme(&mut conn, &empty.id, "XGK1_MATH").unwrap();
    assert_eq!(rebuilt.total_score, 150.0);
    assert_eq!(rebuilt.sections.len(), 4);
    assert_eq!(rebuilt.sections[1].kind, "MULTI");
    assert_eq!(rebuilt.sections[3].score_mode, "PER_ITEM");

    // 路二：已有单选题大题（每题 50 分、2 题）套通用 100 分卷 → 单位分回到 4 分。
    let kp = first_knowledge_id(&conn);
    let id1 = repo::save(&mut conn, &complete_choice_input(&kp, "PUBLISHED")).unwrap();
    let id2 = repo::save(&mut conn, &complete_choice_input(&kp, "PUBLISHED")).unwrap();
    let paper = paper_create_custom(&mut conn, "对齐卷");
    let section = paper.sections[0].id.clone();
    crate::paper::save_section(
        &mut conn,
        &paper.id,
        &crate::paper::PaperSectionInput {
            id: Some(section.clone()),
            title: "一、单项选择题".to_string(),
            kind: "SINGLE".to_string(),
            score_mode: "UNIFORM".to_string(),
            unit_score: Some(50.0),
            ..Default::default()
        },
    )
    .unwrap();
    crate::paper::add_questions(&mut conn, &paper.id, &section, &[id1, id2]).unwrap();
    let aligned = crate::paper::apply_scheme(&mut conn, &paper.id, "GENERAL_100").unwrap();
    assert_eq!(aligned.total_score, 100.0);
    assert_eq!(aligned.sections[0].unit_score, 4.0);
    assert_eq!(aligned.assigned_score, 8.0);
    assert_eq!(aligned.sections[0].questions[0].score, 4.0);

    // 不存在的方案只报错，不动数据。
    let error = crate::paper::apply_scheme(&mut conn, &paper.id, "NOPE").unwrap_err();
    assert!(error.contains("不存在"), "{error}");
    assert_eq!(
        crate::paper::get(&conn, &paper.id).unwrap().total_score,
        100.0
    );
}

/// 造一道已入库的题：题型 / 难度可指定，SOLVED 不带选项。
fn published_question(
    conn: &mut Connection,
    knowledge_id: &str,
    stem: &str,
    kind: &str,
    difficulty: i64,
) -> String {
    let mut input = complete_choice_input(knowledge_id, "PUBLISHED");
    input.stem = stem.to_string();
    input.kind = kind.to_string();
    input.difficulty = difficulty;
    if kind == "SOLVED" {
        input.options = vec![];
        input.answer = "解：$x=1$".to_string();
    }
    repo::save(conn, &input).unwrap()
}

fn blueprint_section(kind: &str, count: i64, unit: f64) -> crate::paper::BlueprintSection {
    crate::paper::BlueprintSection {
        kind: kind.to_string(),
        count,
        unit_score: unit,
        score_mode: "UNIFORM".to_string(),
        ..Default::default()
    }
}

/// 蓝图先算分再抽题：Σ(题数×分值) 与总分不一致时直接拒绝，不去动题库。
#[test]
fn paper_blueprint_rejects_unbalanced_plan() {
    let (mut conn, _dir) = fresh();
    let paper = paper_create_custom(&mut conn, "蓝图分值校验");
    let blueprint = crate::paper::Blueprint {
        total_score: 100.0,
        sections: vec![blueprint_section("SINGLE", 3, 10.0)],
        ..Default::default()
    };
    let error = crate::paper::blueprint_fill(&mut conn, &paper.id, &blueprint, true).unwrap_err();
    assert!(error.contains("不平"), "{error}");
    assert!(error.contains("还差"), "差 70 分要报「还差」：{error}");
    assert_eq!(
        crate::paper::get(&conn, &paper.id).unwrap().question_count,
        0,
        "算分没过就不能动卷子"
    );
}

/// 缺题时整次回滚：不留半份卷子，缺口逐条报出来（哪道大题、哪个难度、差几题）。
#[test]
fn paper_blueprint_gaps_roll_back_the_whole_fill() {
    let (mut conn, _dir) = fresh();
    let kp = first_knowledge_id(&conn);
    published_question(&mut conn, &kp, "蓝图缺口：唯一一道单选题", "SINGLE", 2);
    let paper = paper_create_custom(&mut conn, "缺口卷");

    let blueprint = crate::paper::Blueprint {
        total_score: 100.0,
        sections: vec![blueprint_section("SINGLE", 2, 50.0)],
        ..Default::default()
    };
    let result = crate::paper::blueprint_fill(&mut conn, &paper.id, &blueprint, true).unwrap();
    assert!(!result.report.filled, "缺 1 题就不能算组卷成功");
    assert_eq!(result.report.gaps.len(), 1);
    assert_eq!(result.report.gaps[0].need, 2);
    assert_eq!(result.report.gaps[0].got, 1);
    assert_eq!(result.report.added, 0);
    assert!(
        result.report.message.contains("没有改动"),
        "{}",
        result.report.message
    );
    let paper = crate::paper::get(&conn, &paper.id).unwrap();
    assert_eq!(paper.question_count, 0, "回滚后不能留半份卷子");
    assert_eq!(paper.total_score, 100.0, "总分也不该被蓝图改掉");
}

/// 按难度档抽题：每档取够、卷内不重题、分值自动配平；「再组一套」不会抽到同一批题。
#[test]
fn paper_blueprint_fills_by_difficulty_then_again_avoids_reuse() {
    let (mut conn, _dir) = fresh();
    let kp = first_knowledge_id(&conn);
    for index in 0..2 {
        published_question(&mut conn, &kp, &format!("蓝图易题 {index}"), "SINGLE", 1);
    }
    for index in 0..4 {
        published_question(&mut conn, &kp, &format!("蓝图难题 {index}"), "SINGLE", 3);
    }
    let paper = paper_create_custom(&mut conn, "按蓝图组卷");

    let mut difficulty = std::collections::BTreeMap::new();
    difficulty.insert("1".to_string(), 1);
    difficulty.insert("3".to_string(), 2);
    let blueprint = crate::paper::Blueprint {
        total_score: 60.0,
        sections: vec![crate::paper::BlueprintSection {
            kind: "SINGLE".to_string(),
            count: 3,
            unit_score: 20.0,
            score_mode: "UNIFORM".to_string(),
            difficulty,
            ..Default::default()
        }],
        ..Default::default()
    };
    let filled = crate::paper::blueprint_fill(&mut conn, &paper.id, &blueprint, true).unwrap();
    assert!(filled.report.filled, "{}", filled.report.message);
    assert_eq!(filled.report.added, 3);
    assert_eq!(filled.paper.question_count, 3);
    assert_eq!(filled.paper.total_score, 60.0, "蓝图总分落到卷子上");
    assert_eq!(
        filled.paper.assigned_score, 60.0,
        "每题 20 分 × 3 题要自动配平"
    );

    let first_ids: Vec<String> = filled.paper.sections[0]
        .questions
        .iter()
        .filter_map(|item| item.question_id.clone())
        .collect();
    assert_eq!(first_ids.len(), 3);
    // 难度档要真的生效：1 道易题 + 2 道难题。
    let snapshot_diff = |id: &str| -> i64 {
        let conn = &conn;
        conn.query_row(
            "select difficulty from question where id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
    };
    let easy = first_ids.iter().filter(|id| snapshot_diff(id) == 1).count();
    assert_eq!(easy, 1, "难度 1 只该来 1 道");

    // 再组一套：新卷、题不同。
    let again = crate::paper::blueprint_again(&mut conn, &paper.id).unwrap();
    assert!(again.report.filled, "{}", again.report.message);
    assert_ne!(again.paper.id, paper.id);
    let second_ids: Vec<String> = again.paper.sections[0]
        .questions
        .iter()
        .filter_map(|item| item.question_id.clone())
        .collect();
    for id in &second_ids {
        assert!(!first_ids.contains(id), "B 卷不该和 A 卷撞题：{id}");
    }
}

/// 换题保留题号与分值；撤销把上一题（含快照）原样放回。
#[test]
fn paper_replace_question_keeps_score_and_undo_restores() {
    let (mut conn, _dir) = fresh();
    let (paper, original, _) = balanced_paper(&mut conn);
    let kp = first_knowledge_id(&conn);
    let replacement = published_question(&mut conn, &kp, "换题候选", "SINGLE", 2);
    let section = paper.sections[0].id.clone();

    let candidates = crate::paper::replace_candidates(
        &conn,
        &paper.id,
        &crate::paper::CandidateQuery {
            section_id: section.clone(),
            ord: 0,
            ..Default::default()
        },
    )
    .unwrap();
    let ids: Vec<&str> = candidates.iter().map(|item| item.id.as_str()).collect();
    assert!(ids.contains(&replacement.as_str()), "候选里应有这道题");
    assert!(
        !ids.contains(&original.as_str()),
        "卷内已用的题不能出现在候选里"
    );

    let replaced =
        crate::paper::replace_question(&mut conn, &paper.id, &section, 0, &replacement).unwrap();
    assert_eq!(
        replaced.sections[0].questions[0].question_id.as_deref(),
        Some(replacement.as_str())
    );
    assert_eq!(
        replaced.sections[0].questions[0].score, 50.0,
        "换题不改分值"
    );
    assert!(
        replaced.sections[0].questions[0]
            .flags
            .get("prev")
            .is_some(),
        "要记下上一题才能撤销"
    );

    let restored = crate::paper::undo_replace(&mut conn, &paper.id, &section, 0).unwrap();
    assert_eq!(
        restored.sections[0].questions[0].question_id.as_deref(),
        Some(original.as_str())
    );
    assert_eq!(restored.sections[0].questions[0].score, 50.0);
    // 撤销后再撤销应该报错，而不是静默成功。
    assert!(crate::paper::undo_replace(&mut conn, &paper.id, &section, 0).is_err());
}

/// 易/中/难三档配额（老师更常说这三个字）：每档各取 1 题，档位真的生效；
/// 配额合计与题数不一致时直接拒绝。
#[test]
fn paper_blueprint_bands_pick_easy_medium_hard() {
    let (mut conn, _dir) = fresh();
    let kp = first_knowledge_id(&conn);
    for (level, count) in [(1, 2), (3, 2), (5, 2)] {
        for index in 0..count {
            published_question(
                &mut conn,
                &kp,
                &format!("难度{level}第{index}题"),
                "SINGLE",
                level,
            );
        }
    }
    let paper = paper_create_custom(&mut conn, "易中难蓝图");

    let mut bands = std::collections::BTreeMap::new();
    bands.insert("easy".to_string(), 1);
    bands.insert("medium".to_string(), 1);
    bands.insert("hard".to_string(), 1);
    let blueprint = crate::paper::Blueprint {
        total_score: 60.0,
        sections: vec![crate::paper::BlueprintSection {
            kind: "SINGLE".to_string(),
            count: 3,
            unit_score: 20.0,
            score_mode: "UNIFORM".to_string(),
            bands: bands.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let filled = crate::paper::blueprint_fill(&mut conn, &paper.id, &blueprint, true).unwrap();
    assert!(filled.report.filled, "{}", filled.report.message);
    let difficulties: Vec<i64> = filled.paper.sections[0]
        .questions
        .iter()
        .map(|item| {
            item.snapshot
                .get("difficulty")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        })
        .collect();
    assert_eq!(difficulties.len(), 3);
    assert!(difficulties.contains(&1), "易档：{difficulties:?}");
    assert!(difficulties.contains(&3), "中档：{difficulties:?}");
    assert!(difficulties.contains(&5), "难档：{difficulties:?}");

    // 配额合计 4 题但题数写 3 → 明确报错，不去抽题。
    let mut bad = bands.clone();
    bad.insert("hard".to_string(), 2);
    let blueprint = crate::paper::Blueprint {
        total_score: 60.0,
        sections: vec![crate::paper::BlueprintSection {
            kind: "SINGLE".to_string(),
            count: 3,
            unit_score: 20.0,
            score_mode: "UNIFORM".to_string(),
            bands: bad,
            ..Default::default()
        }],
        ..Default::default()
    };
    let error = crate::paper::blueprint_fill(&mut conn, &paper.id, &blueprint, true).unwrap_err();
    assert!(error.contains("难度配额合计"), "{error}");
}

/// 学生维度的学情名单：薄弱的排前面、没交的在最后，个人页能给出班内位置。
/// 这条链路的重点是「不先选班级也能看到一个具体的人」，所以断言都落在个人身上。
#[test]
fn insights_roster_lists_students_and_ranks_inside_class() {
    let (mut conn, _dir) = fresh();
    let question = single_choice(&mut conn, "1+1 等于几？", "B");

    crate::teaching::student_import_commit(
        &mut conn,
        "姓名,年级\n甲,七年级\n乙,七年级\n丙,七年级\n",
    )
    .expect("导入名单");
    let class = crate::teaching::class_save(
        &conn,
        &crate::teaching::ClassInput {
            id: None,
            name: "七年级一班".into(),
            subject_id: None,
            term: "2026 秋".into(),
        },
    )
    .expect("建班");
    let ids: Vec<String> = crate::teaching::student_list(&conn, None, None)
        .expect("学生列表")
        .iter()
        .map(|student| student.id.clone())
        .collect();
    crate::teaching::class_add_members(&mut conn, &class, &ids).expect("加成员");

    let now = crate::text::now();
    conn.execute(
        "insert into local_assignment (id, class_id, title, type, description, status, created_at, updated_at)
         values ('asg-roster', ?1, '第一次作业', 'PRACTICE', '', 'PUBLISHED', ?2, ?2)",
        rusqlite::params![class, now],
    )
    .expect("建作业");
    conn.execute(
        "insert into local_assignment_question (assignment_id, question_id, ord, score)
         values ('asg-roster', ?1, 0, 5)",
        [&question],
    )
    .expect("作业挂题");

    let roster = crate::teaching::student_list(&conn, None, None).unwrap();
    let id_of = |name: &str| {
        roster
            .iter()
            .find(|student| student.name == name)
            .unwrap()
            .id
            .clone()
    };
    let add_attempt = |student: &str, correct: i64, sub: &str| {
        conn.execute(
            "insert into local_attempt
                (id, assignment_id, question_id, student_id, submission_id, answer,
                 submitted_at, correct, score, max_score)
             values (?1, 'asg-roster', ?2, ?3, ?4, 'B', ?5, ?6, ?7, 5)",
            rusqlite::params![
                format!("att-{student}-{sub}"),
                question,
                student,
                sub,
                now,
                correct,
                if correct == 1 { 5.0 } else { 0.0 }
            ],
        )
        .expect("插作答");
    };
    add_attempt(&id_of("甲"), 1, "s1");
    add_attempt(&id_of("乙"), 0, "s1");

    let roster = crate::insights::roster(&conn, None, None).expect("学生名单");
    assert_eq!(roster.students.len(), 3);
    assert_eq!(roster.students[0].name, "乙", "薄弱的排前面");
    assert_eq!(roster.students[2].name, "丙", "没交作业的排最后");
    assert_eq!(roster.students[2].answered, 0);
    assert_eq!(roster.summary.total, 3);
    assert_eq!(roster.summary.active, 2);
    assert_eq!(roster.summary.weak, 1);
    assert_eq!(roster.summary.mastered, 0);
    assert_eq!(roster.summary.idle, 0);
    assert_eq!(roster.students[0].weak.len(), 1, "薄弱知识点带出来");

    let jia = roster.students.iter().find(|row| row.name == "甲").unwrap();
    assert_eq!(jia.mastery, 80, "难度 1 全对 = 80");
    assert_eq!(jia.class_names, vec!["七年级一班".to_string()]);

    // 个人页：名次、班均、薄弱点都要能对上，不能只有一堆原始作答。
    let detail = crate::insights::student_insight(&conn, &jia.id).expect("个人学情");
    assert_eq!(detail.rank, Some(1));
    assert_eq!(detail.class_size, Some(2));
    assert_eq!(detail.class_avg, Some(42), "(80 + 5) / 2");
    assert!(detail.weak.is_empty(), "80 分的知识点不算薄弱");

    let yi = crate::insights::student_insight(&conn, &id_of("乙")).expect("个人学情");
    assert_eq!(yi.overall, 5);
    assert_eq!(yi.rank, Some(2));
    assert_eq!(yi.weak.len(), 1);

    // 按班 + 关键字筛选，名单只留一个人。
    let only = crate::insights::roster(&conn, Some(&class), Some("甲")).expect("筛选名单");
    assert_eq!(only.students.len(), 1);
    assert_eq!(only.students[0].name, "甲");
}

/// 学号：导入能认表头、能给空号自动编号、重号当场拦下，不静默合并两个学生。
#[test]
fn student_numbers_are_imported_auto_assigned_and_guarded() {
    let (mut conn, _dir) = fresh();
    let text = "学号,姓名,年级\n2026001,甲,七年级\n,乙,七年级\n2026001,丙,七年级\n";
    let preview = crate::teaching::student_import_preview(&conn, text).expect("预览");
    assert_eq!(preview.total, 3);
    assert_eq!(preview.duplicates, 0);
    assert_eq!(preview.conflicts, 1, "同一份名单里重号要标出来");
    assert_eq!(preview.rows[0].student_no, "2026001");

    let result = crate::teaching::student_import_commit(&mut conn, text).expect("导入");
    assert_eq!(result.created, 2);
    assert_eq!(
        result.errors.len(),
        1,
        "重号那行要报错，而不是建一个同号学生"
    );

    let roster = crate::teaching::student_list(&conn, None, None).expect("名单");
    let no_of = |name: &str| {
        roster
            .iter()
            .find(|student| student.name == name)
            .unwrap()
            .student_no
            .clone()
    };
    assert_eq!(no_of("甲"), "2026001");
    assert_eq!(no_of("乙"), "2026002", "空号按已用最大号往后排");
    assert!(!roster.iter().any(|student| student.name == "丙"));

    // 老格式（姓名, 年级, 备注）不能被误判成学号。
    let legacy = crate::teaching::student_import_commit(&mut conn, "丁,七年级,转学生\n")
        .expect("老格式导入");
    assert_eq!(legacy.created, 1);
    let ding = crate::teaching::student_list(&conn, Some("丁"), None)
        .expect("按姓名搜")
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(ding.grade, "七年级");
    assert_eq!(ding.note, "转学生");
    assert!(ding.student_no.parse::<i64>().is_ok(), "自动编号应是数字");

    // 手工改学号：撞号要拒绝，空着表示不动。
    let err = crate::teaching::student_save(
        &conn,
        &crate::teaching::StudentInput {
            id: Some(ding.id.clone()),
            name: ding.name.clone(),
            student_no: "2026001".into(),
            grade: ding.grade.clone(),
            subject_ids: vec![],
            note: ding.note.clone(),
            status: None,
        },
    )
    .expect_err("撞号应拒绝");
    assert!(err.contains("已经被别的学生用了"), "{err}");
}

/// 成绩：缺考不拉低平均、并列同名次、及格/优秀线与分数段都对得上。
#[test]
fn exam_scores_rank_and_stats_exclude_absentees() {
    let (mut conn, _dir) = fresh();
    crate::teaching::student_import_commit(
        &mut conn,
        "学号,姓名,年级\n1001,甲,七年级\n1002,乙,七年级\n1003,丙,七年级\n",
    )
    .expect("导入");
    let class = crate::teaching::class_save(
        &conn,
        &crate::teaching::ClassInput {
            id: None,
            name: "七年级二班".into(),
            subject_id: None,
            term: "2026 秋".into(),
        },
    )
    .expect("建班");
    let ids: Vec<String> = crate::teaching::student_list(&conn, None, None)
        .expect("名单")
        .iter()
        .map(|student| student.id.clone())
        .collect();
    crate::teaching::class_add_members(&mut conn, &class, &ids).expect("加成员");
    let id_map: std::collections::HashMap<String, String> =
        crate::teaching::student_list(&conn, None, None)
            .unwrap()
            .into_iter()
            .map(|student| (student.name.clone(), student.id.clone()))
            .collect();
    let id_of = |name: &str| id_map.get(name).cloned().unwrap();

    let exam = crate::scores::exam_save(
        &conn,
        &crate::scores::ExamInput {
            id: None,
            class_id: class.clone(),
            title: "第一次月考".into(),
            kind: "EXAM".into(),
            full_score: 100.0,
            pass_line: Some(60.0),
            good_line: Some(90.0),
            taken_at: Some("2026-11-10".into()),
            note: String::new(),
        },
    )
    .expect("建考试");

    let saved = crate::scores::exam_scores_save(
        &mut conn,
        &exam,
        &[
            crate::scores::ScoreInput {
                student_id: id_of("甲"),
                score: Some(92.0),
                note: String::new(),
            },
            crate::scores::ScoreInput {
                student_id: id_of("乙"),
                score: Some(88.0),
                note: String::new(),
            },
            crate::scores::ScoreInput {
                student_id: id_of("丙"),
                score: None,
                note: "缺考".into(),
            },
        ],
    )
    .expect("登分");
    assert_eq!(saved, 3);

    let sheet = crate::scores::exam_sheet(&conn, &exam).expect("成绩表");
    assert_eq!(sheet.rows.len(), 3);
    assert_eq!(sheet.stats.entered, 2);
    assert_eq!(sheet.stats.missing, 1);
    assert_eq!(sheet.stats.average, Some(90.0), "缺考不计入平均");
    assert_eq!(sheet.stats.highest, Some(92.0));
    assert_eq!(sheet.stats.lowest, Some(88.0));
    assert_eq!(sheet.stats.pass, 2);
    assert_eq!(sheet.stats.good, 1, "90 是优秀线，88 不算");
    assert_eq!(sheet.stats.good_rate, Some(50.0));
    let jia = sheet.rows.iter().find(|row| row.name == "甲").unwrap();
    let bing = sheet.rows.iter().find(|row| row.name == "丙").unwrap();
    assert_eq!(jia.rank, Some(1));
    assert_eq!(jia.vs_average, Some(2.0));
    assert_eq!(bing.rank, None, "缺考没有名次");
    let top = sheet
        .dist
        .iter()
        .find(|bucket| bucket.label == "90+")
        .unwrap();
    assert_eq!(top.count, 1);

    let exams = crate::scores::student_exams(&conn, &id_of("乙"), 12).expect("个人考试");
    assert_eq!(exams.len(), 1);
    assert_eq!(exams[0].rank, Some(2));
    assert_eq!(exams[0].class_size, 2);
    assert_eq!(exams[0].class_avg, Some(90.0));

    // 超出满分不落库，避免把 150 分制录进 100 分卷。
    let bad = crate::scores::exam_scores_save(
        &mut conn,
        &exam,
        &[crate::scores::ScoreInput {
            student_id: id_of("甲"),
            score: Some(120.0),
            note: String::new(),
        }],
    );
    assert!(bad.is_err());
}

/// 学习安排：没有模型也要给出可执行的周计划，且指向最薄弱的那个知识点。
#[test]
fn advice_falls_back_to_rules_and_targets_weakest_point() {
    let (mut conn, _dir) = fresh();
    let question = single_choice(&mut conn, "3 + 5 等于几？", "B");
    let kp_name: String = conn
        .query_row(
            "select k.name from knowledge_point k
               join question_knowledge qk on qk.knowledge_point_id = k.id
              where qk.question_id = ?1 limit 1",
            [&question],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "知识点".into());

    crate::teaching::student_import_commit(&mut conn, "姓名,年级\n甲,七年级\n乙,七年级\n")
        .expect("导入");
    let class = crate::teaching::class_save(
        &conn,
        &crate::teaching::ClassInput {
            id: None,
            name: "七年级三班".into(),
            subject_id: None,
            term: "2026 秋".into(),
        },
    )
    .expect("建班");
    let ids: Vec<String> = crate::teaching::student_list(&conn, None, None)
        .expect("名单")
        .iter()
        .map(|student| student.id.clone())
        .collect();
    crate::teaching::class_add_members(&mut conn, &class, &ids).expect("加成员");
    let id_of = |name: &str| {
        crate::teaching::student_list(&conn, None, None)
            .unwrap()
            .into_iter()
            .find(|student| student.name == name)
            .unwrap()
            .id
    };

    let now = crate::text::now();
    conn.execute(
        "insert into local_assignment (id, class_id, title, type, description, status, created_at, updated_at)
         values ('asg-advice', ?1, '练习一', 'PRACTICE', '', 'PUBLISHED', ?2, ?2)",
        rusqlite::params![class, now],
    )
    .expect("建作业");
    conn.execute(
        "insert into local_assignment_question (assignment_id, question_id, ord, score)
         values ('asg-advice', ?1, 0, 5)",
        [&question],
    )
    .expect("挂题");
    for (student, correct) in [(id_of("甲"), 1), (id_of("乙"), 0)] {
        conn.execute(
            "insert into local_attempt
                (id, assignment_id, question_id, student_id, submission_id, answer,
                 submitted_at, correct, score, max_score)
             values (?1, 'asg-advice', ?2, ?3, ?4, 'B', ?5, ?6, ?7, 5)",
            rusqlite::params![
                format!("att-{student}"),
                question,
                student,
                format!("sub-{student}"),
                now,
                correct,
                if correct == 1 { 5.0 } else { 0.0 }
            ],
        )
        .expect("插作答");
    }

    // 数据差的那个：结论要指向薄弱点，计划要能直接照着做。
    let weak = crate::advice::get(&conn, &id_of("乙")).expect("规则版建议");
    assert_eq!(weak.source, "RULE");
    assert_eq!(weak.level, "CRITICAL");
    assert_eq!(weak.plan.days.len(), 7);
    assert!(weak.plan.total_minutes > 0);
    assert_eq!(weak.plan.focus.first(), Some(&kp_name));
    assert!(weak.headline.contains("分"), "{}", weak.headline);
    assert!(!weak.issues.is_empty());
    assert!(!weak.facts.is_empty(), "结论要给依据");

    // 没配模型：generate 不能报错，要退回规则版并且落库。
    let client = reqwest::blocking::Client::new();
    let fallback =
        crate::advice::generate(&conn, &client, &id_of("乙"), true).expect("无模型也能生成");
    assert_eq!(fallback.source, "RULE");
    let cached = crate::advice::get(&conn, &id_of("乙")).expect("读缓存");
    assert_eq!(cached.source, "RULE");
    assert!(!cached.stale);

    // 做对的那个：掌握度 80，不算薄弱，建议应该是「保持节奏」。
    let solid = crate::advice::get(&conn, &id_of("甲")).expect("规则版建议");
    assert_eq!(solid.level, "STEADY");
    assert!(solid.issues.iter().all(|issue| issue.kp != kp_name));
}

/// 合并趋势：作业与考试统一换成百分制，按日期正序；缺考不进时间轴。
#[test]
fn progress_mixes_assignments_and_exams_on_percent_scale() {
    fn add_assignment(
        conn: &Connection,
        class: &str,
        question: &str,
        student: &str,
        id: &str,
        title: &str,
        at: &str,
        score: f64,
        max: f64,
    ) {
        conn.execute(
            "insert into local_assignment (id, class_id, title, type, description, status, created_at, updated_at)
             values (?1, ?2, ?3, 'PRACTICE', '', 'PUBLISHED', ?4, ?4)",
            rusqlite::params![id, class, title, at],
        )
        .expect("建作业");
        conn.execute(
            "insert into local_attempt
                (id, assignment_id, question_id, student_id, submission_id, answer,
                 submitted_at, correct, score, max_score)
             values (?1, ?1, ?2, ?3, ?4, 'B', ?5, 1, ?6, ?7)",
            rusqlite::params![id, question, student, format!("sub-{id}"), at, score, max],
        )
        .expect("插作答");
    }

    let (mut conn, _dir) = fresh();
    crate::teaching::student_import_commit(
        &mut conn,
        "学号,姓名,年级\n1001,甲,七年级\n1002,乙,七年级\n",
    )
    .expect("导入");
    let class = crate::teaching::class_save(
        &conn,
        &crate::teaching::ClassInput {
            id: None,
            name: "七年级四班".into(),
            subject_id: None,
            term: "2026 秋".into(),
        },
    )
    .expect("建班");
    let ids: Vec<String> = crate::teaching::student_list(&conn, None, None)
        .expect("名单")
        .iter()
        .map(|student| student.id.clone())
        .collect();
    crate::teaching::class_add_members(&mut conn, &class, &ids).expect("加成员");
    let id_map: std::collections::HashMap<String, String> =
        crate::teaching::student_list(&conn, None, None)
            .unwrap()
            .into_iter()
            .map(|student| (student.name.clone(), student.id.clone()))
            .collect();
    let jia = id_map["甲"].clone();
    let yi = id_map["乙"].clone();
    let question = single_choice(&mut conn, "1 + 1 等于几？", "B");

    // 两次作业：2026-11-01 得 60%，2026-11-05 得 80%。
    add_assignment(
        &conn,
        &class,
        &question,
        &jia,
        "asg-p1",
        "练习一",
        "2026-11-01T10:00:00+08:00",
        3.0,
        5.0,
    );
    add_assignment(
        &conn,
        &class,
        &question,
        &jia,
        "asg-p2",
        "练习二",
        "2026-11-05T10:00:00+08:00",
        8.0,
        10.0,
    );

    // 一次 150 分制考试：120 / 150 = 80%；乙缺考，不进时间轴也不占班均。
    let exam = crate::scores::exam_save(
        &conn,
        &crate::scores::ExamInput {
            id: None,
            class_id: class.clone(),
            title: "期中考试".into(),
            kind: "EXAM".into(),
            full_score: 150.0,
            pass_line: None,
            good_line: None,
            taken_at: Some("2026-11-15".into()),
            note: String::new(),
        },
    )
    .expect("建考试");
    crate::scores::exam_scores_save(
        &mut conn,
        &exam,
        &[
            crate::scores::ScoreInput {
                student_id: jia.clone(),
                score: Some(120.0),
                note: String::new(),
            },
            crate::scores::ScoreInput {
                student_id: yi,
                score: None,
                note: "缺考".into(),
            },
        ],
    )
    .expect("登分");

    let progress = crate::progress::student_progress(&conn, &jia, 12).expect("合并趋势");
    assert_eq!(progress.points.len(), 3, "两次作业 + 一次考试，缺考不占位");
    assert_eq!(progress.assignments, 2);
    assert_eq!(progress.exams, 1);
    let ats: Vec<&str> = progress
        .points
        .iter()
        .map(|point| point.at.as_str())
        .collect();
    assert_eq!(
        ats,
        vec!["2026-11-01", "2026-11-05", "2026-11-15"],
        "按日期正序"
    );
    assert_eq!(progress.points[0].percent, 60.0);
    assert_eq!(progress.points[1].percent, 80.0);
    let mid = &progress.points[2];
    assert_eq!(mid.percent, 80.0, "150 分制换成百分制");
    assert_eq!(mid.kind, "EXAM");
    assert_eq!(mid.rank, Some(1));
    assert_eq!(mid.class_avg, Some(80.0), "缺考不拉低班均");
    assert_eq!(progress.assignment_avg, Some(70.0));
    assert_eq!(progress.exam_avg, Some(80.0));

    // 只有三次，还不足以比「最近三次 vs 之前三次」。
    assert_eq!(progress.delta, None);
    assert!(progress.note.contains("2 次作业"), "{}", progress.note);

    // 再补一次 100% 的作业：四次之后就能算出变化方向。
    add_assignment(
        &conn,
        &class,
        &question,
        &jia,
        "asg-p3",
        "练习三",
        "2026-11-17T10:00:00+08:00",
        10.0,
        10.0,
    );
    let progress = crate::progress::student_progress(&conn, &jia, 12).expect("合并趋势");
    assert_eq!(progress.points.last().unwrap().percent, 100.0);
    assert_eq!(progress.delta, Some(26.7), "最近三次 100/80/80 对之前 60");
}

// ---------------------------------------------------------------------------
// 课件批量导入（0014）：任务表、无 AI 草稿、原稿保留、判重与逐项重试
// ---------------------------------------------------------------------------

#[test]
fn courseware_import_migration_creates_table_and_indexes() {
    let (conn, _dir) = fresh();
    let found: i64 = conn
        .query_row(
            "select count(*) from sqlite_master where type = 'table' and name = 'local_courseware_import'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(found, 1);
    for column in [
        "id",
        "batch_id",
        "source_path",
        "managed_path",
        "sha256",
        "title",
        "status",
        "mode",
        "use_ai",
        "extracted_text",
        "draft_json",
        "raw_ai_json",
        "warnings_json",
        "error",
        "courseware_id",
        "attempts",
        "created_at",
        "updated_at",
    ] {
        let exists: i64 = conn
            .query_row(
                "select count(*) from pragma_table_info('local_courseware_import') where name = ?1",
                [column],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "缺字段 {column}");
    }
    for index in [
        "ix_local_courseware_import_batch",
        "ix_local_courseware_import_status",
        "ix_local_courseware_import_sha256",
    ] {
        let exists: i64 = conn
            .query_row(
                "select count(*) from sqlite_master where type = 'index' and name = ?1",
                [index],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "缺索引 {index}");
    }
}

#[test]
fn courseware_import_without_ai_preserves_original_and_retries_independently() {
    use crate::courseware::ingest::{self, ImportStartInput};

    let (conn, dir) = fresh();
    let good = dir.join("牛顿 第一定律.md");
    let same = dir.join("同内容.txt");
    let bad = dir.join("空讲义.txt");
    let original_text = "第一段：物体保持静止或匀速直线运动。\n\n第二段：原文完整保留，不补答案。";
    std::fs::write(&good, original_text).unwrap();
    std::fs::write(&same, original_text).unwrap();
    std::fs::write(&bad, "").unwrap();
    let input = ImportStartInput {
        paths: vec![
            good.to_string_lossy().to_string(),
            same.to_string_lossy().to_string(),
            bad.to_string_lossy().to_string(),
        ],
        mode: String::new(),
        subject_id: None,
        subject: "物理".into(),
        grade: "高一".into(),
        use_ai: false,
    };
    let started = ingest::start(&conn, &dir, &input).expect("创建导入批次");
    assert_eq!(started.len(), 3);
    assert!(started.iter().all(|item| item.status == "QUEUED"));
    assert!(started.iter().all(|item| item.managed_path.is_empty()));
    assert_eq!(started[0].mode, "FORMAT_ONLY");
    let batch_id = started[0].batch_id.clone();
    let http = reqwest::blocking::Client::new();
    let rows = ingest::process_batch(&dir.join("neuralkb.db"), &dir, &http, &batch_id)
        .expect("批处理单项失败不中断");
    assert_eq!(
        rows.iter()
            .map(|item| item.status.as_str())
            .collect::<Vec<_>>(),
        vec!["REVIEW", "DUPLICATE", "FAILED"]
    );
    assert!(rows[2].error.contains("空"), "{}", rows[2].error);

    let first = &rows[0];
    assert_eq!(first.extracted_text, original_text);
    assert_eq!(first.sha256.len(), 64);
    assert!(!PathBuf::from(&first.managed_path).is_absolute());
    assert!(!first.managed_path.contains(".."));
    let managed = dir.join("assets").join(&first.managed_path);
    let managed_bytes = std::fs::read(&managed).expect("受管原稿存在");
    assert_eq!(managed_bytes, original_text.as_bytes());
    let draft = first.draft.as_ref().expect("本地草稿");
    assert_eq!(draft.title, "牛顿 第一定律");
    assert_eq!(draft.points.len(), 1);
    assert_eq!(draft.points[0].detail, original_text);
    assert!(draft.examples.is_empty() && draft.knowledge.is_empty());
    assert_eq!(draft.source.original_path, first.managed_path);
    assert_eq!(draft.source.sha256, first.sha256);
    assert_eq!(draft.source.mode, "FORMAT_ONLY");
    assert!(!draft.source.locations.is_empty());

    // 重试读取受管原稿和已抽取文字，原位置即使被改写也不会覆盖受管副本。
    std::fs::write(&good, "原位置的新内容不应进入这次重试").unwrap();
    let queued = ingest::retry(&conn, &first.id, false, true).expect("重试待校对项");
    assert_eq!(queued.status, "QUEUED");
    let retried = ingest::process_one(&dir.join("neuralkb.db"), &dir, &http, &first.id, true)
        .expect("允许重复后重试");
    assert_eq!(retried.status, "REVIEW");
    assert_eq!(retried.attempts, first.attempts + 1);
    assert_eq!(retried.extracted_text, original_text);
    assert_eq!(std::fs::read(&managed).unwrap(), managed_bytes);
    assert_eq!(
        retried.draft.as_ref().unwrap().points[0].detail,
        original_text
    );

    assert!(ingest::retry(&conn, &rows[1].id, false, false).is_err());
    let queued = ingest::retry(&conn, &rows[1].id, false, true).expect("重试重复项");
    assert_eq!(queued.status, "QUEUED");
    let duplicate_retried =
        ingest::process_one(&dir.join("neuralkb.db"), &dir, &http, &rows[1].id, true)
            .expect("显式允许重复可生成新草稿");
    assert_eq!(duplicate_retried.status, "REVIEW");

    // 空文件失败不影响其他项；补上内容后可单独重试。
    std::fs::write(&bad, "后来补上的讲义正文").unwrap();
    ingest::retry(&conn, &rows[2].id, false, false).expect("重试失败项");
    let recovered = ingest::process_one(&dir.join("neuralkb.db"), &dir, &http, &rows[2].id, false)
        .expect("失败项独立恢复");
    assert_eq!(recovered.status, "REVIEW");
    assert_eq!(recovered.extracted_text, "后来补上的讲义正文");

    // 保存校对时强制恢复受管来源；内容闸失败仍留 REVIEW，补齐后可统一入库。
    let mut corrected = retried.draft.clone().unwrap();
    corrected.description = "教师校对说明".into();
    corrected.source.original_path = "/tmp/不应允许的来源.md".into();
    let saved = ingest::save_draft(&conn, &first.id, &corrected).expect("保存教师校对");
    assert_eq!(saved.draft.as_ref().unwrap().description, "教师校对说明");
    assert_eq!(
        saved.draft.as_ref().unwrap().source.original_path,
        first.managed_path
    );
    let rejected = ingest::commit(&conn, &dir, &first.id, &corrected).expect("返回内容闸结果");
    assert!(!rejected.ok, "本地草稿默认无知识点，入库前应被拦下");
    let review = ingest::list(&conn, Some(&batch_id)).unwrap();
    let review = review.iter().find(|item| item.id == first.id).unwrap();
    assert_eq!(review.status, "REVIEW");
    assert!(!review.error.is_empty());
    corrected.knowledge = vec!["惯性".into()];
    let built = ingest::commit(&conn, &dir, &first.id, &corrected).expect("校对后入库");
    assert!(built.ok, "{}", built.message);
    let committed = ingest::list(&conn, Some(&batch_id)).unwrap();
    let committed = committed.iter().find(|item| item.id == first.id).unwrap();
    assert_eq!(committed.status, "COMMITTED");
    assert_eq!(committed.courseware_id, built.id);
    assert!(managed.exists(), "入库后仍永久保留受管原稿");

    let _ = std::fs::remove_dir_all(&dir);
}
