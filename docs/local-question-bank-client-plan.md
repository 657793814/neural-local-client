# 个人本地题库客户端（Tauri）规划

> 状态：**P0–P4 已落地（`local-client/`）；录题、OCR 与网页取题收尾完成**｜最后更新 2026-11-17｜目标平台：macOS 优先，Windows 其次
>
> 关联文档：[桌面壳方案](../../neural-knowledge-base/docs/product/desktop-client-plan.md)（现有 `desktop/` 只是「截图能力容器」）、[题库与试卷](../../neural-knowledge-base/docs/product/question-bank-and-papers.md)、[试卷解析与截图出题](../../neural-knowledge-base/docs/product/question-paper-extraction-plan.md)、[多模型与用途路由](../../neural-knowledge-base/docs/product/llm-model-routing.md)、[AI 出题](../../neural-knowledge-base/docs/product/ai-question-generation-plan.md)

## 1. 结论先说

这是一个**新客户端产品**，不是现有 `desktop/` 壳的升级：

| | 现有 `desktop/` | 本文的「个人客户端」 |
|---|---|---|
| 定位 | 给已部署中心系统的人加「全局热键截图」 | 给独立个体从零搭建自己的本地题库 |
| 数据 | 全部在服务器 PostgreSQL / Milvus | **本地 SQLite + 本地图片目录**，不依赖服务器 |
| UI | 加载 `http://<server>/#/…` | 前端**打包进安装包**，离线可用 |
| 联网 | 必须联网 | AI 需要联网；其余离线可用。远程同步是可选扩展 |
| 用户 | 教师 / 教研员（已在机构内） | 小白用户（个人学习者 / 家长 / 自由教师） |

一句话架构：

```
Tauri 2 安装包
  ├─ React 前端（打包，离线）
  ├─ Rust 后端（SQLite / 文件 / 截图 / 模型调用 / 同步）
  └─ 可选：内置浏览器窗口 + 全局截图选区层
        │
        └─ HTTPS（可选）→ 中心系统 /api/desktop/**（账号登录 + 设备令牌）
```

**核心取舍**：把「解析、出题、归类、制图、修图」都做成客户端本地任务，模型调用由客户端自己发起；中心系统只承担**可选的题库同步**。这样小白用户不装服务器也能用，机构用户仍能把题目汇总到中心。

## 2. 目标用户与典型场景

| 用户 | 场景 | 现在的痛点 |
|---|---|---|
| 家长 / 自学者 | 拍一张试卷图，识别成题目，按知识点归档，之后搜索、刷题 | 没有服务器，不会部署 PostgreSQL / Milvus |
| 自由教师 | 把十几年积累的 Word / Excel / PDF 题库导入，统一格式 | 格式五花八门，手工整理不可能 |
| 小机构 | 几个人各自整理，最后汇总到中心题库 | 每人装一套服务器不现实 |
| 已有中心系统的老师 | 客户端随手录题，选一批同步到学校题库 | 现有桌面壳只能截图，离线不能用 |

**非目标（2026-11-16 局部更新）**：原定「不做班级、作业、批改、学情」已被新需求推翻——见 [本地优先教学能力规划](local-first-teaching-plan.md) 决策 D0：将以本地档案方式补班级/学生/作业/批改/学情，并与中心打通。**仍保留**：客户端是单用户登录（学生不登录本地）、多人协作只在中心系统、本地只做「个人题库 + 可选上传」的定位不变。本文后续章节若与新方案冲突，以新方案为准。

## 3. 产品边界（明确不做）

- **不内置 PostgreSQL / Milvus / Redis / MinIO**：本地用 SQLite + 文件目录，检索先用 SQLite FTS5。
- **不做多用户 / 多租户**：一个安装包一个本地库；需要多人共享就走中心同步。
- **不做题库市场 / 社交**：不提供公开题库下载。
- **不自动爬第三方网站**：内置浏览器只做「人浏览、人取题」——截图与 DOM 文字提取都由用户点击按钮单次触发，不做批量抓取、不绕过登录与付费墙。
- **不在客户端里跑中心系统的管理台**：成员、审计、模型池这些仍在服务器。
- **不承诺「AI 修复」凭空补内容**：修图只做画质与遮挡处理，原图永远保留。

## 4. 功能清单与优先级

| 能力 | 说明 | 优先级 |
|---|---|---|
| 本地题库 | 题目增删改查、搜索、标签、知识点、难度、图片附件 | **P0** |
| 统一题目包导入 | **只支持 JSON / JSONL / ZIP**；提供标准提示词让第三方智能体转换；导入前做严格格式校验与逐行报错 | **P0** |
| 截图识别题目 | 全局热键 → 选区 → 多题切分 → 结构化 → 校对入库 | **P0** |
| AI 归类 | 自动给学科、难度、知识点、答案解析；只处理还没归类的题 | **P0** |
| AI 出题 | 按知识点 / 难度 / 题型生成，预览后入库 | **P0** |
| 大模型配置 | **不内置模型**，用户自行添加多个 OpenAI 兼容端点；用途路由、连通性测试、Key 进系统钥匙串 | **P0** |
| GeoGebra 制图 | 题干 → applet 规格 → 预览 → 存为题目图 | **P1** |
| 内置浏览器采集 | 内嵌 WebView 浏览试题网，DOM 文字提取 + 截图识别双路入库 | **P0** |
| AI 修复破损图片 | 纠偏、裁边、去噪、增强、放大；AI 修复为可选下载 | **P1** |
| 远程同步题目 | 账号登录 + 设备令牌 + 增量推拉 + 图片上传 | **P1** |
| 自动更新 | 签名清单 + 应用内升级 | **P2** |
| 本地模型 | Ollama / llama.cpp / ONNX，离线 AI | **P2** |
| PDF / Word 整卷导入 | 客户端本地解析 | **P2** |

## 5. 信息架构

```
左侧导航
├─ 题库        列表 / 筛选 / 详情 / 批量操作
├─ 录入        截图识别 · 批量导入 · 手工录题
├─ 生成        AI 出题 · AI 归类 · GeoGebra 制图
├─ 浏览器      内嵌第三方试题网（可选）
├─ 同步        账号、设备、待同步、冲突处理（可选）
├─ 帮助        使用说明 · 常见问题 · 题目包格式 · 智能体提示词
└─ 设置        模型配置 · 数据目录 · 备份恢复 · 关于
```

首次启动向导（小白友好）：

1. 选数据目录（默认 `~/Documents/NeuralKB-Local`）；
2. 添加模型（**不内置任何厂商模板**）：填名称、Base URL、模型名、API Key，可加多个；不确定可点「稍后配置」，先离线录题；
3. 导入教材目录（内置一份学科 / 章节 / 知识点快照）；
4. 可选：登录中心系统，开启同步；
5. 进入题库，给一个「截一张图试试」的引导按钮。

### 5.1 使用说明与帮助

目标：**不假设用户懂 API、懂 JSON、懂数据库**。帮助内容随安装包离线提供，不依赖外网文档。

| 入口 | 内容 |
|---|---|
| 首次向导 | 5 步以内，每步一句话说明「这一步得到什么」；可跳过模型配置 |
| 帮助中心 | 快速上手、截图识别、批量导入、题目包格式、模型配置、同步、备份恢复、常见问题 |
| 页面内提示 | 空状态给下一步动作；错误提示带「查看帮助」锚点（例如「没有可用的视觉模型 → 如何添加」） |
| 模型配置说明 | 什么是 Base URL / 模型名 / API Key、如何判断模型是否支持图片、多个模型怎么排用途 |
| 智能体提示词 | §8 的转换提示词可一键复制，并附「粘到哪里、得到什么」的示例 |
| 数据安全 | 明确告诉用户：AI 识别会把题目图片发给你配置的模型服务商；本地题库不上传，除非开启同步 |
| 备份恢复 | 教用户复制数据目录、导出整库 ZIP、换机恢复 |
| 故障排查 | 401 / 429 / 超时 / 没有视觉模型 / 图片过大 / 同步冲突，各自给可执行步骤 |

落地要求：

- **界面与帮助均为中文**，不引入翻译层；文案面向零基础用户。
- 帮助正文用 Markdown 放在 `local-client/src/help/`，构建时打包；页面用统一 `HelpLink` 组件跳到对应锚点。
- 每个核心页面右上角有「?」，打开当前页的说明，而不是笼统的官网链接。
- 首次完成一件事（识别第一题、导入第一批、同步第一次）后给一次轻量成功提示，不重复打扰。
- 文案只写用户能做的事，不写数据模型与实现细节（与中心系统文案规范一致）。

## 6. 本地数据模型（SQLite）

数据库文件：`<数据目录>/neuralkb.db`，图片在 `<数据目录>/assets/`。开启 `WAL`、`foreign_keys=ON`。

| 表 | 作用 | 关键字段 |
|---|---|---|
| `meta` | 模式版本、内置目录版本、安装 ID | `key`, `value` |
| `subject` / `textbook` / `chapter` / `knowledge_point` | 本地教材目录快照 | `id`, `name`, `parent_id`, `code`, `external_id` |
| ~~`exam_point`~~ | 已取消（v10 迁移归并进 `knowledge_point`，旧表删除） |
| `question` | 题目主表 | `id`(UUID), `subject_id`, `type`, `stem`, `options_json`, `answer`, `analysis`, `difficulty`, `stem_normalized`, `source`, `year`, `status`, `origin`, `created_at`, `updated_at`, `deleted_at`, `sync_state`, `server_id` |
| `question_tag` | 自由标签 | `question_id`, `tag`, `kind`(`FREE`) |
| `question_knowledge` | 题目 ↔ 知识点 | `question_id`, `knowledge_point_id`, `weight` |
| `question_asset` | 图片附件 | `id`, `question_id`, `kind`(`PAGE`/`FIGURE`/`CAPTURE`/`GEOGEBRA`/`REPAIRED`), `relative_path`, `content_type`, `sha256`, `width`, `height`, `page_no`, `bbox_json`, `created_at` |
| `import_batch` | 导入批次 | `id`, `filename`, `source_format`, `total`, `imported`, `skipped`, `failed`, `created_at` |
| `import_error` | 逐行错误 | `batch_id`, `row_no`, `code`, `message`, `raw_json` |
| `generation_job` | AI 出题 / 归类 / 识别任务 | `id`, `kind`, `status`, `input_json`, `result_json`, `error`, `created_at` |
| `llm_model` | 模型池 | `id`, `name`, `base_url`, `model`, `supports_vision`, `enabled`, `key_ref` |
| `llm_route` | 用途 → 模型顺序 | `purpose`, `priority`, `model_id` |
| `sync_account` | 中心账号与设备 | `server_url`, `username`, `device_id`, `token_ref`, `enabled` |
| `sync_outbox` | 待推送变更（本地为 source of truth） | `id`, `entity_type`, `entity_id`, `op`, `payload_json`, `attempts`, `last_error`, `created_at` |
| `sync_link` | 本地 ↔ 服务端映射 | `local_id`, `server_id`, `server_updated_at` |
| `app_setting` | 通用设置 | `key`, `value_json` |

约定：

- **题干指纹**：`stem_normalized` 复用中心系统的归一化口径（去空白、全半角、标点），本地查重先用它，再做可选的相似度。
- **软删除**：`deleted_at` 与中心一致，避免同步时把「删了」和「没同步过」混为一谈。
- **`sync_state`**：`LOCAL`（从未同步）/ `PENDING` / `SYNCED` / `CONFLICT` / `FAILED`。
- **图片只存相对路径 + 哈希**，换数据目录不用改库。
- **密钥不进 SQLite**：`key_ref` 是系统钥匙串里的条目名。

## 7. 统一题目包格式（v1）

这是「批量导入」和「第三方智能体转换」的共同契约。目标：**人看得懂，机器校验得住，图片带得走**。

### 7.1 容器

支持三种：

| 形式 | 用途 |
|---|---|
| `questions.json` | 纯文字题库，单个文件 |
| `questions.jsonl` | 大文件流式导入，一行一题 |
| `*.zip` | 含图片：`questions.json` + `images/` + `README.txt` |

**不解析 Excel / CSV**：客户端只认上面的三种容器。手里是 Excel 的用户，先用 §8 的提示词让第三方智能体转成 JSON，再导入。这样只有一套校验口径，不会出现「Excel 能进、JSON 进不去」的两套逻辑。

ZIP 结构：

```
my-pack.zip
├─ questions.json
├─ images/
│  ├─ q1-1.png
│  └─ q2-1.jpg
└─ README.txt
```

### 7.2 JSON 结构

```json
{
  "format": "neural-question-pack",
  "version": 1,
  "generatedAt": "2026-11-16T10:00:00Z",
  "generator": "third-party-agent/1.0",
  "subject": "数学",
  "questions": [
    {
      "clientId": "0b0e7f36-7a7e-4d1e-9e2a-3f0f6a0b9c11",
      "type": "SINGLE",
      "stem": "若 $a=2$，则 $2a$ 等于多少？",
      "options": [
        {"key": "A", "text": "4"},
        {"key": "B", "text": "5"}
      ],
      "answer": "A",
      "analysis": "$2a=2\\times2=4$。",
      "difficulty": 1,
      "examPoints": ["代数式求值"],
      "tags": ["七年级"],
      "knowledgePath": ["初中数学", "七年级上册", "代数式"],
      "source": "某校月考题",
      "year": 2025,
      "images": [
        {"file": "images/q1-1.png", "kind": "FIGURE", "alt": "图1"}
      ],
      "missingImages": []
    }
  ]
}
```

字段规则（与中心系统 `QuestionImportService` 对齐）：

| 字段 | 必填 | 规则 |
|---|---|---|
| `clientId` | 否 | UUID；没有则导入时生成，用于幂等与图片命名 |
| `type` | 是 | `SINGLE` / `MULTI` / `FILL` / `SOLVED`；兼容中文「单选题 / 解答题」等别名 |
| `stem` | 是 | 题干；公式**必须**用 `$...$` 包裹；Markdown 里的图片引用会被解析成附件 |
| `options` | 选择题必填 | 数组 `{key,text}`，或字符串 `A|4;B|5`；最多 7 项（七选五） |
| `answer` | 是 | 选择题填字母（多选 `AC`）；其他填文本 |
| `analysis` | 否 | 解析，公式同样用 `$...$` |
| `difficulty` | 否 | 1–5，默认 3 |
| `examPoints` | 是 | 1–6 个，字符串数组或 `;` 分隔 |
| `tags` | 否 | 最多 12 个 |
| `knowledgePath` | 否 | 教材 / 章节 / 知识点路径，导入时尽量匹配本地目录；匹配不到落为待归类 |
| `images` | 否 | `file` 必须是 ZIP 内相对路径；`kind`=`FIGURE`/`PAGE`/`CAPTURE`；禁止 `../` |
| `missingImages` | 否 | 智能体无法导出的图片，写进这里让客户端提示用户补图 |

校验与容错：

- 逐题校验，**单题失败不阻断整包**；导入前展示「可导入 / 重复 / 相似 / 无效」四类计数和逐行原因。
- `stem`、`options`、`analysis` 里的 `![](images/...)` 会被抽取成 `question_asset`，并把正文改写成客户端本地资源引用。
- ZIP 安全：拒绝绝对路径、`..`、符号链接；解压后单文件、总大小、图片像素设上限，防 zip bomb。
- 图片只接受 jpg / png / webp / gif；按文件头嗅探，不信任扩展名。

### 7.3 导入向导

```
选择文件 → 识别格式 → 预览（题目卡片 + 错误列表）→ 图片匹配 → 去重 → 导入 → 可撤销
```

- 去重：`stem_normalized` 精确匹配 + 可选相似度（编辑距离 / 分词 Jaccard，本地算，不依赖模型）。
- 图片匹配：优先 `images` 数组；其次题干里的 Markdown 引用；最后按 `clientId` / 题号找同名文件。
- 撤销：按 `import_batch` 批量软删除，不硬删。

## 8. 第三方智能体「转换技能」提示词

给小白用户的用法：把下面这段提示词连同自己的 Excel / Word / 图片丢给任意支持文件与视觉的智能体，让它按格式输出 `questions.json`（能打包就打包 ZIP）。

提示词要点（最终版放进客户端「导入 → 用 AI 转换」里的可复制块，并单独提供 `question-pack-skill.md`）：

```text
你是题库格式转换助手。请把我提供的题目转换成 Neural Question Pack v1（JSON）。

要求：
1. 只输出一个 JSON 对象，不要解释、不要 Markdown 代码围栏外的文字。
2. 顶层字段：format="neural-question-pack"、version=1、questions=[...]。
3. 每题字段：
   - type：单选题 SINGLE，多选题 MULTI，填空 FILL，解答/证明/计算 SOLVED。
   - stem：完整题干。所有数学公式用 $...$ 包裹；反斜杠写成两个（\\frac、\\sqrt）。
   - options：选择题必须给 [{key:"A",text:"..."}]，最多 7 个；非选择题给 []。
   - answer：选择题填字母；其他填最终答案文本。
   - analysis：解析；没有就写 ""，不要编造。
   - difficulty：1–5 的整数；拿不准写 3。
   - knowledgePath / knowledgePaths：教材 / 章节 / 知识点路径，至少 1 条；旧包里的 examPoints 会按知识点名兼容读入。
   - tags：可选，最多 12 个。
   - knowledgePath：如果你能判断教材/章节/知识点，给字符串数组；不能就 []。
   - source / year：能确定才写，否则省略。
   - images：题目依赖的图，写成 [{"file":"images/题号-序号.png","kind":"FIGURE","alt":"图1"}]。
4. 图片处理：
   - 如果你能把原题里的图裁出来，保存到 images/ 目录，并在 images 里引用；
   - 如果不能导出图片，在 missingImages 里写 ["图1","图2"]，不要用文字描述代替图。
5. 不确定的题不要丢：在题目里加 "review": true，并在 "reviewNote" 说明哪里不确定。
6. 不要合并题目，不要改写题意；一题一条。
```

配套约定：

- 智能体不需要生成 UUID，客户端会补。
- 智能体输出单个 JSON 也可以：用户把 JSON 粘贴进「粘贴导入」文本域。
- 涉及图片时，**优先建议用户用客户端的截图 / PDF 导入**；第三方智能体往往拿不到原图裁剪能力，`missingImages` 就是留给这种情况的。

## 9. AI 能力设计

### 9.1 模型配置与用途路由

**不内置任何模型，也不预置厂商模板**：所有模型由用户自己添加，且支持添加多个。应用只提供「OpenAI 兼容接口」这一种通用写法与字段说明。

- **模型池**：名称（用户自定义）、Base URL、模型名、API Key、是否支持视觉、启用开关。可加任意多个，同一厂商也可以加多次（不同 Key / 不同配额）。
- **用途路由**：对话 / 截图识别 / AI 出题 / 归类 / GeoGebra / 图片修复；每个用途一个有序候选列表，失败自动换下一个。未配置的用途显示「去添加模型」，不静默失败。
- **不内置本地模型**：用户如果自己有 Ollama / llama.cpp，按 OpenAI 兼容地址（例如 `http://127.0.0.1:11434/v1`）自行添加即可；安装包不携带任何权重。
- **测试连接**：发一条 1 token 的请求，显示延迟与错误；错误原文可复制，便于用户排查。
- **参数说明内置**：设置页直接解释 Base URL（到 `/v1`）、模型名（服务商给的 ID）、API Key 从哪复制，避免小白卡在第一步。
- **Key 存储**：macOS 钥匙串 / Windows 凭据管理器（`keyring` crate）；SQLite 只存引用。**绝不回传 Key**，设置页只显示 `已保存`。
- **冷却**：429 / `rate limit` / `quota` 时该模型冷却 2 分钟，所有候选都冷却时仍尝试一次。
- **调用位置**：一律在 **Rust 侧**发起（`reqwest`），流式结果通过 Tauri 事件推给前端。避免 Key 进入 WebView，也避免渲染模型返回的 HTML 时被 XSS 顺走。

模型返回 JSON 的解析规则与中心系统一致：**先修 LaTeX 单反斜杠，再解析**（`\sqrt` 是非法转义、`\times` 会被吃成制表符）。把这套逻辑移植到 Rust，并复用同一组测试用例；不要各写一套。

### 9.2 截图识别题目

参考现有 `QuestionExtractionService.capture` 与 `question.extract.*` 提示词：

1. 全局热键 / 页面按钮 → 系统抓帧 → 置顶选区（直接复用 `desktop/src-tauri` 的 `capture.rs`、`overlay.rs`、`clipboard.rs`，抽成共享 crate）。
2. 选区图可先做基础增强（纠偏、对比度、去噪）。
3. 调视觉模型：一次截图可能含多道题 → 提示词要求按题切分，返回结构化 JSON。
4. 客户端本地校验：必填、公式、题型、选项数；不合格的进「待校对」。
5. 预览卡片逐题确认 → 入库；原截图存为 `CAPTURE` 附件，裁剪图存 `FIGURE`。
6. 单题识别失败可「重试本题」或「手工补录」，不整批回滚。

存储：识别任务进 `generation_job`，结果先落临时表，用户确认后才写 `question`，避免半成品污染题库。

### 9.3 AI 出题

- 维度：知识点 / 难度 / 题型 / 数量 / 是否带图。
- 先出**预览**，逐题显示答案、解析、知识点；确认后入库（`status=DRAFT`，`needs_review=1`）。
- 去重：生成后先跑 `stem_normalized` 与相似度，重复题标黄。
- 约束：**必须同时有答案、解析、知识点**，缺一项不允许入库（考察点已并入知识点，不再单独要求）。
- 长任务异步：进度条 + 可取消 + 失败重试。

### 9.4 AI 归类与解析

对已有题目批量补：

- 学科（在本地学科表内选，不自由发挥）
- 难度 1–5
- 知识点 1–6 个（在本地目录内选，跨学科可搜；旧考察点词表已并入）
- 答案 / 解析（仅当原题缺失时生成，并标记「AI 生成」）

后台队列逐题处理，带置信度；低置信度进「待确认」，不静默写入。

### 9.5 GeoGebra 制图

- 输入：题干 + 可选知识点 + 可选参考图。
- 输出：GeoGebra applet 规格（`material_id` 或 `ggb` 脚本 / JSON），存 `question_asset(kind=GEOGEBRA)`。
- 预览：优先内置离线 MathApps；没有离线包时用 iframe 打开 geogebra.org（需联网）。
- 安全：applet 规格只允许白名单命令；渲染在隔离窗口，不加载任意第三方脚本。
- 失败兜底：生成不合法时给「重新生成」和「手工贴 GeoGebra 链接」两条路。

### 9.6 AI 修复破损图片

分两层，避免承诺过头：

| 层级 | 做什么 | 实现 |
|---|---|---|
| **基础修复（P1，必须）** | EXIF 自动转正、纠偏、裁边、去噪、对比度/亮度、2× Lanczos 放大、去元数据 | Rust `image` + `imageproc`，纯本地，无模型 |
| **AI 修复（P1，可选下载）** | 去水印/去笔迹、补断线、超分 | 本地 ONNX（LaMa / Real-ESRGAN）或支持图像编辑的云模型 |

规则：

- **非破坏性**：原图永远保留；修复结果另存为 `kind=REPAIRED` 的新附件，可一键回退。
- 修复前后并排对比；用户确认才替换题目里引用的图。
- 模型权重按需下载（50–200MB），标注来源与许可；不默认打包进安装包。
- 不把「修复」做成「重画」：文字题面不允许模型改写。

## 10. 采集：内置浏览器与截图

### 10.1 内置浏览器

> 本节保留早期设计。当前实现以 §18.3 为准：结构化题块直接进入预览；整页文字只填入文本框，用户再次点击「整理成题目」后才调用模型。

- Tauri 独立 `WebviewWindow` 打开第三方试题网。
- **DOM 文字提取（新增）**：控制条提供「识别页面文字」按钮——注入脚本读取页面 DOM，提取结构化题干文本，走文字识别链路入库。比截图识别更准（无 OCR 误字、公式保留源码），是文字型题目的首选路径。
- **截图兜底**：「截取当前页」按钮触发选区截图，走视觉模型识别。适合图片型题目、公式图。
- **不自动翻页、不做批量抓取**、不绕过登录与付费墙；提取由用户点击按钮单次触发。
- 每个站点独立 Cookie 分区；关闭即清或按站点保留（用户可选）。
- 站点限制：登录、验证码、`X-Frame-Options`、反爬策略都可能挡住；被挡时提示「用系统浏览器打开，再按热键截图」，不视为 bug。

**DOM 提取链路**：

```
内置浏览器浏览题目页 → 点「识别页面文字」→ eval() 注入脚本读 DOM
  → 清洗去噪（脚本/样式/导航/广告）→ 提取正文文本
  → 回传主窗 → 走「粘贴文字」识别链路 → 预览 → 存草稿
```

提取策略（启发式）：
1. 优先匹配常见题目容器 class（`.question` / `.topic` / `[class*="question"]` / `article` / `main` / `.content`）；
2. 匹配不到就取 `document.body` 纯文本；
3. 保留段落结构与编号（`1.` `2.` `一、` `二、`），帮助下游切题；
4. 公式优先取 MathJax/KaTeX 源码（`annotation` 标签或 `data-tex` 属性），避免渲染图失真。

### 10.2 截图链路

直接复用现有桌面壳已验证的实现：

```
热键 → 隐藏主窗 → 抓帧（CGDisplayCreateImage / Windows GDI）→ 置顶选区
     → 标注（矩形/箭头/马赛克/文字）→ 提交 → JPEG + 透明 PNG 标注
     → 本地识别任务 → 预览入库
```

落地时把 `desktop/src-tauri/src/{capture,overlay,clipboard,perm}.rs` 抽成 `crates/nb-capture`，两个产品共用，避免两套截图逻辑各自修 bug。

### 10.3 合规

- 只做「用户主动浏览 + 主动取题（截图 / DOM 文字提取）」，不做无头抓取、不做批量下载、不绕过付费墙或登录。
- DOM 提取仅读取用户已打开页面的可见文本，不发额外请求、不存 Cookie、不模拟登录。
- 抓取的题目内容仅供个人教学使用，不得批量转售或商用，遵守网站用户协议与 robots 规则。
- 客户端内置提示：同步到中心前确认题目版权与来源；默认同步为**仅自己可见**。

## 11. 远程同步（P1，可选扩展）

### 11.1 原则

- 本地永远是 source of truth，同步失败不影响本地使用。
- 同步是**增量、幂等、可重试**的；断网、关机、重复点击都不会产生重复题。
- 默认**私有**：同步到中心后只有本人和管理员可见；用户显式勾选「共享到题库」才写 `g_all` 或进入待审。
- 传输必须 HTTPS；令牌只存在系统钥匙串。

### 11.2 鉴权：账号登录 → 设备令牌

不复用浏览器 Cookie / CSRF（桌面客户端没有浏览器会话），新增一套仅限 `/api/desktop/**` 的令牌：

```
POST /api/desktop/auth/login
  {username, password, deviceId, deviceName, platform}
→ {token, tokenPrefix, expiresAt, user:{id, username, displayName, role}}

POST /api/desktop/auth/refresh   （Authorization: Bearer …）
→ {token, expiresAt}

POST /api/desktop/auth/revoke    （Authorization: Bearer …）
→ 204
```

服务端存储（`app_user` 目前没有 `tenant_id`，设备表先按单实例处理；将来多租户再加列）：

```sql
create table desktop_device (
    id           uuid primary key default gen_random_uuid(),
    user_id      uuid not null references app_user (id) on delete cascade,
    device_id    varchar(128) not null,
    device_name  varchar(128),
    platform     varchar(32),
    token_hash   varchar(64) not null unique,   -- sha256(token)
    token_prefix varchar(12) not null,          -- 仅用于展示
    created_at   timestamptz not null default now(),
    last_used_at timestamptz,
    expires_at   timestamptz not null,
    revoked_at   timestamptz,
    constraint ux_desktop_device unique (user_id, device_id)
);
```

- 令牌：32 字节随机数，Base64URL；**只存哈希**，明文只在登录响应里返回一次。
- 有效期 180 天，使用中自动续期；管理台可查看设备并吊销。
- 过滤器：`/api/desktop/**` 接受 `Authorization: Bearer`，绕过 CSRF；其余接口行为不变。
- 管理台新增「桌面设备」列表：设备名、平台、最近使用、吊销。
- 审计：`desktop.login` / `desktop.sync.push` / `desktop.sync.pull` / `desktop.device.revoke`。

### 11.3 加密

| 层 | 方案 |
|---|---|
| 传输 | **强制 HTTPS**（TLS 1.2+）；客户端校验证书，自签部署允许用户导入 CA 指纹 |
| 本地 | 令牌 / Key 进系统钥匙串；数据库可选 SQLCipher（P2） |
| 端到端（可选） | 若要求「服务器也看不到明文」，用设备令牌派生 AES-256-GCM 密钥加密题面/图片；P2 再评估，P0 不做 |

P1 的验收口径就是「全链路 HTTPS + 令牌鉴权 + 服务端按用户隔离」。不把「自研加密」当成 TLS 的替代。

### 11.4 推送（本地 → 中心）

两步式，避免大 JSON 带 base64：

```
POST /api/desktop/assets            multipart(file) → {assetId, sha256, contentType}
POST /api/desktop/sync/push
  {
    "batchId": "uuid",              // 客户端生成，幂等键
    "questions": [
      {
        "clientId": "uuid",
        "type": "SINGLE",
        "stem": "...",
        "options": [...],
        "answer": "A",
        "analysis": "...",
        "difficulty": 3,
        "examPoints": ["..."],
        "tags": ["..."],
        "knowledgePath": ["..."],
        "subjectName": "数学",
        "assets": [{"assetId": "uuid", "kind": "FIGURE"}],
        "share": false,
        "updatedAt": "2026-11-16T10:00:00Z"
      }
    ]
  }
→ {
    "batchId": "...",
    "results": [
      {"clientId": "...", "serverId": "...", "status": "CREATED|UPDATED|SKIPPED|CONFLICT", "message": null}
    ],
    "serverTime": "..."
  }
```

服务端：

- 写入 `question`：`owner_id = 令牌用户`，`status = DRAFT`，`needs_review = true`，`origin = DESKTOP`；
- 学科按 `subjectName` 匹配中心 `subject` 表；匹配不到该题返回 `INVALID_SUBJECT`，不自动创建学科；
- `share=false` → `acl_groups = {}`（仅本人 + 管理员）；`share=true` → `{g_all}` 或进审核队列；
- 知识点按 `knowledgePath` 匹配，匹配不到留空并返回提示，不阻塞入库；
- 幂等：`desktop_question_link(device_id, client_question_id)` 唯一；同 `batchId` 重复提交直接返回上次结果。
- 附件：图片先走 `POST /api/desktop/assets` 上传，再在题目里引用 `assetId`。本地扩展的 `GEOGEBRA` / `REPAIRED` 附件同步时先映射为服务端已有的 `FIGURE` / `CAPTURE`；若产品要求保留语义，再给服务端 `question_asset.kind` 加值（新迁移），不在客户端绕过约束。

新表：

```sql
create table desktop_question_link (
    device_id         uuid not null references desktop_device (id) on delete cascade,
    client_question_id uuid not null,
    question_id       uuid not null references question (id) on delete cascade,
    server_updated_at timestamptz not null,
    primary key (device_id, client_question_id)
);
```

### 11.5 拉取（中心 → 本地）

```
GET /api/desktop/sync/pull?cursor=<updatedAt,id>&limit=200
→ {
    "items": [ { ...question..., "serverId": "...", "updatedAt": "...", "deleted": false } ],
    "nextCursor": "...",
    "hasMore": true
  }
```

- 只返回令牌用户 `owner_id = 自己` 的题（管理员可选全量）；
- `cursor` 用 `(updated_at, id)`，避免同一毫秒丢数据；
- 软删除以 `deleted=true` 下发，本地对应软删除；
- 题目附件返回清单（`assetId` + 下载地址），图片按需下载并校验 `sha256`。

### 11.6 冲突处理

| 场景 | 处理 |
|---|---|
| 本地改了、服务端没改 | 推送覆盖，`UPDATED` |
| 服务端改了、本地没改 | 拉取更新 |
| 两边都改 | **不自动合并**：保留服务端版本，本地另存一份「冲突副本」并标 `CONFLICT`，让用户选 |
| 本地删了、服务端还在 | 推送软删除（需二次确认） |
| 服务端删了、本地有未同步修改 | 保留本地为 `CONFLICT` |

原则：**宁可再生一份，不静默丢数据**。

### 11.7 同步 UI

- 状态总览：未同步 N 题 / 同步中 / 失败 M 题 / 上次成功时间。
- 逐题状态徽章：本地 / 待同步 / 已同步 / 冲突 / 失败。
- 失败可看原因（网络、401、配额、校验），支持「重试全部」「只重试本题」。
- 同步前可选范围：全部 / 某标签 / 某批次 / 勾选。
- 登录态失效时提示重新登录，不清本地数据。

## 12. 技术架构

| 层 | 选型 | 理由 |
|---|---|---|
| 壳 | Tauri 2 | 与现有 `desktop/` 一致，包体小，原生截图 / 热键 / 钥匙串 |
| 前端 | React 19 + Vite | 复用现有组件习惯与 `latexMath` / KaTeX 渲染口径 |
| 本地库 | SQLite（`rusqlite`，bundled） | 零安装、单文件、可备份；不用让用户装 PG |
| 迁移 | 内嵌 SQL + `user_version` / `rusqlite_migration` | 升级安装包时自动迁移 |
| 检索 | SQLite FTS5（题干 / 标签 / 解析） | 本地够用；向量检索 P2 再评估 |
| 文件 | 数据目录 + 相对路径 | 换机 / 备份只需拷目录 |
| HTTP | `reqwest`（rustls） | 模型与同步；Key 不进 WebView |
| 密钥 | `keyring`（macOS Keychain / Windows Credential Manager） | 系统级保护，优于明文 |
| 图片 | `image` + `imageproc`；可选 `ort`(ONNX) | 基础修复纯本地；AI 修复按需下载 |
| 数学 | 复用 `latexMath.splitLooseMath()` + KaTeX | 展示口径与中心系统一致，避免两套公式判断 |
| 截图 | 抽 `crates/nb-capture` | 与现有桌面壳共用 |
| 打包 | `.dmg`（macOS）/ NSIS（Windows） | Tauri 原生产物 |

### 12.1 代码结构（已定：本仓库 `local-client/`）

```
local-client/
├─ src/                        # React
│  ├─ pages/                   # 题库 / 录入 / 生成 / 浏览器 / 同步 / 设置
│  ├─ components/
│  ├─ api/                     # invoke 封装
│  └─ styles/
├─ src-tauri/
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ db/                   # 连接、迁移、仓储
│  │  ├─ llm/                  # 模型池、路由、JSON 修复、SSE
│  │  ├─ capture/              # 依赖仓库根的 crates/nb-capture
│  │  ├─ import/               # 题目包解析、图片入库、校验
│  │  ├─ sync/                 # 令牌、推拉、outbox
│  │  ├─ imageops/             # 修复、缩放、纠偏
│  │  └─ commands.rs
│  └─ migrations/
crates/nb-capture/             # 仓库根：desktop/ 与 local-client/ 共用
shared/
├─ question-format/            # JSON Schema + TS 类型 + 校验用例
└─ prompts/                    # 与 server 共用提示词源文件
```

`desktop/` 继续留在原位，只把截图相关模块迁到 `crates/nb-capture/` 并被两边引用；不改变现有壳的 UI 与行为。

**提示词单一来源**：把 `PromptCatalog` 里与识别 / 出题 / 归类 / GeoGebra 相关的段落抽成 `shared/prompts/*.md`，服务端从 classpath 读取、客户端打包读取，附版本号与校验和；两边不一致时启动日志告警。避免「服务端改了提示词、客户端还是旧的」。

## 13. 分期与验收

| 阶段 | 范围 | 产出 | 预估 |
|---|---|---|---|
| **P0 骨架** | Tauri + SQLite + 题库 CRUD + 搜索 + 图片附件 + 设置 + 模型配置（不内置模型）+ 帮助中心骨架 | 能离线录一道题、看图、搜索、自查帮助 | 5–8 人日 |
| **P0 导入** | 题目包 v1（JSON / JSONL / ZIP）+ 校验 + 预览 + 去重 + 图片匹配 + 批次撤销 | 1000 题导入可回滚 | 4–6 人日 |
| **P0 截图与识别** | 共享截图 crate + 选区 + 多题切分 + 视觉模型 + 校对入库 | 热键到可校对 ≤ 3 秒 | 5–8 人日 |
| **P0 AI** | 提示词抽到 `shared/prompts/`（服务端改为同源加载）+ 出题 + 归类 + 解析 + 异步任务 + 预览 | 出 10 题、归类 50 题可验收；两端口径一致 | 6–9 人日 |
| **P0 打包** | 首次向导 + 内置帮助内容 + 数据备份 + macOS 签名 / 公证 / DMG | 小白可安装、能按帮助自助 | 3–6 人日 |
| **P1 同步** | 服务端设备令牌 + 推送 / 拉取 / 图片 / 冲突 + 客户端同步 UI + 管理台设备页 | 断网重试不重复、冲突不丢数据 | 8–12 人日（含后端） |
| **P1 采集增强** | 内置浏览器 + GeoGebra（在线优先 + 离线包可选）+ 基础修图 | 浏览→截图→入库闭环 | 6–9 人日 |
| **P1 更新** | 全量自动更新（签名清单）+ 提示词 / 帮助 / 教材目录内容热更新 | 用户无感升级，内容可独立更新 | 3–5 人日 |
| **P1 Windows** | GDI/DXGI 抓帧、WebView2、NSIS、SmartScreen | Windows 包 | 4–6 人日 |
| **P2** | AI 修图（按需下载）、PDF/Word 导入、SQLCipher、同步更多内容类型 | — | 10–15 人日 |

总计：**P0 约 23–37 人日，P1 约 21–32 人日**。不含法务、证书采购与内容运营。

验收口径：

1. **离线**：断网时题库、导入、搜索、修图全部可用；AI 明确提示需要联网。
2. **导入**：1000 题 + 200 张图的包，导入成功、错误可定位到行、可整批撤销。
3. **识别**：单题截图到可校对结果 ≤ 3 秒（不含模型排队）；10 题截图中至少 8 题结构完整。
4. **同步**：同一批重复推送不产生重复题；断网 10 分钟后恢复能自动续传；冲突题不丢。
5. **安全**：任何接口 / 日志 / 页面都不回传 Key 与令牌明文；`/api/desktop/**` 无令牌一律 401。
6. **小白**：从安装到录入第一道题 ≤ 3 分钟，向导不超过 5 步。

## 14. 风险与合规

| 风险 | 影响 | 应对 |
|---|---|---|
| 第三方试题网站 ToS / 版权 | 被投诉、下架 | 只做用户主动浏览与截图；不做批量抓取；同步默认私有；文档写明 |
| 模型费用不可控 | 小白被账单吓到 | 设置里显示每次调用估算与本月累计；出题数量上限；本地模型兜底 |
| 模型返回不合规 JSON | 识别/出题失败 | 复用 LaTeX 修复 + 严格校验 + 重试 + 手工补录 |
| AI 修图篡改题面 | 题目失真 | 非破坏性、保留原图、只修图不修文字、人工确认 |
| API Key 泄露 | 盗刷 | 系统钥匙串、Rust 侧调用、日志脱敏、设置页不回显 |
| 图片体积膨胀 | 数据目录失控 | 上传前压缩、长边上限、缩略图、可清理原图（保留题图） |
| mac 公证 / Windows SmartScreen | 装不上 | 固定签名 + 公证；首次安装指引 |
| 多平台截图差异 | 选区错位 | 按物理像素算；先 macOS 验收，再 Windows |
| 同步与中心 schema 漂移 | 数据不一致 | 版本握手 + `formatVersion` + 服务端拒绝未知版本 |
| 许可 | 二进制分发前需确认 | 当前 PolyForm Noncommercial，商业交付前确认授权 |

## 15. 决策记录

### 15.1 第一轮（2026-11-16）

| 决策 | 结论 |
|---|---|
| 代码位置 | 本仓库 `local-client/` 子目录，与 `desktop/` 并列；共享 `shared/question-format/`、`shared/prompts/`、`crates/nb-capture/` |
| 首版平台 | **macOS 优先**；Windows 放 P1，先保证单平台验收通过 |
| 模型配置 | **不内置任何模型或厂商模板**；用户自行添加多个 OpenAI 兼容端点，按用途路由 |
| 使用说明 | 安装包内置 Help 中心（快速上手、题目包、模型配置、同步、故障排查），**纯中文** |
| 同步鉴权 | **账号密码换设备令牌**（`/api/desktop/auth/login`），令牌存系统钥匙串，管理台可吊销 |
| 同步默认可见性 | **仅自己与管理员**（`acl_groups = {}`）；共享到题库需用户显式勾选，并进中心待复核队列 |
| 修图首版 | **只做基础修图**（纠偏、裁边、去噪、对比度、2× 放大）；暂不做 AI 修图 |

### 15.2 第二轮（2026-11-16）

| # | 决策 | 结论 |
|---|---|---|
| D1 | 批量导入格式 | **只支持 JSON / JSONL / ZIP**；不解析 Excel / CSV。提供标准提示词，由用户用第三方智能体转换；**导入必须做严格格式校验**（字段、公式、图片引用、ZIP 路径安全） |
| D2 | 无视觉模型 | 提示「去添加支持图片的模型」+ 把图复制到剪贴板兜底；不内置 OCR |
| D3 | 语言 | **纯中文**（界面与帮助） |
| D4 | 更新方式 | **P1 做全量自动更新**（Tauri updater，签名清单）；前端代码不做热替换。提示词 / 帮助 / 教材目录等内容可独立热更新 |
| D5 | 崩溃 / 使用日志 | **完全不收集、不上报**；排障依赖用户截图、同步错误提示与页面上的错误码 |
| D6 | 同步范围 | 只同步**题目 + 图片**；笔记、试卷、练习册后续再评估 |
| D7 | 教材目录 | 随安装包**内置**，设置里可**手动检查更新** |
| D8 | GeoGebra | **在线 iframe 优先**；离线 MathApps 作为可选下载 |
| D9 | 共享策略 | 勾选「共享到题库」后进入中心**待复核队列**，管理员通过才公开 |
| D10 | 提示词抽取 | **P0 就抽到 `shared/prompts/`**，服务端与客户端共用 |
| D11 | 授权 | 个人使用免费；商业交付前确认授权（当前 PolyForm Noncommercial） |
| D12 | AI 修图 | **暂不做**，只保留入口占位 |

### 15.3 关于「热更新」

Tauri 把前端资源打进应用包，**不能安全地只替换前端代码**：绕过签名与公证、引入远程代码执行风险，也会让「代码是哪一版」无法审计。所以按 D4 处理：

| 类型 | 方式 |
|---|---|
| 应用代码 / 界面 | 全量自动更新：下载签名安装包 → 提示重启 |
| 提示词、帮助正文、教材目录、题目包 Schema | **内容热更新**：下载签名 JSON / Markdown 到数据目录，校验版本与哈希后生效，不用重启 |
| 用户题库与图片 | 永远只在本机，不参与更新 |

### 15.3b 第三轮：网页题目 DOM 提取（2026-11-17）

| # | 决策 | 结论 |
|---|---|---|
| D13 | 网页取题方式 | **DOM 文字提取优先、截图识别兜底**。题目页多为结构化 HTML，解析 DOM 比 OCR 更准（无误字、公式保留源码）；图片型题目仍走截图 + 视觉模型 |
| D14 | DOM 提取触发方式 | **用户点击按钮单次触发**（控制条「识别页面文字」），不做自动轮询、不做批量抓取；合规要求「人主动、单次」 |
| D15 | 提取范围 | 只读用户已打开页面的可见文本；不发额外请求、不绕登录/付费墙；启发式选内容容器，选不到就取 body |
| D16 | 下游链路 | DOM 提取的文字走**现有「粘贴文字」识别链路**（LLM 切题、结构化），不另建管线；与粘贴文字共用提示词与预览/草稿流程 |
| D17 | 公式处理 | 优先取 MathJax/KaTeX 源码（`<annotation encoding="application/x-tex">`、`data-tex`），避免渲染图截取失真 |

落地时保留 D13–D15 与 D17；D16 收窄为结构化提取失败后的文字兜底，且必须经用户再次点击确认才调用模型。实际链路与边界见 §18.3，不改写以上历史决策。

### 15.4 仍待定（仅实现细节，不阻塞开工）

1. 题目包 Schema 的版本兼容规则（`version` 递增时旧客户端如何提示）。
2. 备份包格式（整库 ZIP / 只导出题目包）与恢复校验。
3. 同步批量大小、重试退避与最大重试次数。
4. 帮助正文的具体目录与每篇篇幅。
5. 模型用途的默认排序（用户第一次添加多个模型后，哪个用途优先）。

## 16. 与现有系统的关系

- **不改**现有 `desktop/` 壳的行为：它继续服务「中心系统 + 浏览器 UI」的用户。
- **复用**现有截图实现、`question.extract.*` 提示词、`LlmJson` 修复规则、题目导入校验口径、KaTeX 渲染口径。
- **新增**（仅 P1 同步时）：`desktop_device`、`desktop_question_link`、`/api/desktop/**`、管理台设备页、审计动作。
- **不新增**中心系统的本地化部署要求：客户端不需要 PostgreSQL / Milvus / Redis / MinIO。

## 17. 回滚说明

本文只是规划，无数据库与代码改动。后续实施时：

- 服务端同步相关迁移单独编号，回滚语句写在迁移文件尾部（`drop table desktop_question_link; drop table desktop_device;`）。
- 客户端数据库用 `user_version` 管理，升级失败时保留旧库备份，不自动降级。
- 同步关闭只需在客户端退出登录并吊销设备令牌，不影响本地题库。

## 18. 实施记录（2026-11-16，P0，历史）

代码在 `local-client/`，与 `desktop/` 并列。跑法、数据目录、验收清单见 `local-client/README.md`。下表与 §18.1–18.2 保留 2026-11-16 的 P0 当时状态；后续能力与本次收尾见 §18.3。

| 计划项 | 状态 | 落地位置 |
|---|---|---|
| Tauri 2 + React 19 + SQLite 骨架 | 完成 | `local-client/src-tauri`、`local-client/src` |
| §6 全部表 + WAL + 外键 + 软删除 + `sync_state` | 完成 | `src-tauri/migrations/0001_init.sql` |
| 题干指纹、全半角与标点归一、bigram 相似度 | 完成 | `src-tauri/src/text.rs` |
| SQLite FTS5 trigram（建不出来退 LIKE） | 完成 | `src-tauri/src/db.rs` |
| 题目 CRUD / 搜索 / 标签 / 知识点 / 回收站 | 完成 | `repo.rs`、`pages/Library.jsx` |
| 题目包 v1（JSON / JSONL / ZIP）+ 逐题校验 + 去重 + 批次撤销 | 完成 | `import.rs`、`pages/ImportPack.jsx` |
| 题目包导出（单 JSON / 带图 ZIP，可原样回导） | 完成 | `export.rs`、`pages/Library.jsx` |
| 截图识别（系统选区 → 视觉模型 → 多题切分 → 核对入库） | 完成 | `capture.rs`、`jobs.rs`、`pages/Capture.jsx` |
| 模型池 / 用途路由 / 429 冷却 / 连通性测试 / Key 进钥匙串 | 完成 | `llm/mod.rs`、`pages/Settings.jsx` |
| LaTeX 单反斜杠修复（移植 `LlmJson` + 同一组用例） | 完成 | `llm/json.rs`（7 个用例） |
| AI 出题 / 批量归类（异步任务 + 轮询 + 缺字段不许入库） | 完成 | `jobs.rs`、`pages/Generate.jsx` |
| 帮助中心（9 篇，随包离线） | 完成 | `src/help/*.md`、`pages/Help.jsx` |
| 首次向导 5 步 | 完成 | `pages/Wizard.jsx` |
| 备份 / 换目录 | 完成 | `commands.rs` |
| 同步客户端侧（令牌、outbox、推送拉取、冲突状态） | 客户端完成，服务端未做 | `sync.rs`、`pages/Sync.jsx` |
| GeoGebra / 内置浏览器 / 修图 / 自动更新 / Windows | 未开始 | — |

### 18.1 与本文的偏差（有意为之）

1. **截图没抽 `crates/nb-capture`**：`screen.rs` 的进程内 CG 抓帧（失败退回 `screencapture`）+ 置顶选区层（`overlay.rs` / `public/overlay.*`）与 `desktop/` 是各写一份，没抽共享 crate——为了不动 desktop 已验收的热键链路。两套代码各自修 bug，界面一致。早先 `capture.rs::region` 直接调 `screencapture -i` 的旧方案已废弃删除，当前截图统一走 `commands::capture_region`。
2. **提示词只共享两段**：`shared/prompts/question.system.json.md`、`question-pack-skill.md` 用 `include_str!` 编进客户端。识别 / 出题 / 归类三段是客户端专用（**一图多题先切分**、账号级归类），与 `PromptCatalog` 的「一图一题」口径不同；强行同源会改坏服务端。理由写在 `shared/prompts/README.md`。
3. **识别不做 bbox 裁图**：P0 把原截图整体存 `CAPTURE`，`FIGURE` 由导入包与手工录题提供。裁剪属于 P1（方案 §9.2 第 5 步）。
4. **同步服务端未实现**：`/api/desktop/**`、`desktop_device`、`desktop_question_link`、管理台设备页都还没做。客户端会明确报「服务器没有该接口」，不假装成功。

### 18.2 验证

- `cargo test`：全量用例通过；用例数随功能迭代增长（截至本文末尾 `cargo test -- --list` 约 120 条，含各模块单测与 `flow_tests` 端到端流程）。
  端到端四条在临时目录里跑真库：冷启动建库灌快照、`cases.json`「紧凑选项」包导入→判重→撤销进回收站→恢复、手工题 FTS 检索与 LaTeX 反斜杠保真、备份 ZIP 内容校验（含 `neuralkb.db` 与 `assets/`，不含 `backup/` 与 WAL 影子文件），以及「带图包导入 → 导出带图 ZIP → 原包再导入」的回环（图片、多知识点、图注、同 id 覆盖语义）。
- 回环测试顺手查出并修掉三个真问题：`question_asset` 在题目入库前插入导致带图包必然 `FOREIGN KEY constraint failed`；导入改写图片引用时丢掉 `alt` 图注；包内 `clientId` 命中已有题时附件被追加成两份。
- `npm run build` 通过；`npm run app:build`（= `scripts/package.sh`）一条命令产出约 6 MB 的 `.dmg`，`hdiutil verify` 校验 VALID，挂载后含 `NeuralKB Local.app` 与 `/Applications` 链接，并打印体积与 SHA-256。体积随版本浮动，以构建末尾打印值为准。
- `bundle.targets` 只保留 `["dmg"]`，DMG 盘面布局写在 `bundle.macOS.dmg`（窗口 660×400、图标坐标）。
- 打包脚本刻意绕开 Tauri 内嵌的 create-dmg：那一步要驱动 Finder 摆图标，无 Finder 自动化权限时会失败，而 `.app` 其实已产出。脚本改为 `tauri build --bundles app` + `scripts/make-dmg.sh`（纯 `hdiutil create/convert`），报错形态记在 `local-client/README.md`。
- 添加/编辑模型的「模型名」支持从服务商拉取列表选择：后端 `model_list_remote`（`GET {base}/models`，Key 优先取表单、其次钥匙串，兼容 `{"data":[{"id":…}]}` / `{"models":[{"name":…}]}` / 纯数组三种返回并排序去重），前端 `ModelPicker` 组件（输入即过滤、键盘上下选择、弹层 fixed 定位避开 modal-body 裁剪），设置页与首启向导共用；Base URL 误填到 `/chat/completions` 也会剥掉后缀。
- release 包（`bundle/macos/NeuralKB Local.app`）实际启动验证过：向导第 1 步、侧边栏七项、题库页统计均正常渲染，不是白屏。
- 冷启动会在 `~/Documents/NeuralKB-Local` 建库并灌入 9 个学科、139 个知识点。
- 仍需人工验收：录题的 KaTeX 渲染、⌘⇧O 多题识别（都要真模型）、设置页「无视觉模型」提示。

### 18.3 录题、OCR 与网页取题收尾（2026-11-17）

本次除收拢 `local-client/` 既有链路外，新增 `0010_merge_exam_points.sql`（考察点并入知识点）；中心系统未改。

| 链路 | 已落地行为 | 位置 |
|---|---|---|
| 手工录题与草稿复核 | 草稿允许缺答案、解析、知识点；手工直接入库和草稿复核统一要求三项齐全。单选答案只能对应一个真实选项，多选不能含不存在的选项；批量复核任一题不合格会整批回滚。配图保存失败沿用当前题目与表单重试，不重复建题 | `repo.rs`、`components/QuestionEditor.jsx`、`components/DraftEditor.jsx`、`pages/Library.jsx` |
| 四个互斥列表 | 题库列表（已入库）/ 草稿箱 / 驳回列表 / 回收站；已驳回不再出现在题库列表 | `repo.rs::list`、`pages/Library.jsx` |
| 批量归类 | 只列未归类的草稿；候选题干走 KaTeX 渲染，已处理的不再展示 | `repo.rs::list`、`pages/Generate.jsx`、`lib/Math.jsx` |
| 题目包导入 | 重复 / 相似题默认不选；手动勾选后还必须通过确认框显式确认。取消预览或替换题目包会清理解包临时文件；既有批次撤销语义不变 | `import.rs`、`pages/ImportPack.jsx` |
| OCR 与图片提交 | `capture` / `capture_batch` 写题前预校验所有已选原图，任一缺失、不可读或格式不支持则整批不写题。附件写入异常会列出条目与题目 id；部分题目未存时保留原文、不自动翻页，避免已存题目被重复提交 | `jobs.rs`、`pages/Capture.jsx` |
| 结构化网页取题 | 只读当前页可见内容，隐藏答案 / 模板不混入；保留 MathJax / KaTeX 源码。可见图片只提示截图补图，不自动下载。前后端均限制一次最多 60 题并保留总题数；来源一律取浏览器窗口真实 URL，覆盖网页上报值 | `browser.rs`、`commands.rs::browser_collect` |
| 工具条与文字兜底 | 工具条先监听真实结果事件再注入脚本，收到题目或文字后才报完成；注入失败、页面错误与 20 秒超时都会恢复按钮。结构化失败时整页文字只填入「粘贴文字」文本框，用户再次点击「整理成题目」才创建 `capture_text` 模型任务 | `public/browserbar.js`、`commands.rs`、`pages/Capture.jsx`、`App.jsx` |
| 远程权限边界 | 第三方网页仍只能调用 `browser_collect` 与 `emit_browser_text`，没有增加本机命令或事件权限；坚持用户主动打开、单页提取，不自动爬站 / 翻页，不绕登录、验证码、付费墙 | `capabilities/remote-browser.json`、`permissions/browser.toml` |

自动化覆盖新增正式入库门槛、选择题答案、重复 / 相似导入确认、OCR 缺图 / 坏图零新增与原图附件、网页字段与 60 题边界、真实来源 URL、隐藏内容与工具条状态。结构化脚本另以本机临时页面和 Headless Chrome 验证标准题块、紧凑选项、公式源码、隐藏内容、含图提示、超量截断、文字兜底和失败恢复；临时页面与浏览器配置均已清理。未把临时页面验收冒充真实第三方网站验收。

