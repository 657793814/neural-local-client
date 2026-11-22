//! 变量化课件提示词与模型草稿解析。
//!
//! 模板只维护一份，常规课 / 专题课只换大纲。所有变量都在这里确定性替换，
//! 模型返回统一走 `llm::json` 修复，再经 serde 校验为 HandoutSpec。

use super::{source::ExtractedSource, HandoutSpec};
use crate::llm::{self, msg, Msg};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

const REGULAR_OUTLINE: &str =
    "情境导入 → 学习目标 → 新知讲解 → 例题/活动 → 巩固练习 → 易错提醒 → 课堂小结 → 作业";
const TOPIC_OUTLINE: &str = "专题背景 → 核心概念/方法 → 类型拆解 → 典型例题 → 变式与综合应用 → 易错点 → 方法清单 → 专题训练";

const DEFAULT_TEMPLATE: &str = r#"你是一位经验丰富的学科教师与讲义编辑。请根据下面的变量制作一份可直接校对的课件草稿。

【基本信息】
学科：{{subject}}
课题：{{title}}
年级：{{grade}}
课时：{{duration}}
学生情况：{{audience}}
章节：{{chapter}}
知识点：{{knowledge_points}}
课程类型：{{content_mode}}

【教学要求】
教学目标：{{objectives}}
难度：{{difficulty}}
教学重点：{{teaching_focus}}
教学难点：{{teaching_difficulty}}
教学风格：{{teaching_style}}
例题数量：{{example_count}}
练习数量：{{exercise_count}}
补充要求：{{custom_instruction}}

【内容结构】
{{outline}}

【原始素材】
参考题目：
{{reference_questions}}
参考文字：
{{reference_text}}

【本地图片清单】
{{images}}
只可按图号引用图片；没有实际看到图片时，不得描述图片里的具体细节。正文需要图片的位置写图号（例如「此处插入图1」）。

【输出与素材位置】
目标文件类型：{{file_types}}
幻灯片参考页数：{{slide_count}}
文档篇幅：{{document_length}}
版式：{{theme}}
输出目录：{{output_directory}}
如果接收提示词的智能体具有文件工具，可按上述目录与图片位置工作；没有文件工具时，只返回下面的结构化草稿，不要假装已经写出文件。

【自定义变量】
{{custom_variables}}

【质量约束】
- 不编造事实、来源、题目条件或图片内容；信息不足就留空或明确写待老师补充。
- 公式必须使用 $...$ 包裹；JSON 字符串中的 LaTeX 反斜杠必须写成两个。
- 例题和练习应条件完整、答案明确；解析写关键步骤，不把答案重复一遍当解析。
- 内容适合上述学生与课时，不把输出文件类型当成要求模型直接生成二进制文件。
- 只返回一个 HandoutSpec JSON 对象，不要 Markdown 代码围栏，不要解释。

JSON 字段：title、subject、grade、chapter、description、knowledge、objectives、warmup、
points[{name,summary,detail,misconception,formula}]、
examples/variants/challenge[{questionId,stem,answer,analysis,steps}]、
thinking[{prompt,hint,answer}]、summary、homework、theme、contentMode、duration、audience、
difficulty、teachingFocus、teachingDifficulty、teachingStyle、customInstruction、aiAdded。
questionId 一律留空；images 与 source 不必返回，由本地系统保留。数组没有内容时返回 []。"#;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PromptInput {
    pub spec: HandoutSpec,
    #[serde(default)]
    pub reference_questions: String,
    #[serde(default)]
    pub reference_text: String,
    #[serde(default)]
    pub file_types: Vec<String>,
    #[serde(default)]
    pub slide_count: u32,
    #[serde(default)]
    pub document_length: String,
    #[serde(default)]
    pub output_directory: String,
    #[serde(default)]
    pub example_count: u32,
    #[serde(default)]
    pub exercise_count: u32,
    #[serde(default)]
    pub custom_variables: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResult {
    pub prompt: String,
    pub missing: Vec<String>,
    pub outline: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GenerateInput {
    pub input: PromptInput,
    /// 用户手改后的提示词；为空时使用同一份内置模板现场构建。
    pub prompt: String,
    /// 首版视觉调用只附这一张图，其余图片仍以清单出现在提示词里。
    pub vision_image_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateResult {
    pub spec: Option<HandoutSpec>,
    pub raw: String,
    pub error: Option<String>,
}

fn value(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "未指定".into()
    } else {
        trimmed.to_string()
    }
}

fn lines(items: &[String]) -> String {
    if items.is_empty() {
        "未指定".into()
    } else {
        items.join("\n")
    }
}

fn images_text(spec: &HandoutSpec) -> String {
    if spec.images.is_empty() {
        return "无".into();
    }
    spec.images
        .iter()
        .map(|image| {
            let source = if image.source_path.trim().is_empty() {
                image.path.as_str()
            } else {
                image.source_path.as_str()
            };
            format!(
                "- {}｜文件：{}｜本地位置：{}｜说明：{}｜建议位置：{}｜{}",
                image.id,
                value(&image.name),
                value(source),
                value(&image.caption),
                value(&image.placement),
                if image.required {
                    "必须使用"
                } else {
                    "可选"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn build(input: &PromptInput) -> Result<PromptResult, String> {
    let spec = &input.spec;
    let topic = spec.content_mode.eq_ignore_ascii_case("TOPIC");
    let outline = if topic {
        TOPIC_OUTLINE
    } else {
        REGULAR_OUTLINE
    };
    let file_types = if input.file_types.is_empty() {
        "HTML、DOCX、PPTX、PDF".into()
    } else {
        input.file_types.join("、")
    };
    let custom = if input.custom_variables.is_empty() {
        "无".into()
    } else {
        let mut entries = input.custom_variables.iter().collect::<Vec<_>>();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        entries
            .into_iter()
            .map(|(key, val)| format!("{key}：{}", value(val)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut variables = HashMap::from([
        ("subject", value(&spec.subject)),
        ("title", value(&spec.title)),
        ("grade", value(&spec.grade)),
        ("duration", value(&spec.duration)),
        ("audience", value(&spec.audience)),
        (
            "content_mode",
            if topic {
                "专题课".into()
            } else {
                "常规课".into()
            },
        ),
        ("chapter", value(&spec.chapter)),
        ("knowledge_points", lines(&spec.knowledge)),
        ("objectives", lines(&spec.objectives)),
        ("difficulty", value(&spec.difficulty)),
        ("teaching_focus", value(&spec.teaching_focus)),
        ("teaching_difficulty", value(&spec.teaching_difficulty)),
        ("teaching_style", value(&spec.teaching_style)),
        ("outline", outline.into()),
        ("example_count", input.example_count.max(1).to_string()),
        ("exercise_count", input.exercise_count.max(1).to_string()),
        ("custom_instruction", value(&spec.custom_instruction)),
        ("reference_questions", value(&input.reference_questions)),
        ("reference_text", value(&input.reference_text)),
        ("images", images_text(spec)),
        ("file_types", file_types),
        ("slide_count", input.slide_count.max(1).to_string()),
        ("document_length", value(&input.document_length)),
        ("theme", value(&spec.theme)),
        ("output_directory", value(&input.output_directory)),
        ("custom_variables", custom),
    ]);
    for (key, val) in &input.custom_variables {
        variables.insert(key.as_str(), value(val));
    }

    let mut prompt = DEFAULT_TEMPLATE.to_string();
    for (key, val) in variables {
        prompt = prompt.replace(&format!("{{{{{key}}}}}"), &val);
    }
    if let Some(start) = prompt.find("{{") {
        let end = prompt[start..]
            .find("}}")
            .map(|at| start + at + 2)
            .unwrap_or(prompt.len());
        return Err(format!("提示词还有未识别的变量：{}", &prompt[start..end]));
    }
    let mut missing = Vec::new();
    if spec.title.trim().is_empty() {
        missing.push("课题".into());
    }
    if spec.subject.trim().is_empty() {
        missing.push("学科".into());
    }
    Ok(PromptResult {
        prompt,
        missing,
        outline: outline.into(),
    })
}

fn strip_nulls(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|_, node| !node.is_null());
            for node in map.values_mut() {
                strip_nulls(node);
            }
        }
        Value::Array(items) => {
            for node in items {
                strip_nulls(node);
            }
        }
        _ => {}
    }
}

fn fill(target: &mut String, base: &str) {
    if target.trim().is_empty() {
        *target = base.to_string();
    }
}

/// 模型只负责内容；受管图片、来源追溯和本地学科 id 始终以本地草稿为准。
pub fn parse_draft(raw: &str, base: &HandoutSpec, what: &str) -> Result<HandoutSpec, String> {
    let mut value = llm::parse_json(raw, what)?;
    if let Some(wrapper) = value
        .get("spec")
        .or_else(|| value.get("handout"))
        .or_else(|| value.get("courseware"))
        .cloned()
    {
        value = wrapper;
    }
    strip_nulls(&mut value);
    let mut spec: HandoutSpec =
        serde_json::from_value(value).map_err(|e| format!("{what}字段不符合课件草稿结构：{e}"))?;
    fill(&mut spec.title, &base.title);
    fill(&mut spec.subject, &base.subject);
    fill(&mut spec.grade, &base.grade);
    fill(&mut spec.chapter, &base.chapter);
    fill(&mut spec.description, &base.description);
    fill(&mut spec.content_mode, &base.content_mode);
    fill(&mut spec.duration, &base.duration);
    fill(&mut spec.audience, &base.audience);
    fill(&mut spec.difficulty, &base.difficulty);
    fill(&mut spec.teaching_focus, &base.teaching_focus);
    fill(&mut spec.teaching_difficulty, &base.teaching_difficulty);
    fill(&mut spec.teaching_style, &base.teaching_style);
    fill(&mut spec.custom_instruction, &base.custom_instruction);
    fill(&mut spec.theme, &base.theme);
    if spec.knowledge.is_empty() {
        spec.knowledge = base.knowledge.clone();
    }
    if spec.objectives.is_empty() {
        spec.objectives = base.objectives.clone();
    }
    spec.subject_id = base.subject_id.clone();
    spec.images = base.images.clone();
    spec.source = base.source.clone();
    if spec.theme.trim().is_empty() {
        spec.theme = "paper".into();
    }
    if spec.title.trim().is_empty() {
        return Err(format!("{what}没有给出课题，请补充后重试"));
    }
    Ok(spec)
}

pub fn generation_messages(prompt: &str) -> Vec<Msg> {
    vec![
        msg(
            "system",
            "你是课件内容生成器。只输出符合用户给定 HandoutSpec 字段的 JSON 对象。公式用 $...$，JSON 中反斜杠写两个；不编造图片内容、事实或文件写入结果。",
        ),
        msg("user", prompt),
    ]
}

/// 调用本机配置的模型。即使结构化解析失败，也把原始返回交给工作台保留。
pub fn generate(
    conn: &rusqlite::Connection,
    client: &reqwest::blocking::Client,
    assets_root: &std::path::Path,
    input: &GenerateInput,
) -> Result<GenerateResult, String> {
    let built = if input.prompt.trim().is_empty() {
        build(&input.input)?.prompt
    } else {
        input.prompt.trim().to_string()
    };
    if built.contains("{{") {
        return Err("提示词里还有未替换的变量，请先补全或恢复默认模板".into());
    }
    if input.input.spec.title.trim().is_empty() || input.input.spec.subject.trim().is_empty() {
        return Err("先填课题和学科，再调用模型生成".into());
    }
    let image = if input.vision_image_id.trim().is_empty() {
        None
    } else {
        let selected = input
            .input
            .spec
            .images
            .iter()
            .find(|image| image.id == input.vision_image_id)
            .ok_or_else(|| "要附给视觉模型的图片不在这份草稿里".to_string())?;
        let (bytes, mime) = super::read_image(assets_root, selected)?;
        Some(llm::ImageInput { mime, bytes })
    };
    let raw = llm::complete(
        conn,
        client,
        "courseware",
        &generation_messages(&built),
        image.as_ref(),
        true,
        8192,
        &mut HashMap::new(),
    )?;
    match parse_draft(&raw, &input.input.spec, "课件生成") {
        Ok(spec) => Ok(GenerateResult {
            spec: Some(spec),
            raw,
            error: None,
        }),
        Err(error) => Ok(GenerateResult {
            spec: None,
            raw,
            error: Some(error),
        }),
    }
}

pub fn import_messages(base: &HandoutSpec, source: &ExtractedSource) -> Vec<Msg> {
    let optimize = base.source.mode == "OPTIMIZE";
    let rule = if optimize {
        "可以润色表达、补足教学结构与缺失解析，但所有新增或实质改写都要在 aiAdded 数组里逐项说明，供老师校对。"
    } else {
        "仅统一格式：逐字保留原稿的教学内容，不改写、不补答案、不新增知识点。你只能判断标题、目标、讲解、例题、练习、答案、解析、小结和作业的角色；无法确定的内容放 points[].detail，不得丢弃。aiAdded 返回 []。"
    };
    let text = if source.text.chars().count() > 60_000 {
        source.text.chars().take(60_000).collect::<String>()
            + "\n[后文超出本次整理上限，请分段重试]"
    } else {
        source.text.clone()
    };
    let prompt = format!(
        "请把下面这份已有讲义整理成 HandoutSpec JSON 草稿。\n整理模式：{}\n{}\n学科：{}\n课题：{}\n\n原稿抽取文字：\n---\n{}\n---\n\n字段与生成课件相同：title、subject、grade、chapter、description、knowledge、objectives、warmup、points、examples、variants、challenge、thinking、summary、homework、theme、contentMode、aiAdded。images 与 source 不必返回。只返回 JSON；公式用 $...$，JSON 反斜杠写两个。",
        base.source.mode,
        rule,
        base.subject,
        base.title,
        text
    );
    vec![
        msg(
            "system",
            "你是讲义结构整理器，不是自由创作者。严格遵守用户给的整理模式，只输出 HandoutSpec JSON。不得声称保留了实际未出现在草稿里的原文。",
        ),
        msg("user", prompt),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::courseware::HandoutImage;

    fn input(mode: &str) -> PromptInput {
        PromptInput {
            spec: HandoutSpec {
                title: "勾股定理".into(),
                subject: "数学".into(),
                content_mode: mode.into(),
                images: vec![HandoutImage {
                    id: "图1".into(),
                    path: "courseware/drafts/d/图1.png".into(),
                    name: "三角形.png".into(),
                    source_path: "/tmp/三角形.png".into(),
                    caption: "直角三角形".into(),
                    required: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            file_types: vec!["DOCX".into(), "PPTX".into()],
            slide_count: 12,
            ..Default::default()
        }
    }

    #[test]
    fn replaces_all_variables_and_keeps_image_locations() {
        let result = build(&input("REGULAR")).unwrap();
        assert!(!result.prompt.contains("{{"));
        assert!(result.prompt.contains("勾股定理"));
        assert!(result.prompt.contains("/tmp/三角形.png"));
        assert!(result.prompt.contains("必须使用"));
        assert!(result.missing.is_empty());
        assert!(result.outline.contains("情境导入"));
    }

    #[test]
    fn topic_mode_changes_outline_and_reports_required_fields() {
        let mut input = input("TOPIC");
        input.spec.title.clear();
        let result = build(&input).unwrap();
        assert!(result.outline.contains("类型拆解"));
        assert_eq!(result.missing, vec!["课题"]);
    }

    #[test]
    fn parses_wrapped_model_json_without_losing_managed_images() {
        let base = input("REGULAR").spec;
        let raw = r#"```json
        {"spec":{"title":"新课程","points":[],"examples":[{"stem":"$\\sqrt{2}$","answer":"1","analysis":"化简"}]}}
        ```"#;
        let spec = parse_draft(raw, &base, "课件生成").unwrap();
        assert_eq!(spec.title, "新课程");
        assert_eq!(spec.images.len(), 1);
        assert_eq!(spec.images[0].path, base.images[0].path);
        assert_eq!(spec.subject, "数学");
    }
}
