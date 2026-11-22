// 本地题库客户端：Tauri 2 + React 19 + SQLite。
// 数据只在本机；只有 AI 与可选同步需要联网。

mod advice;
mod auth;
mod browser;
mod capture;
mod capture_store;
mod center_assets;
mod commands;
mod compose;
mod courseware;
mod db;
mod dictation;
mod diagram;
mod diagram_scene;
mod export;
mod filein;
mod grading;
mod import;
mod insights;
mod jobs;
mod llm;
mod models;
mod ocr;
mod overlay;
mod paper;
mod perm;
mod photo_upload_server;
mod pinyin;
mod progress;
mod repo;
mod review;
mod scores;
mod screen;
mod smart;
mod stats;
mod state;
mod storage;
mod sync;
mod teaching;
mod text;

#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod real_capture_probe;

use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let key = shortcut.to_string();
                        if key.trim_end().ends_with('N') {
                            let _ = app.emit("nb://dictation-next", ());
                        } else if key.trim_end().ends_with('R') {
                            let _ = app.emit("nb://dictation-repeat", ());
                        } else {
                            let _ = app.emit("nb://capture", ());
                        }
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(),
        )
        .setup(|app| {
            let built = state::AppState::bootstrap(app.handle()).map_err(|error| {
                Box::<dyn std::error::Error>::from(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    error,
                ))
            })?;
            app.manage(built);

            // 预热置顶选区层：热键路径上只剩挪位 + show，不必等 WKWebView 冷启动。
            if let Err(error) = overlay::prewarm(app.handle()) {
                eprintln!("选区层预热失败（截图仍可用系统选区）：{error}");
            }

            // 全局热键冲突不该让应用起不来：注册失败只记日志，页面里还有按钮。
            if let Err(error) = app.global_shortcut().register("CmdOrCtrl+Shift+O") {
                eprintln!("全局热键 ⌘⇧O 注册失败：{error}");
            }
            // 听写台：下一个 / 重播这条。注册失败只记日志，页面按钮始终可用。
            if let Err(error) = app.global_shortcut().register("CmdOrCtrl+Shift+N") {
                eprintln!("全局热键 ⌘⇧N 注册失败：{error}");
            }
            if let Err(error) = app.global_shortcut().register("CmdOrCtrl+Shift+R") {
                eprintln!("全局热键 ⌘⇧R 注册失败：{error}");
            }

            // 临时冒烟：NB_SMOKE_BROWSER 打开第三方页面窗口，验证远程 IPC 白名单。
            if let Ok(url) = std::env::var("NB_SMOKE_BROWSER") {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    match browser::open(&handle, &url) {
                        Ok(()) => eprintln!("SMOKE: browser opened {url}"),
                        Err(error) => eprintln!("SMOKE: open failed {error}"),
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::auth_status,
            commands::auth_signup,
            commands::auth_login,
            commands::auth_logout,
            commands::auth_change_password,
            commands::auth_update_profile,
            commands::stats,
            commands::setting_get_value,
            commands::setting_set_value,
            commands::wizard_finish,
            commands::app_info,
            commands::data_dir_set,
            commands::reveal_data_dir,
            commands::backup_export,
            commands::db_export,
            commands::write_text_file,
            commands::pack_prompt,
            commands::curriculum,
            commands::knowledge_point_list,
            commands::knowledge_point_create,
            commands::question_list,
            commands::question_sources,
            commands::question_stats,
            commands::question_get,
            commands::question_save,
            commands::question_review,
            commands::question_delete,
            commands::question_restore,
            commands::question_purge,
            commands::tag_list,
            commands::question_export,
            commands::question_export_zip,
            commands::asset_data,
            commands::asset_add,
            commands::asset_remove,
            commands::capture_region,
            commands::overlay_ready,
            commands::submit_selection,
            commands::cancel_selection,
            commands::overlay_report,
            commands::image_from_path,
            commands::file_open,
            commands::save_temp_image,
            commands::diagram_generate,
            commands::import_scan,
            commands::import_scan_text,
            commands::import_commit,
            commands::import_discard,
            commands::import_batches,
            commands::import_errors,
            commands::import_undo,
            commands::models_list,
            commands::model_save,
            commands::model_delete,
            commands::model_test,
            commands::model_list_remote,
            commands::routes_get,
            commands::route_set,
            commands::job_start,
            commands::job_get,
            commands::job_recent,
            commands::job_cancel,
            commands::job_commit,
            commands::job_item_update,
            commands::sync_status,
            commands::sync_login,
            commands::sync_logout,
            commands::sync_push,
            commands::sync_pull,
            commands::sync_retry,
            commands::center_classes,
            commands::center_assignments,
            commands::center_binding,
            commands::center_bind_assignment,
            commands::center_pull_assets,
            commands::student_bind_center,
            commands::emit_capture,
            commands::browser_open,
            commands::browser_close,
            commands::browser_bounds,
            commands::browser_hide,
            commands::browser_status,
            commands::emit_capture_image,
            commands::browser_extract_text,
            commands::emit_browser_text,
            commands::browser_extract_questions,
            commands::browser_collect,
            commands::browser_set_subject,
            commands::ocr_status,
            commands::dictation_ocr_from_path,
            commands::dictation_pinyin,
            commands::dictation_set_save,
            commands::dictation_set_list,
            commands::dictation_set_get,
            commands::dictation_set_delete,
            commands::smart_regions,
            commands::smart_crop,
            commands::image_stitch,
            commands::capture_sources,
            commands::capture_source_remember,
            commands::capture_source_star,
            commands::capture_source_forget,
            commands::capture_queue_list,
            commands::capture_queue_add,
            commands::capture_queue_retry,
            commands::capture_queue_remove,
            commands::capture_queue_clear_done,
            commands::student_list,
            commands::student_save,
            commands::student_archive,
            commands::student_delete,
            commands::student_import_preview,
            commands::student_import_commit,
            commands::class_list,
            commands::class_get,
            commands::class_save,
            commands::class_close,
            commands::class_delete,
            commands::class_regenerate_code,
            commands::class_members,
            commands::class_add_members,
            commands::class_remove_member,
            commands::assignment_list,
            commands::assignment_get,
            commands::assignment_save,
            commands::assignment_publish,
            commands::assignment_close,
            commands::assignment_reopen,
            commands::assignment_delete,
            commands::compose_pool,
            commands::compose_draft,
            commands::compose_swap,
            commands::review_sheet,
            commands::attempt_submit,
            commands::attempt_sheet,
            commands::attempt_grade,
            commands::attempt_batch_grade,
            commands::attempt_clear,
            commands::teaching_overview,
            commands::insights_class,
            commands::insights_student,
            commands::insights_roster,
            commands::wrong_book_list,
            commands::wrong_book_review,
            commands::wrong_book_remove,
            commands::wrong_book_due,
            commands::insight_advice,
            commands::insight_advice_generate,
            commands::score_exams,
            commands::score_exam_sheet,
            commands::score_exam_save,
            commands::score_exam_delete,
            commands::score_save,
            commands::score_student_exams,
            commands::student_progress,
            commands::storage_usage,
            commands::storage_clean,
            commands::courseware_list,
            commands::courseware_import,
            commands::courseware_update,
            commands::courseware_delete,
            commands::courseware_preview,
            commands::courseware_build,
            commands::courseware_spec,
            commands::courseware_prompt,
            commands::courseware_image_add,
            commands::courseware_image_dir,
            commands::courseware_ai_generate,
            commands::courseware_parse_draft,
            commands::courseware_export,
            commands::courseware_import_start,
            commands::courseware_import_list,
            commands::courseware_import_retry,
            commands::courseware_import_save,
            commands::courseware_import_commit,
            commands::courseware_import_original,
            commands::courseware_assign,
            commands::courseware_unassign,
            commands::courseware_ai_draft,
            commands::grading_photo_add,
            commands::grading_photo_list,
            commands::grading_photo_by_attempt,
            commands::grading_photo_regions,
            commands::grading_photo_crop,
            commands::grading_photo_attach,
            commands::grading_photo_remove,
            commands::grading_photo_data,
            commands::grading_paper_submit,
            commands::grading_transcribe,
            commands::photo_upload_start,
            commands::photo_upload_stop,
            commands::photo_upload_status,
            paper::paper_schemes,
            paper::paper_list,
            paper::paper_get,
            paper::paper_create,
            paper::paper_update,
            paper::paper_set_total,
            paper::paper_apply_scheme,
            paper::paper_score_check,
            paper::paper_finalize,
            paper::paper_reopen,
            paper::paper_archive,
            paper::paper_duplicate,
            paper::paper_delete,
            paper::paper_restore,
            paper::paper_section_save,
            paper::paper_section_delete,
            paper::paper_section_reorder,
            paper::paper_add_questions,
            paper::paper_remove_question,
            paper::paper_reorder_questions,
            paper::paper_set_scores,
            paper::paper_refresh_snapshot,
            paper::paper_print_log,
            paper::paper_blueprint_suggest,
            paper::paper_blueprint_fill,
            paper::paper_blueprint_again,
            paper::paper_replace_candidates,
            paper::paper_replace_question,
            paper::paper_undo_replace,
            paper::open_path,
        ])
        // 讲义预览走自定义协议：只回已登记的课件文件，不开放整个数据目录。
        .register_uri_scheme_protocol("nbasset", |ctx, request| {
            courseware::serve(ctx.app_handle(), request)
        })
        .run(tauri::generate_context!())
        .expect("启动失败：请把上面的错误发给开发者");
}

#[cfg(test)]
mod acl_tests {
    /// 应用一旦声明权限清单，没列进 `permissions/local.toml` 的命令
    /// 连本机页面也调不动，而且只表现为一个被忽略的 Promise。
    /// 所以每次往 generate_handler 里加命令，这里都会提醒补权限。
    #[test]
    fn local_permissions_cover_every_command() {
        let source = include_str!("main.rs");
        let start = source
            .find("generate_handler![")
            .expect("main.rs 里找不到 generate_handler!");
        let handlers = &source[start..];
        let handlers = &handlers[..handlers.find("])").expect("generate_handler! 没有结束")];
        let permissions = include_str!("../permissions/local.toml");

        let mut missing = Vec::new();
        // 命令按模块分组（commands / paper …），每种前缀都要扫到。
        for prefix in ["commands::", "paper::"] {
            for piece in handlers.split(prefix).skip(1) {
                let name: String = piece
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if name.is_empty() {
                    continue;
                }
                if !permissions.contains(&format!("\"{name}\"")) {
                    missing.push(name);
                }
            }
        }
        assert!(
            missing.is_empty(),
            "permissions/local.toml 少了这些命令：{missing:?}"
        );
    }

    /// 第三方页面注入脚本能调的命令必须同时在 `permissions/browser.toml`
    /// 和 `capabilities/remote-browser.json` 里放行，否则 page 上点「收这题」
    /// 只会安静失败，主窗永远等不到题。
    #[test]
    fn remote_page_scripts_only_call_allowed_commands() {
        let capability = include_str!("../capabilities/remote-browser.json");
        let permissions = include_str!("../permissions/browser.toml");
        assert!(
            capability.contains("\"webviews\"") && capability.contains("\"site\""),
            "remote-browser.json 要按 webview 标签绑定到内嵌网页视图 site"
        );
        assert!(
            capability.contains("\"remote\""),
            "remote-browser.json 要给远程页面能力，不能只写 local"
        );

        let mut missing = Vec::new();
        for source in [include_str!("browser.rs"), include_str!("commands.rs")] {
            for piece in source.split("invoke('").skip(1) {
                let name: String = piece.chars().take_while(|c| *c != '\'').collect();
                if name.is_empty() {
                    continue;
                }
                let permission = format!("allow-{}", name.replace('_', "-"));
                if !permissions.contains(&format!("\"{name}\"")) {
                    missing.push(format!("{} 不在 permissions/browser.toml", name));
                }
                if !capability.contains(&format!("\"{permission}\"")) {
                    missing.push(format!("{} 没在 remote-browser.json 放行", permission));
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "第三方页面注入脚本调用了未放行的命令：{missing:?}"
        );
    }
}
