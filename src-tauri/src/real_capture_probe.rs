//! 一次性排查用：拿真实库 + 真实 Key + 真实 PDF 图，跑通 capture 全链路。
//! 定位「识别图片报错」到底卡在哪一步。
//! 用法：cargo test --lib real_capture_probe -- --nocapture --ignored

use crate::llm::{self, prompts, ImageInput};
use crate::{db, jobs};
use serde_json::json;
use std::path::PathBuf;

fn real_db() -> PathBuf {
    PathBuf::from(std::env::var("NB_REAL_DB").unwrap_or_else(|_| {
        format!(
            "{}/Documents/NeuralKB-Local/neuralkb.db",
            std::env::var("HOME").unwrap_or_default()
        )
    }))
}

#[test]
#[ignore]
fn real_capture_probe() {
    let db_path = real_db();
    println!("== 真实库：{}", db_path.display());
    let conn = db::open(&db_path).expect("打开真实库");

    // 1) 路由与模型
    let models = llm::list_models(&conn).expect("读模型");
    for m in &models {
        println!(
            "== 模型 {} base={} model={} vision={} enabled={} key_present={}",
            m.name,
            m.base_url,
            m.model,
            m.supports_vision,
            m.enabled,
            llm::get_key(&m.id).is_some()
        );
    }

    // 2) 找一张最新的 tmp PDF 图
    let tmp = db_path.parent().unwrap().join("tmp");
    let mut imgs: Vec<PathBuf> = std::fs::read_dir(&tmp)
        .expect("读 tmp")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map(|e| e == "png" || e == "jpg")
                .unwrap_or(false)
        })
        .collect();
    imgs.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let img_path = imgs.last().expect("tmp 里要有图").clone();
    println!("== 用图：{}", img_path.display());

    // 3) 读图 + 嗅探
    let bytes = std::fs::read(&img_path).expect("读图");
    let (mime, _) = crate::text::sniff_image(&bytes).expect("嗅探图片类型");
    println!("== 图片 {} bytes, mime={}", bytes.len(), mime);

    // 4) 造提示词（与前端 capture 用途一致）
    let subject = "数学".to_string();
    let messages = prompts::capture(&subject, &["集合".to_string()]);
    println!("== 提示词消息数：{}", messages.len());

    // 5) 真正调模型
    let http = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .expect("建 http");
    let image = ImageInput {
        mime: mime.to_string(),
        bytes: bytes.clone(),
    };
    let mut cooldowns = std::collections::HashMap::new();
    match llm::complete(
        &conn,
        &http,
        "capture",
        &messages,
        Some(&image),
        true,
        4096,
        &mut cooldowns,
    ) {
        Ok(raw) => {
            println!("== 模型返回长度 {}", raw.len());
            match llm::parse_json(&raw, "识别模型") {
                Ok(value) => {
                    let n = value
                        .get("questions")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    println!("== ✅ JSON 解析成功，题数 {}", n);
                }
                Err(error) => println!("== ❌ JSON 解析失败：{}", error),
            }
        }
        Err(error) => println!("== ❌ 模型调用失败：{}", error),
    }

    // 6) 再走一遍 jobs::start 的完整任务路径（含入库与状态流转）
    let ctx = crate::state::JobContext {
        db_path: db_path.clone(),
    };
    let job_id = jobs::start(
        ctx,
        http,
        &conn,
        "capture",
        json!({
            "imagePath": img_path.to_string_lossy(),
            "subjectId": "",
            "sourceUrl": "",
        }),
    )
    .expect("起任务");
    println!("== 任务已起：{}", job_id);

    for _ in 0..90 {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let job = jobs::get(&conn, &job_id).expect("读任务");
        if job.status != "RUNNING" {
            println!("== 任务终态 status={} error={:?}", job.status, job.error);
            if let Some(result) = job.result {
                let n = result
                    .get("items")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                println!("== 识别条目数 {}", n);
            }
            return;
        }
    }
    println!("== 任务超时未完成");
}
