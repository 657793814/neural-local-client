//! 客户端提示词。
//!
//! 与中心系统 `PromptCatalog` 同一套口径：逐字转录、公式 `$...$`、JSON 反斜杠写两个。
//! 与服务端共用的段落随本仓库 `shared/prompts/` 目录分发（从原 monorepo 复制），
//! 这里用 `include_str!` 读，不复制。客户端专属的（一图多题切分、账号级归类）留在本文件，
//! 服务端对应条目见 `server/src/main/java/com/neuralkb/llm/prompt/PromptCatalog.java`（原 monorepo 路径）。

use super::{msg, Msg};

/// 与 `shared/prompts/question.system.json.md` 同一个文件，编译期读入，不复制。
const JSON_SYSTEM: &str = include_str!("../../../shared/prompts/question.system.json.md");

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "暂无".to_string()
    } else {
        items.join("、")
    }
}

/// 题型的中文说法。提示词里不能直接写 SINGLE / SOLVED——模型看不懂代号，
/// 会照着 JSON 示例里的 "SINGLE" 一路出选择题（用户在界面上选简答题也照旧）。
fn type_cn(code: &str) -> &'static str {
    match code {
        "SINGLE" => "单选题",
        "MULTI" => "多选题",
        "FILL" => "填空题",
        _ => "解答题（计算、证明、简答都算）",
    }
}

fn is_choice(code: &str) -> bool {
    code == "SINGLE" || code == "MULTI"
}

/// 截图识别：一张图可能有多道题，要求先切分再逐题给字段。
pub fn capture(subject: &str, knowledge: &[String]) -> Vec<Msg> {
    let system = format!(
        "{JSON_SYSTEM}
你是题目整理助手。用户会给你一张截图，里面可能有一道题，也可能有多道题。

要求：
- 先按题号切分，一道题一条，不要把两道题合成一条，也不要漏掉小题。
- 题干与选项必须逐字来自图片，不能改写、不能补条件、不能新增小问。
- 「如图」「图中」原样保留，不要改成文字描述图形。
- 题干或选项里有几何图形、函数图像、表格、示意图、实物照片时，不要用文字描述，
  在原位置写 `![](图1)`（同一张图内编号递增），并在 figures 里给出这块区域的位置。
  选项本身就是一张图时，该选项的 text 只写 `![](图N)`。
- figures[].bbox 写 [左上角x, 左上角y, 宽, 高]，按整张图片的 0–1000 相对坐标
  （左上角是 0,0，右下角是 1000,1000）。框要**紧贴图形本身**：图形的线条、坐标轴、
  图上的字母标注都在框内，但题干文字、选项文字、题号、相邻题目、页眉页脚、
  大片空白都不能圈进来。文字和图形在同一行时，只框图形那一块。
- 题干只写这道题本身。截图里如果有别的题目、卷头、按钮、说明文字，不要并入题干，
  也不要当成这道题的一部分；切不开就让它们各自成题或直接忽略。
- 没有图形时 figures 给 []，不要为纯文字区域生成图。
- 模糊、被裁掉、公式看不清的部分保留原文，并写进 warnings，不要猜。
- 每题的 answerSource：图片里有答案就写 KEY，自己解出来写 MODEL，信息不足写 MISSING。
- 只整理题目本身，不要输出题号、卷头、分值。
- 网页截图底部/侧边的工具栏与按钮文字（如今日、N次组卷、N卷引用、相似题、纠错、详情、收藏、加入试题篮、下载、打印）不是题目，一律忽略，不要写成题干，也不要单独当成一道题。"
    );
    let user = format!(
        "学科：{subject}

可选题库知识点（只能从这里选，标题原样抄写）：
{}

请输出：
{{\"questions\":[{{
  \"type\":\"SINGLE\",
  \"stem\":\"\",
  \"options\":[{{\"key\":\"A\",\"text\":\"\"}}],
  \"figures\":[{{\"ref\":\"图1\",\"bbox\":[0,0,0,0],\"desc\":\"\"}}],
  \"answer\":\"\",
  \"analysis\":\"\",
  \"difficulty\":3,
  \"knowledgeNames\":[\"\"],
  \"warnings\":[],
  \"answerSource\":\"MISSING\",
  \"confidence\":0.8
}}]}}

约定：
- type：SINGLE 单选 / MULTI 多选 / FILL 填空 / SOLVED 解答、计算、证明。
- options：选择题给 2–7 项，非选择题给 []。
- figures：题干 / 选项里的图形；ref 与正文 `![](图N)` 一致；没有图形给 []。
- answer：选择题填字母（多选写 AC），其他填最终答案文本。
- analysis：解题过程；看不出思路就留空，不要编。
- difficulty：1–5 的整数。
- 公式用 $...$ 包裹，JSON 里的反斜杠写成两个。
- knowledgeNames：必填，每题 1–3 个知识点；只能从上面的知识点标题里原样抄写，没有合适的也给最接近的 1 个，不要留空、不要编造。",
        list_or_none(knowledge)
    );
    vec![msg("system", system), msg("user", user)]
}

/// 文本取题：用户从网页复制的文字（无图），整理成题目；不需要视觉模型。
pub fn capture_text(subject: &str, knowledge: &[String], text: &str) -> Vec<Msg> {
    let system = format!(
        "{JSON_SYSTEM}
你是题目整理助手。用户会贴给你从网页复制的题目文字。
把里面的题目逐题整理成结构化数据；不要凭空补题干里没有的条件。"
    );
    let user = format!(
        "学科：{subject}

可选题库知识点（只能从这里选，标题原样抄写）：
{}

下面是用户复制的文字，请从中整理题目：
---
{text}
---

输出与截图识别相同的 JSON：{{\"questions\":[{{\"type\":\"SINGLE\",\"stem\":\"\",\"options\":[{{\"key\":\"A\",\"text\":\"\"}}],\"answer\":\"\",\"analysis\":\"\",\"difficulty\":3,\"knowledgeNames\":[\"\"],\"warnings\":[],\"answerSource\":\"MISSING\",\"confidence\":0.8}}]}}

约定：
- 逐题切分，一题一条；题号、卷头、分值不要写进题干。
- 网页复制文字里常夹着工具栏按钮（如今日、N次组卷、N卷引用、相似题、纠错、详情、收藏、加入试题篮、下载、打印），这些不是题目，一律忽略。
- 题干与选项逐字来自上面的文字，不要改写。
- answer：文字里有答案就照抄并写 answerSource=KEY，自己解出来写 MODEL，信息不足写 MISSING。
- 公式用 $...$ 包裹，反斜杠写成两个。
- knowledgeNames：必填，每题 1–3 个知识点；从上面的知识点标题里原样抄写，没有合适的也给最接近的 1 个，不要留空、不要编造。",
        list_or_none(knowledge)
    );
    vec![msg("system", system), msg("user", user)]
}

/// AI 出题：按知识点 / 题型 / 难度生成新题，必须同时有答案、解析、知识点。
#[allow(clippy::too_many_arguments)]
pub fn generate(
    subject: &str,
    types: &[String],
    difficulty: &str,
    difficulty_level: i64,
    count: i64,
    knowledge: &[String],
    focus: Option<&str>,
    figures: bool,
) -> Vec<Msg> {
    let system = format!(
        "{JSON_SYSTEM}
你是学科教研员，负责为个人题库出题。题目必须完整独立、条件充分、答案明确，不能引用其他题目。"
    );
    let focus_line = focus
        .map(|value| format!("\n本次重点：{value}"))
        .unwrap_or_default();
    // 题型用中文写死，JSON 示例也照着所选题型示例，避免模型习惯性输出选择题。
    let type_words: Vec<&str> = types.iter().map(|code| type_cn(code)).collect();
    let sample_type = types.first().map(|code| code.as_str()).unwrap_or("SINGLE");
    let sample_options = if is_choice(sample_type) {
        "[{\"key\":\"A\",\"text\":\"\"}]"
    } else {
        "[]"
    };
    let only = if types.len() == 1 {
        format!("\n这一次只出「{}」，不要出其它题型。", type_words[0])
    } else {
        String::new()
    };
    // 带图出题：模型只判断「这题该不该配图」，图由本地制图链路单独画，
    // 不要求出题模型直接输出图形——它的本职是出题，兼职画图两件都做不好。
    let figure_rule = if figures {
        "\n- 每道题多给一个 needFigure 字段：几何、函数图象、统计图、物理示意图、\
         需要看图才能作答的题给 true；纯计算、纯文字题给 false。\
         needFigure 为 true 的题，题干里必须出现「如下图」或类似指代，图形稍后由系统补上。"
    } else {
        "\n- 不要输出 needFigure 字段。"
    };
    let sample_figure = if figures { ",\"needFigure\":false" } else { "" };
    let user = format!(
        "学科：{subject}
生成数量：{count}
允许的题型：{}{only}
难度目标：{difficulty}{focus_line}

可用知识点（只能从这里选，标题原样抄写）：
{}

要求：
- 题型只能从「允许的题型」里选，不许自己换：要解答题就不能给选择题。
- 只有单选题 / 多选题才写 options（4 个）；填空题、解答题的 options 必须是 []。
- 解答题的 answer 写最终结论（可分问），不能写成选项字母。
- 每道题的 analysis 必须写清关键步骤并体现最终答案，选择题不少于 20 字，解答题不少于 40 字。
- 公式用 $...$ 包裹，JSON 里的反斜杠写成两个。
- knowledgeNames：必填，每题 1–3 个知识点；从上面的知识点标题里挑最贴切的、原样抄写，不能编造。
- 选择题必须给 4 个选项，SINGLE 的 answer 只填一个字母，MULTI 至少两个字母。
- 各题情境与设问必须互不重复。{figure_rule}
- 每题 difficulty 字段必须填 {difficulty_level}（对应「{difficulty}」），不许用示例里的数字；这是本次出题的硬性难度要求。

只输出：
{{\"questions\":[{{\"type\":\"{sample_type}\",\"stem\":\"\",\"options\":{sample_options},\"answer\":\"\",\"analysis\":\"\",\"difficulty\":{difficulty_level},\"knowledgeNames\":[\"\"]{sample_figure}}}]}}",
        type_words.join("、"),
        list_or_none(knowledge)
    );
    vec![msg("system", system), msg("user", user)]
}

/// 归类：只允许从本地目录里选学科与知识点，答案 / 解析仅在原题缺失时补。
pub fn classify(subjects: &[String], knowledge: &[String], question: &str) -> Vec<Msg> {
    let system = format!(
        "{JSON_SYSTEM}
你是题库整理助手。你的任务是给已有题目补元数据，不是重写题目。"
    );
    let user = format!(
        "可选学科：{}
可选知识点（code | 标题）：
{}

题目：
{question}

请输出：
{{\"subject\":\"\",\"difficulty\":3,\"knowledgeNames\":[\"\"],\"answer\":\"\",\"analysis\":\"\",\"confidence\":0.8,\"note\":\"\"}}

约定：
- subject 只能从上面的学科里选一个，选不出就留空。
- knowledgeNames：必填，1–3 个知识点；只能从上面的知识点标题里原样抄写，没有合适的也给最接近的 1 个，不要自编。单个也可以用 knowledgeName 给一个标题。
- answer / analysis 只在原题缺失时补写；原题已有就原样返回，不要改写。
- 公式用 $...$ 包裹，JSON 里的反斜杠写成两个。
- difficulty：1–5。
- confidence：0–1，拿不准就给低分，让用户自己决定。",
        list_or_none(subjects),
        list_or_none(knowledge)
    );
    vec![msg("system", system), msg("user", user)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_mention_math_and_json_rules() {
        let messages = capture("数学", &["MATH-1-1 | 正数和负数".into()]);
        assert_eq!(messages.len(), 2);
        assert!(messages[0].content.contains("$...$"));
        assert!(messages[0].content.contains("反斜杠"));
        assert!(messages[1].content.contains("questions"));
        assert!(
            !generate("数学", &["SINGLE".into()], "中档", 3, 3, &[], None, false)[1]
                .content
                .is_empty()
        );
        assert!(generate("数学", &["SINGLE".into()], "中档", 3, 3, &[], None, true)[1]
            .content
            .contains("needFigure"));
    }

    /// 题型必须用中文写进提示词，示例也照着所选题型走——否则模型会照着
    /// 示例里的 "SINGLE" 一路出选择题，界面上选的简答题等于没选。
    #[test]
    fn generate_prompt_uses_chinese_type_words() {
        let messages = generate("数学", &["SOLVED".into()], "中档", 3, 2, &[], None, false);
        let user = &messages[1].content;
        assert!(user.contains("解答题"), "应写出中文题型：{user}");
        assert!(
            user.contains("这一次只出"),
            "单选一种题型时要写明只能出这种"
        );
        assert!(
            !user.contains("允许的题型：SOLVED"),
            "不能把枚举值直接丢给模型"
        );
        assert!(
            user.contains("\"type\":\"SOLVED\""),
            "JSON 示例要跟着所选题型"
        );
        assert!(user.contains("\"options\":[]"), "解答题的示例不能带选项");
    }

    /// 选「很难」时提示词的示例与硬性规则都得跟着走，不能写死 3。
    #[test]
    fn generate_prompt_pins_difficulty_level() {
        let user = generate("数学", &["SINGLE".into()], "很难", 5, 3, &[], None, false)[1]
            .content
            .clone();
        assert!(
            user.contains("\"difficulty\":5"),
            "示例里的难度要跟着选择走：{user}"
        );
        assert!(
            user.contains("必须填 5"),
            "要明确告诉模型难度是硬性要求：{user}"
        );
        assert!(
            !user.contains("\"difficulty\":3"),
            "不能再出现写死的 3：{user}"
        );
    }
}

/// 讲义补题：以一道例题为锚，补变式 / 拔高 / 思维训练题（P4 §8.2）。
/// 产出只是草稿，老师在编辑器里过目、改完再编译进讲义。
pub fn handout_draft(input: &serde_json::Value) -> Vec<Msg> {
    let subject = input.get("subject").and_then(|v| v.as_str()).unwrap_or("");
    let grade = input.get("grade").and_then(|v| v.as_str()).unwrap_or("");
    let knowledge: Vec<String> = input
        .get("knowledge")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let example = input.get("example").and_then(|v| v.as_str()).unwrap_or("");
    let want = input
        .get("want")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>()
                .join("、")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "变式、拔高、思维训练".to_string());
    let system = format!(
        "{JSON_SYSTEM}\n你是教研员，为教师讲义补题。所有题目必须紧扣给定知识点与例题的考查方式，不引入超纲内容；变式题要「同考点、换情境或换问法」，拔高题适度综合，思维训练题给开放设问与提示。"
    );
    let user = format!(
        "学科：{subject}
学段：{grade}
本讲义知识点（题目必须落在这上面）：{}
例题（锚点，变式要与它同考点）：
{example}

需要补的题型：{want}

要求：
- 变式题与例题同考点但情境/设问不同，不能只是换个数字；
- 拔高题可以综合 2 个小步骤，但仍在知识点范围内；
- 思维训练题是开放设问，必须给 hint，answer 写参考思路（可以简短）；
- 每题独立完整，不引用其他题；
- analysis 写关键步骤，变式/拔高不少于 20 字。

只输出：
{{\"variants\":[{{\"stem\":\"\",\"answer\":\"\",\"analysis\":\"\",\"steps\":[\"\"]}}],
 \"challenge\":[{{\"stem\":\"\",\"answer\":\"\",\"analysis\":\"\",\"steps\":[\"\"]}}],
 \"thinking\":[{{\"prompt\":\"\",\"hint\":\"\",\"answer\":\"\"}}]}}",
        list_or_none(&knowledge),
    );
    vec![msg("system", system), msg("user", user)]
}

/// 拍照批改：把学生手写的答案抄成文字，并给一个建议分（决策 D4：只给建议，
/// 分数与评语必须老师确认）。原文照片永远保留，模型不许「重写」答案。
pub fn grading_transcribe(stem: &str, answer: &str, max_score: f64) -> Vec<Msg> {
    let system = format!(
        "{JSON_SYSTEM}\n你是批改助手，看的是学生手写作答的照片。你只做两件事：\n\
1) 把手写内容**逐字抄录**成文字（看不清的地方写「□」并说明，不要猜）；\n\
2) 给出建议得分与一句评语。你不是阅卷老师，分数只是建议，老师会自己定。\n\
严禁改写、补全、润色学生写的内容；空着的地方就是空着。"
    );
    let user = format!(
        "题目：{stem}

参考答案（用来判断对错，**不要抄进 answer**）：
{answer}

本题满分：{max_score}

要求：
- answer 只写学生实际写出来的内容，数学式用 $...$ 包裹；学生没写就写空字符串；
- 看不清的字用「□」占位，并在 warning 里说明是哪一处；
- score 是建议得分，0 到 {max_score} 之间，可以是小数；
- remark 一句话，不超过 30 字，写给老师看，指出关键对错；
- confidence：看得清写 high，看不清写 low。

只输出：
{{\"answer\":\"\",\"score\":0,\"remark\":\"\",\"confidence\":\"high\",\"warning\":\"\"}}",
    );
    vec![msg("system", system), msg("user", user)]
}

/// 「用 AI 转换」里可一键复制的提示词：让任意支持文件与视觉的智能体把
/// Excel / Word / 图片转成题目包。与方案 §8 一致，客户端内不假设用户懂 JSON。
/// 「用 AI 转换」里可一键复制的提示词，源文件在 `shared/prompts/`。
pub const PACK_PROMPT: &str = include_str!("../../../shared/prompts/question-pack-skill.md");

/// 学情诊断：把一份结构化快照翻译成「结论 + 原因 + 这周怎么做」。
///
/// 刻意限制了三件事：只看给到的数字（不猜性格 / 家庭）、不给具体教辅和付费课、
/// 计划必须落到每天 10–30 分钟能做完的量——否则模型很容易写成正确的废话。
pub fn advice(snapshot: &serde_json::Value) -> Vec<Msg> {
    let system = format!(
        "{JSON_SYSTEM}
你是一位带过多届毕业班的学科老师兼教研组长。你擅长从零散的作业与考试数据里判断一个学生真实卡在哪，并把补救方案写成孩子今天就能开始做、家长能配合的具体安排。"
    );
    let user = format!(
        "下面是本地题库统计出来的一个学生学情（掌握度 0–100，按时间衰减加权；weak 是掌握度低于 70 的知识点；dueReview 是错题本里到期该复习的题数；idleDays 是距上次练习的天数；exams 是最近的考试分数与班内位置）。

{}

请只根据这些数字判断，遵守：
- 不编造数据里没有的事实，不猜性别、性格、家庭情况；
- 不推荐教辅书、App、付费课程；
- 结论要带具体数字（掌握度、题量、名次、分数），让老师知道依据；
- 每天任务控制在 10–30 分钟，任务必须是「看 1 个例题」「做 8 道同类题」「重做错题」这种能直接执行的；
- 如果数据不足以判断（answered 很少或为 0），就如实说「先建立数据」，不要硬凑分析。

只输出这个 JSON：
{{\"headline\":\"一句话结论，40 字以内，带一个数字\",\"strengths\":[{{\"point\":\"知识点名\",\"evidence\":\"用数据说话\"}}],\"issues\":[{{\"kp\":\"知识点名\",\"why\":\"结合题量与掌握度说明卡在哪\",\"action\":\"具体到几道题、看什么\"}}],\"plan\":{{\"focus\":[\"本周集中的知识点\"],\"days\":[{{\"day\":\"周一\",\"task\":\"具体任务\",\"minutes\":15}}]}},\"parentNote\":\"给家长的两三句话：本周在家怎么配合，不制造焦虑\"}}

约束：strengths 最多 2 条，issues 最多 3 条，days 最多 7 天、至少 3 天。",
        serde_json::to_string_pretty(snapshot).unwrap_or_else(|_| "{}".into())
    );
    vec![msg("system", system), msg("user", user)]
}

/// AI 制图：让模型只描述「点、线、面」的结构，由 `diagram_scene` 本地画成 SVG。
///
/// 早先是直接要一整张 SVG，问题不在格式而在几何：模型会漏棱、给题干里没有的点标号、
/// 把标签压在线段上、把顶点画到底面某个角正上方。这些它自己看不见，提示词也管不住。
/// 改成坐标化场景后，模型只需给数字，剩下的排版 / 避让 / 虚线样式由本地保证，
/// 输出长度还只有原来的几分之一，不会再被 max_tokens 截断。
pub fn diagram(subject: &str, stem: &str, focus: &str, style: &str) -> Vec<Msg> {
    let system = format!(
        "{JSON_SYSTEM}
你是教研员兼插图师。用户给你一道题的题干，你要给出这张图的**结构描述**（有哪些点、怎么连、哪里填充），由程序按你给的坐标画出来。你不需要写任何 SVG。

坐标系统：画布是 0–100 的方格，x 向右增大，y 向下增大（y 越大越靠下）。所有点控制在 8–92 之间。

只返回一个 JSON 对象：
{{\"scene\":{{
 \"points\":[{{\"id\":\"A\",\"x\":20,\"y\":80}},{{\"id\":\"B\",\"x\":48,\"y\":80}},{{\"id\":\"E\",\"midpoint\":[\"B\",\"C\"]}}],
 \"faces\":[{{\"points\":[\"A\",\"B\",\"C\",\"D\"]}}],
 \"segments\":[{{\"a\":\"P\",\"b\":\"A\"}},{{\"a\":\"P\",\"b\":\"C\",\"dashed\":true}}],
 \"curves\":[{{\"points\":[[20,60],[40,40],[60,40],[80,55]]}}],
 \"circles\":[{{\"x\":50,\"y\":50,\"r\":12}}],
 \"marks\":[{{\"type\":\"right\",\"at\":\"A\",\"a\":\"B\",\"b\":\"P\"}}],
 \"labels\":[{{\"x\":50,\"y\":96,\"text\":\"θ=60°\"}}]}}}}

字段说明：
- points：题干里出现的点，一个点一条。id 直接写题干里的字母，下标用数字：A1、B1、C1、D1。题干没提到的点一个都不要加。**中点不要自己算坐标**，写成 {{\"id\":\"E\",\"midpoint\":[\"B\",\"C\"]}}，程序会求平均；只有普通点才给 x、y。只为定位箭头端点而临时用的点（例如力的箭头尖端）加 \"hideLabel\":true。
- segments：连线。a、b 是 points 里的 id。被挡住的棱写 \"dashed\":true；带箭头的（坐标轴、力的方向）写 \"arrow\":true。
- faces：需要浅色填充的平面（题目明确要求的截面 / 阴影），按顶点顺序**闭合**给。默认不填色；除非题目点明要画截面，不要给底面、侧面或整个装置铺色。
- curves：函数图像、圆弧、抛物线用折线描出来，给 12–20 个点，点越密越顺。
- circles：圆 / 球的大圆，给圆心和半径（半径按 0–100 尺度给）。
- marks：直角标记。
- labels：只放**数值文字**（角度、长度、坐标），并写明是哪两点 / 哪个角，例如 \"AA₁=4\"、\"AB=2\"、\"θ=60°\"。点名不用写在这里，程序会自动摆在点的外侧。
不要写 title，不要把题干整句抄进图里，不要写 svg 字段，不要写解释文字，不要 markdown 代码块。

作图规矩（必须遵守）：
1. 直棱柱按斜二测画：底面是平行四边形，A 在左下、B 在右下（**A、B 的 y 相同，AB 水平**），C 在右上、D 在左上（**C、D 的 y 相同**）。侧棱必须竖直：A1 与 A 的 x 相同、B1 与 B 的 x 相同，依此类推；A1、B1、C1、D1 由底面点直接向上平移得到，不要另摆。顶面四条边都可见，全部实线。
2. 被挡住的棱才用 dashed：直四棱柱从右前上方看时，通常是后底面的三条（AD、DC、DD1）；右侧的侧棱（CC1）是可见的，画实线。不要把顶面或前面的棱画成虚线。棱一条都不能漏。
3. 中点、三等分点必须准确：中点一律用 midpoint，不要凭感觉摆坐标。
4. 只画题干 / 小问真正用到的辅助线，但**用到的每一条都要画**：例如求二面角 A-MA1-N，就要有 AM、MA1、A1N、MN；证明 MN∥平面 C1DE，就要有 MN 和平面 C1DE 的边界线。不要加与题目无关的辅助线，也不要加题干没提到的点。
5. 平面几何：先定关键点再连线；题干说垂直就给 marks（比如 PA⊥底面，就写 marks 到 A 点，at=A、a=B、b=P），说平行就把两段画成平行。
6. 函数题：用带 arrow 的 segments 画 x、y 轴（交点作为原点 O），curves 画曲线，关键点（交点、顶点、对称轴）用 points + labels 单独标出。
7. 函数题的曲线必须真的经过你标出的关键点：与 x 轴的交点，y 要等于 x 轴的 y（同一水平线）；顶点要对准对称轴。curves 的点序列里要包含这些关键点，别只把曲线画在轴上方。
8. 物理：斜面就用一条斜线段（左下到右上）+ 水平底边，倾角标 θ；物块画成贴着斜面的四边形（faces 给四个点）或用两条平行线段；受力都从物块中心出发——重力竖直向下、支持力垂直斜面向外、摩擦力沿斜面向上，箭头长度按大小关系（重力最长），并给 G、N、f 的 labels。
9. 图形占画布 60%–85%，四周留白，不要贴边，也不要画成一条缝。
10. 题干不足以确定图形时，画能体现核心关系的示意结构，不要返回空场景。"
    );
    let user = format!(
        "学科：{subject}\n模式：{style}\n题干：{stem}\n作图重点：{focus}\n\n请按上面的规矩，只输出含 scene 字段的 JSON。"
    );
    vec![msg("system", system), msg("user", user)]
}

/// 制图重试：把上一轮失败的原因直接告诉模型，比原样再问一遍有效得多。
/// 现在失败大多是「场景里引用了不存在的点」和「只给了点没给线」这类结构错误。
pub fn diagram_retry(
    subject: &str,
    stem: &str,
    focus: &str,
    style: &str,
    error: &str,
) -> Vec<Msg> {
    let mut messages = diagram(subject, stem, focus, style);
    messages.push(msg(
        "user",
        format!(
            "上一轮的结构描述没能画成图：{error}\n请重给一次 scene，并特别注意：\n- segments / faces / marks 里出现的每个点，都必须在 points 里定义；\n- 坐标用 0–100，所有点落在 8–92 之间；\n- 中点写成 midpoint，别自己算错；\n- 至少有 segments 或 curves，只有 points 画不出图；\n- 棱、边一条都不要漏，被挡住的加 dashed。"
        ),
    ));
    messages
}
