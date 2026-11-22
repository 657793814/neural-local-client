# 本地优先教学能力规划：班级 / 学生 / 学情 / 课件 / 拍照批改 / 取题优化

> 状态：**规划中，未动工**（本文档不改代码、不改迁移）｜提出 2026-11-16｜目标：macOS 本地客户端 + 中心系统两端打通
>
> 关联：[个人本地题库客户端规划](local-question-bank-client-plan.md)（原边界明确「不做班级/作业/批改/学情」，本文正式推翻，见 §2）、[学情闭环](../../neural-knowledge-base/docs/product/learning-loop.md)、[主观题批改与班级作业](../../neural-knowledge-base/docs/product/subjective-grading-and-class-assignments.md)、[AI 自动生成课件](../../neural-knowledge-base/docs/product/ai-courseware-generation-plan.md)、[本地实验课件化](../../neural-knowledge-base/docs/product/lab-courseware-plan.md)、[试卷解析与截图出题](../../neural-knowledge-base/docs/product/question-paper-extraction-plan.md)

## 1. 结论先说

这三件事一起做，但**不是并行铺开**，有明确先后：

```
阶段 0  同步底座（/api/desktop/** 从零建 + 本地表映射）—— 一切「两端打通」的前置
阶段 1  班级 + 学生管理（本地优先，最小闭环：建班 → 加人 → 布置题目 → 提交 → 批改 → 学情）
阶段 2  拍照批改作业（在阶段 1 的提交/批改链路上挂图片附件，不做平行系统）
阶段 3  自制课件（先做「本地课件库 + 导入/预览/布置」，再做轻量编辑器）
横切    取题体验优化（独立快车道，可与阶段 0 并行开工，不依赖中心）
```

三条硬原则，写进后续所有实施：

1. **本地优先**：所有能力先在 `local-client` 单机可用；中心是「可选汇总端」，断网不影响本地批改与看学情。数据来源永远只有一个——本地库是老师自己的账本，中心存的是「同步副本 + 学生本人视角」。
2. **复用中心已有模型，不造第二套**：中心 `class_group` / `class_member` / `assignment` / `attempt` / `study_progress` / `wrong_book` 是成熟的；本地新增表与之做**字段对齐 + `sync_link` 映射**，不另起炉灶。学情公式两端共用同一份规格（见 §6.1）。
3. **不把多用户带进本地客户端**：本地仍是单用户（`local_user` id=1）。「学生」在本地是**档案记录**（`local_student`），不是能登录的账号；需要学生自己答题/看学情时，走中心 `app_user(STUDENT)` + 邀请码入班。这一步必须在阶段 0 定死，否则后面全乱。

## 2. 为什么要推翻旧边界

`docs/product/local-question-bank-client-plan.md` §2 写「非目标：不做班级、作业、批改、学情、多人协作」，§3 写「不做多用户 / 多租户」。本次用户明确要求补上这些能力，且要求两端打通。**新增决策 D0：局部推翻旧边界**——推翻的是「不做班级/学生/学情/作业/批改/自制课件」，**保留**的是：

- 本地客户端仍是单用户登录门禁，不引入多用户会话、不引入租户模型；
- 学生账号与协作仍只在中心系统，本地只存学生档案与作答记录；
- 服务端 `/api/desktop/**`、设备表仍是新增，未实现前本地功能全部单机可用。

旧文档 §2/§3 与本文冲突处以本文为准；实施时同步在旧文档加一行「教学能力见本文」。

## 3. 现状盘点（以代码为准，2026-11-16）

| 能力 | 中心系统已有 | local-client 已有 | 缺口 |
|---|---|---|---|
| 班级 | `class_group`（6 位邀请码 `join_code`）/ `class_member`，`ClassController` 8 个端点（`server/.../edu/ClassController.java:43-76`） | 无 | 本地建班/成员管理/邀请码生成、两端映射 |
| 学生 | 无独立实体；`app_user.role=STUDENT` + `class_member.role_in_class`（`V5__education.sql:10`） | 无 | 本地学生档案（不登录）、批量导入/移除、与中心账号绑定 |
| 作业 | `assignment` / `assignment_question` / `attempt`，发布到班级（`ClassController.java:73-115`） | 无 | 本地布置、离线提交、两端同步 |
| 批改 | 单题 `/attempts/{id}/grade`、整份 `/submissions/{id}/grade`，`attempt` 存 score/max_score/remark/graded_by/graded_at/correct（`ClassService.java:591-700`） | 无 | 本地批改界面、图片作答附件 |
| 拍照/图片作答 | **完全没有**：`answer` 是纯文本（`V5__education.sql:150`），`attempt_submission` 事实表无图片列；`question_asset` 是「题里的图」不是「学生交的图」 | 无 | 新建作答附件表 + 存储 + 拍照上传 + 看图批改 |
| 学情 | `study_progress(user,knowledge_point,mastery)`；`/mastery/me`（衰减加权）、`/mastery/class/{id}`（实时按正确率重算，口径不同）；`wrong_book` | 无 | 本地学情、per-student 下钻、知识点矩阵、趋势、两端口径统一 |
| 课件 | `courseware-lab` Node CLI 编译器（spec→四块→单文件 HTML + 五道闸），`public/courseware` 11 份静态成品 + `studio.js` 运行时；前端 `CoursewareLibrary.jsx` 硬编码清单、**零制作 UI** | 无 | 本地课件库、导入/预览/布置、轻量编辑器、动态清单 |
| 取题 | `QuestionExtractionController` 整卷 PDF/图片解析（原卷 30 分钟内存保留、`question_asset`） | `browser.rs` + `browserbar.*`（只浏览不读 DOM）、`smart.rs`（Otsu 投影）、`overlay.*`（选区标注）、`jobs.rs::run_capture`（视觉模型） | 见 §7 的端到端重构 |
| 两端通道 | **`/api/desktop/**` 全库零命中**；无 `desktop_device` 表；`SecurityConfig` 无 Bearer 分支 | `sync.rs` 单侧实现（令牌/outbox/推拉），明确报「服务器没有该接口」 | 阶段 0 从零建服务端 |

中心当前最大迁移是 **`V97__llm_models_and_routes.sql`**，教育模型相关迁移为 V1 / V5 / V33 / V34 / V35 / V44 / V94 / V95。新增一律递增，不改历史迁移。本地当前迁移 `0001_init.sql` + `0002_local_user.sql`，新增从 `0003_teaching.sql` 起。

## 4. 阶段 0：同步底座（前置，不可跳过）

### 4.1 身份模型（决策 D1）

本地老师与中心老师是同一个人，用设备令牌打通；本地学生默认**不是**中心账号：

```
local_user(本机 id=1)  ──设备令牌──>  app_user(中心, TEACHER)
local_student(本地档案) ──可选绑定──>  app_user(中心, STUDENT)   ← 学生要自己答题时绑
local_class            ──映射────>  class_group
```

- 老师首次在「设置→同步」登录中心，走 `POST /api/desktop/auth/login`（用户名密码换 32 字节 Base64URL 令牌，只存哈希，180 天，可吊销），沿用现有规划 §11.2 的 `desktop_device` 表结构，不动 `app_user`。
- 学生分两类：
  - **本地档案生**（默认）：只有 `local_student`，作答记在本地，老师离线批改；同步到中心时只上报统计与匿名化记录，不给账号。
  - **中心绑定生**（可选）：老师把本地学生「绑定」到中心已有 `app_user(STUDENT)`（按用户名/手机号查找），或学生用 6 位邀请码自行入班。绑定后作答可写到中心 `attempt`，学生本人能从中心看到作业与学情。
- 未绑定的本地学生不产生中心 `app_user`，避免给未成年人批量建账号。

### 4.2 服务端新增（全部在 `/api/desktop/**`，与现有 `/api/edu/**` 并存）

| 接口 | 作用 | 要点 |
|---|---|---|
| `POST /api/desktop/auth/login|refresh|revoke` | 设备令牌 | Bearer 过滤只作用于 `/api/desktop/**`，其余接口 CSRF 行为不变；审计 `desktop.login` |
| `POST /api/desktop/assets` | 上传图片（作答照片、课件 HTML、截图） | multipart，返回 `{assetId, sha256, contentType}`；复用 `ObjectStorage`（MinIO），key 按设备分区 |
| `POST /api/desktop/sync/push` | 增量上报 | `{batchId, entities:[{type, clientId, op, payload}]}`；`type ∈ question/student/class/assignment/attempt/grade/progress/courseware/capture_source`；幂等键 `(deviceId, clientId)` + `batchId` 重放返回上次结果 |
| `GET /api/desktop/sync/pull?cursor=&types=` | 增量下发 | `(updated_at,id)` 游标；只下发该令牌用户自己的数据；软删除以 `deleted=true` 下发；图片按需下载并校验 sha256 |
| `GET /api/desktop/sync/status` | 服务端指针、上次批次、冲突数 | 前端状态页用 |
| `POST /api/desktop/students/bind` | 本地学生绑定中心账号 | 老师发起，按中心 `app_user` 用户名查；绑定后中心 `class_member` 补一条 |

新表：`desktop_device`、`desktop_question_link`（已有规划）、`desktop_student_link`、`desktop_class_link`、`desktop_attempt_link`、`desktop_courseware`、`desktop_asset`。所有表带 `device_id`，中心按令牌用户隔离，不跨设备可见。

### 4.3 客户端改动

`local-client/src-tauri/src/sync.rs` 目前只认题目 outbox；改为**通用实体 outbox**（`entity_type` 已经存在，扩展 `payload_json` 的 schema 与 push/pull 分发），新增 `sync_entities` 表记录每类实体的本地版本号与映射 id。命令新增 `sync_status_v2` / `sync_bind_student`；`pages/Sync.jsx` 增加「班级/学生/学情/课件」各类待同步计数。迁移：`0003_teaching.sql` + `0004_sync_entities.sql`（或合并，实施时定）。

## 5. 阶段 1：班级与学生管理（本地优先闭环）

### 5.1 本地数据模型（`0003_teaching.sql`）

| 表 | 关键列 | 说明 |
|---|---|---|
| `local_student` | `id`(UUID), `name`, `subject_ids`(JSON), `grade`, `note`, `center_user_id`(可空), `status`(ACTIVE/ARCHIVED), `created_at`, `updated_at`, `deleted_at` | 本地学生档案；姓名 + 可选年级/科目/备注；`deleted_at` 软删除 |
| `local_class` | `id`, `name`, `subject_id`→subject SET NULL, `term`, `join_code`(唯一, 6 位), `status`(ACTIVE/CLOSED), `created_at`, `updated_at`, `deleted_at` | 班级；邀请码与中心同字符集（`ABCDEFGHJKLMNPQRSTUVWXYZ23456789`） |
| `local_class_member` | `class_id`→local_class CASCADE, `student_id`→local_student CASCADE, `joined_at` | 复合 PK；一个学生可进多班 |
| `local_assignment` | `id`, `class_id`→local_class CASCADE, `title`, `type`(PRACTICE/EXAM), `description`, `due_at`, `late_hours`, `status`(DRAFT/PUBLISHED/CLOSED), `created_at`, `updated_at`, `deleted_at` | 作业；字段对齐中心 `assignment` |
| `local_assignment_question` | `assignment_id`→local_assignment CASCADE, `question_id`→question CASCADE, `ord`, `score`(默认1) | 复合 PK；组卷时从本地题库选 |
| `local_attempt` | `id`, `assignment_id`→local_assignment CASCADE, `question_id`→question, `student_id`→local_student CASCADE, `submission_id`, `answer`, `duration_ms`, `submitted_at`, `correct`(NULL=待批), `score`, `max_score`, `remark`, `graded_by`, `graded_at`, `late`, `superseded_at` | 作答；字段与中心 `attempt` 一一对应；客观题自动判分、主观题留空待批 |
| `sync_link` | 已有 `local_id/server_id/server_updated_at` | 复用，按 entity_type + local_id 联合查 |

约定：所有查询带 `deleted_at is null`；软删除不硬删（与题库一致）；`question` 被作业引用时删除走题库已有保护（先回收站再彻底删，有引用返回 409）。

### 5.2 客户端 Rust 模块与命令

新增 `src-tauri/src/teaching.rs`（仓储）+ 命令，全部注册进 `main.rs::generate_handler!`：

```
student_list / student_save / student_delete / student_import / student_bind_center
class_list / class_save / class_close / class_members / class_add_member / class_remove_member / class_regenerate_code
assignment_list / assignment_save / assignment_publish / assignment_close / assignment_detail
attempt_submit / attempt_list / attempt_grade / attempt_batch_grade / attempt_retry
```

- `student_import`：CSV / 通讯录 vCard 可选，纯本地解析（姓名 + 年级 + 备注），逐行报错、预览后导入；不引入 Excel 依赖。
- `attempt_submit`：客观题按 `question.answer` 本地判分（`text.rs` 已有题型归一化），主观题 `correct=NULL`；按 `submission_id` 聚合一次提交，`superseded_at` 标记旧次。
- `attempt_grade` / `attempt_batch_grade`：复用中心规则 `0<=score<=max_score`、`correct` 缺省按 `score>=max_score*0.6` 推导、写 `graded_by`（本地老师名）与 `graded_at`，触发学情重算 + outbox 入队。

### 5.3 前端页面与导航

`App.jsx` 侧栏从当前 5 项扩到 7 项：题库 / 录入 / **班级** / **学情** / 生成 / 导入 / 同步（设置/帮助仍在头像菜单）。

- `pages/Classes.jsx`（新）：两个 Tab「班级」「学生」。班级卡显示人数、作业数、邀请码（可复制/重置）；详情抽屉显示成员表，支持添加/移除、批量导入学生、绑定到中心账号。学生 Tab 支持搜索、按科目/年级筛选、归档。
- `pages/Assignments.jsx`（新，可从班级卡进入）：从题库选题、设分值/截止时间/说明、发布；提交列表按学生聚合，逐题/整份批改复用 §5.2 命令。
- 与中心 `LearningFlow.jsx` 的差异：本地页永远是「老师视角」，学生视角只在中心。

### 5.4 验收

1. 断网状态：建班 → 导入 20 名学生 → 从题库选 5 题布置 → 本地提交（含主观题）→ 逐题给分 → 学情刷新 → 重启应用数据仍在。
2. 同步开启后：同一班/学生/作答推送到中心，重复推送不产生重复记录；中心绑定学生后从中心能看到同一份作业。
3. 软删除的学生/班级不出现在任何列表与统计。

## 6. 学情管理（阶段 1 尾巴 + 阶段 2 增强）

### 6.1 统一公式（决策 D2：两端共用一份规格）

中心今天有两套口径：`/mastery/me` 用 `study_progress` 的衰减加权（`ClassService.java:1670-1713`：`weight=1/(1+days/14)`，答对系数 `0.75+difficulty*0.05`、答错 `0.05`），`/mastery/class/{id}` 用实时正确率（`ClassService.java:1496-1600`）。规划要求**统一到前者**，值与档位沿用 85/40（`MASTERED/LEARNING/WEAK`）。

落地方式：把公式写成一份规格文档放 `shared/mastery/`（公式 + 阈值 + 5 组固定测试向量），中心 `ClassService` 与本地 `teaching.rs` 各自实现并跑同一组向量测试；`classMastery()` 改为读 `study_progress` 聚合而非实时重算，保持与 `/mastery/me` 一致。这是一次**行为变更**，实施时要跑回归并告知用户历史数值会变。

### 6.2 本地新增视图

| 视图 | 粒度 | 来源 |
|---|---|---|
| 学生学情 | 学生 × 知识点矩阵 | `local_attempt` → 统一公式本地算（不联网） |
| 班级学情 | 班级 × 知识点：掌握率、共性错题 Top5、待批改数、平均分/得分率 | 矩阵聚合 |
| 趋势 | 按周/按作业快照 | 新增 `local_mastery_snapshot(student_id, kp_id, mastery, at)`，每次重算写一条，画折线 |
| 错题本 | `local_wrong_book(student_id, question_id, source_attempt_id, state, next_review_at, review_count)` | 提交时自动写，与中心 `wrong_book` 同状态机 NEW/REVIEWING/MASTERED |

前端 `pages/Insights.jsx`（新）：默认班级视图（矩阵热力 + 共性错题 + 待批改），点学生下钻到个人知识点列表与趋势；薄弱知识点可直接跳「生成」页出同类题、或跳题库筛选。中心 `LearningFlow.jsx::MasteryPanel` 同步加 per-student 下钻与趋势，复用同一接口契约 `/api/edu/mastery/class/{id}/students/{studentId}` 与 `/api/edu/mastery/trend`。

### 6.3 验收

同一组作答在本地与中心跑出相同 mastery（误差 ≤1）；未批改的主观题不计入但显示「待批改 N 题对学情暂不计入」；连续三次作答能看到趋势折线变化。

## 7. 阶段 2：拍照批改作业

**不另起批改系统**：在 §5 的 `local_attempt` / `submission_id` 上挂图片附件，复用 `attempt_grade` 与学情重算。

### 7.1 两种拍摄入口

> 学生端「手机拍照上传」的详细方案（局域网扫码 → 本地临时服务 → 现有 `grading_photo_add` 链路，以及中心中转备选）见 [手机拍照 → 本地批改联动规划](local-phone-photo-submission-plan.md)。核心结论：阶段 A 扫码上传纯本地、断网可用、照片不出内网；阶段 B 中心中转单独排期。

| 入口 | 场景 | 链路 |
|---|---|---|
| **本地拍整页作业**（老师端） | 纸质作业收上来，一叠拍照 | 连续拍照 → 每张图进 `local_asset`（`kind=SUBMISSION_PAGE`）→ 选学生/班级 → 可选智能切题（复用 `smart.rs`）→ 挂到该学生本次 submission |
| **中心学生拍照提交**（学生端，已实现） | 学生在家拍照上传 | 中心 `submission_asset`（`V99`）+ `POST /api/edu/assignments/{id}/assets`（与文字答案互不依赖）；本地「中心照片」绑定中心作业 → `GET /api/desktop/edu/assignments/{id}/assets` 拉图并按学生落成本地整页照片（见 §15.11） |

### 7.2 本地新增表与命令

```
local_asset              id, kind(PAGE/FIGURE/CAPTURE/SUBMISSION_PAGE/REPAIRED), relative_path,
                         content_type, sha256, width, height, page_no, bbox_json, created_at
local_submission_asset   submission_id, asset_id, ord          -- 一张照片可挂多题，一题可有多张照片
local_attempt_asset      attempt_id, asset_id, ord             -- 切分后的题级作答图
```

命令：`submission_photo_add`（从拍照/选图/粘贴落盘）、`submission_photo_split`（整页 → 多块，复用 `smart_regions` 并允许手工拖拽框）、`submission_photo_attach`（挂到学生/题）、`attempt_asset_data`（读图给批改 UI）、`submission_photo_review`（确认/重拍）。

拍照来源：`capture_region` 已有屏幕选区流程；新增 `photo_capture`（Tauri 插件调用系统相机/连续拍照，macOS 用 `AVFoundation` 或系统扫描仪 API，实施时选一），无相机时降级为选图/粘贴。

### 7.3 批改界面

`pages/Grading.jsx`（新）：左「作答照片 + 原题干对照」，右「逐题分数/评语」，顶部「上一份/下一份/待批改 N」。主观题默认只认 `SOLVED` 的局限要修：题型判定改为 `text.rs` 的 `is_subjective()`，不再硬编码 `['SOLVED']`（中心 `LearningFlow.jsx:719` 同改）。给分后写 `attempt.score/correct/remark/graded_by/graded_at` 与 outbox，学情按 §6.1 重算。

### 7.4 验收

一份 3 道主观题的手写作业拍照 → 切 3 块 → 挂到学生 → 逐题给分 + 评语 → 学情更新 → 中心同步后学生在中心看到分数与照片；断网全程可用，联网后补推。

## 8. 阶段 3：教师课件（讲义式，与题库/模型打通）

### 8.1 能力边界（决策 D3′：课件 = 讲义，不是 3D 仿真）

**2026-11-17 用户澄清**：本地题库里的「课件」指**教师讲义**——围绕知识点讲授的教学材料：
知识点讲解、例题精讲、变式拓展、拔高、思维训练，配合板书式排版投屏/打印给学生看。
它与知识库侧 `courseware-lab` 的 3D 交互仿真课件（`public/courseware` 11 份）**不是一回事**：
后者是可操作的实验/现象仿真（抛体、海陆风、电解），前者是「讲授 + 练题」的教案/学案。
原 §8 的 Lesson Spec（主线环、步骤五件套、站位、运镜）属于 3D 仿真课件的规格，讲义不套用。

讲义课件的核心价值在**与本地题库和模型池打通**：

> **2026-11-18 边界更新**：AI 不再只做补题，而是扩展为「变量提示词生成整份讲义 + 批量整理已有 DOCX/PPTX/PDF + 统一校对与多格式导出」。详细规格与实施记录以 [本地 AI 课件](local-ai-courseware-plan.md) 为准；题目仍须校对，不直接正式发布。

- **例题/变式/拔高/思维训练都指向本地题库的题**（`question_id` 复用），学情、错题本、作业都能沿用同一套题；
- **模型生成与整理都先出草稿**：整份讲义、变式题、拔高题、思维训练题与解题步骤提示都经老师校对，不静默替老师决定讲什么；
- **离线可用**：编译出的单文件 HTML 断网可看可打印；题干/解析在生成时内联进讲义，不依赖运行时取数。

因此 P4 拆为两步：

- **8.1a 讲义库（先做）**：把自己已有的单文件 HTML 讲义/课件导入本地，填标题/学科/知识点/简介，
  本地 iframe 预览、打印、按班级布置。同时也兼容导入 `courseware-lab` 的 3D 成品（只是不生成）。
- **8.1b 讲义生成器（后做，本次澄清后成为主体）**：表单填讲义骨架（课题/学段/知识点/教学目标/易错点），
  从题库挑例题与练习题，一键编译成讲义 HTML（含「知识清单 / 例题精讲 / 变式拓展 / 拔高 / 思维训练 /
  小结与作业」六段）；可让模型补变式题与思维训练题，产出过闸后写回题库或只挂讲义。
- **不做**：WYSIWYG 拖拽排版器、把 Node 工具链塞进安装包、运行时调用模型的「活讲义」。

### 8.2 讲义规格与数据模型

讲义骨架（`HandoutSpec`，存 `local_courseware.lesson_spec_json`）：

```
title / subject / grade / chapter / knowledge[]     -- 课题、学段、章节、知识点
objectives[]                                        -- 教学目标（3 条左右）
warmup                                              -- 情境导入（一段话或一个现象）
points[]   : { name, summary, detail, misconception, formula? }
                                                      -- 知识清单：概念、要点、易错提醒、公式
examples[] : { questionId?, stem, answer, analysis, steps[] }
                                                      -- 例题精讲（优先挂题库题）
variants[] : 同上结构                                  -- 变式拓展（可由模型从例题生成）
challenge[]: 同上结构                                  -- 拔高
thinking[] : { prompt, hint, answer? }                 -- 思维训练（开放题，可只给提示）
summary / homework[]                                  -- 小结与作业（作业可挂题库题 id）
```

表：

```
local_courseware            （沿用 0006）source ∈ IMPORT / LOCAL_BUILD
local_courseware_question   courseware_id, question_id, role(EXAMPLE/VARIANT/CHALLENGE/HOMEWORK),
                            ord, primary key(courseware_id, question_id, role)
```

命令（沿用 8.1a 的 import/list/preview/assign，新增/改造 build）：

| 命令 | 说明 |
|---|---|
| `courseware_import` | 导入单文件 HTML（讲义或 3D 成品），静态扫描外链脚本，落 `assets/courseware/` |
| `courseware_list` / `courseware_update` / `courseware_delete` | 库维护 |
| `courseware_preview` | 自定义协议 `nbasset://` 隔离 iframe 预览（不开放整个数据目录） |
| `courseware_build` | `HandoutSpec` → 五道闸 → 单文件 HTML，落库（`source=LOCAL_BUILD`） |
| `courseware_ai_draft` | 模型池补变式/拔高/思维训练题（走「AI 出题」用途，产出 `draft` 待老师确认） |
| `courseware_assign` | 布置到班级（`local_class_courseware`） |

五道闸（编译器前后各一次）：结构与必填 → 题目引用（questionId 必须在库或题干齐全）→
篇幅与体量（一节课 6–10 段为宜）→ 打印可用（CSS 无外链、A4 分页）→ 产物自检（体积/锚点/无外链）。

### 8.3 验收

1. 从题库挑 2 道例题 + 1 道变式 → 编译讲义 → 预览可见「知识清单/例题精讲/变式拓展/拔高/思维训练/小结」
   六段、题干解析内联 → 打印预览 A4 分页正常 → 断网仍可打开；
2. 让模型补 2 道变式题与 1 道思维训练题 → 草稿出现在编辑器里 → 老师改后编译过闸；
3. 导入一份外部 HTML 讲义 → 预览可交互 → 布置到某班；导入含外链脚本的文件被拒绝并给出原因。

## 9. 横切快车道：取题体验重构（用户明确「不好用」）

### 9.1 现状问题（端到端，代码可证）

现在取一道题的路径是：录入页填 URL → `browser_open` 开独立 WebView + 置顶 `browserbar` → 人在网站翻页 → 点「截取当前页」 → `capture_region` 隐藏三个窗口 → 抓帧 → 置顶选区拖框/标注 → 提交 → 回主窗 → 点「开始识别」 → 视觉模型 → 逐题核对 → 入库。全链路跨 4 个窗口、7 次点击以下这些坑：

| # | 问题 | 代码证据 |
|---|---|---|
| 1 | **不读页面内容**：浏览器只浏览，`normalize_url` 后不注入脚本、不读 DOM、不自动翻页 | `browser.rs:1-4`、`commands.rs:1005-1022` |
| 2 | **来源即丢**：URL 只在 React state，关浏览器就没了；`question.source` 不自动填，无法按来源筛选 | `Capture.jsx:28,115-123` |
| 3 | **无批量/连续取题**：一次截取一个 job，取下一题要重新框选→识别→核对→入库；无队列、无翻页连拍 | `browserbar.js:47-70`、`capture_region` 180s 超时 |
| 4 | **智能裁剪脆弱**：写死 Otsu + 投影阈值（白底黑字），两栏/手写/深色主题失效，最多 40 块；点块后**替换整图**，不能回退或累加 | `smart.rs:20-23,127`、`Capture.jsx:152-163` |
| 5 | **强依赖视觉模型**：没有 OCR / 文本兜底，纯文字页也要发图；无模型就完全不能取题 | `jobs.rs:374-383`、`help/capture.md` |
| 6 | **切题靠模型自由发挥**：提示词要求按题号切，题号近/分栏/手写批注就切歪，只能重截 | `prompts.rs:25-33` |
| 7 | **登录/验证码墙**：内置 WebView 被挡时唯一出路是「系统浏览器 + ⌘⇧O」，体验割裂 | `browser.rs` 注释、`Capture.jsx:255-257` |
| 8 | **窗口/时序易回归**：抓帧前藏三个窗口，`set_content_protected` 才能让底图不含选区层，权限首次要重启 | `commands.rs:511-521`、`overlay.rs:52-55` |
| 9 | **大图与文件生命周期**：识别限 8MB / 落盘 12MB，截图文件必须保留否则二次读取失败；无断点、无失败重试队列 | `jobs.rs:347`、`capture.rs:7-11` |
| 10 | **无采集任务视角**：没有「已采集列表 / 翻页游标 / 失败重试」，重复取题靠题干指纹事后判重 | `text.rs`、`repo.rs::fingerprints` |

### 9.2 重构目标（可量化）

> 取一题从「4 窗口 7 点击」降到「**1 个面板 2 次点击**」；连续取 10 题不用重新框选 10 次；来源可追溯、可筛选；没有视觉模型也能取纯文字题。

### 9.3 方案：统一「采集台」替换割裂的多窗口流程

**一个主窗内面板 + 一个可选系统浏览器悬浮按钮**，不再让用户在四个窗口间跳：

```
录入页「采集台」(单面板，四区)
┌─ 来源区 ────────────────────────────────┐
│ 常用站点(可增删改) │ 当前网址(自动填 source) │ 历史采集批次 │
├─ 取景区 ────────────────────────────────┤
│ 截图(⌘⇧O) │ 粘贴 │ 选文件 │ 内置浏览器打开 │ 系统浏览器协同 │
│ 智能裁剪 → 区域缩略图网格（点选/多选，框可拖拽微调、可回退整图） │
├─ 队列区（新） ──────────────────────────┤
│ 连拍列表：缩略图 + 来源URL + 页码 + 状态(待识别/识别中/完成/失败) │
│ 批量「开始识别」→ 一个 job 多图；失败项单独重试 │
└─ 结果区 ────────────────────────────────┘
   逐题核对卡片（复用 JobItemCard）+ 按来源筛选 + 一键入库 + 去重提示
```

具体改造：

| 项 | 做法 | 落点 |
|---|---|---|
| 来源记忆 | 新增 `capture_source` 表（url、站点名、favicon、上次时间、采集次数、星标）；浏览器打开/截图时自动记录，入库存 `question.source = 当前 URL`，题库筛选加「来源」维度 | `0003_teaching.sql`、`commands.rs`、`Library.jsx` |
| 常用站点 | 预置 3–5 个通用试题站域名（仅 URL 模板，不内置爬虫/选择器），用户可增删改；支持「复制题目链接粘贴」直接打开 | `Capture.jsx` + `app_setting` |
| 连续取题队列 | `capture_queue` 表（图路径、来源、状态、job_id）；每次截图/粘贴/连拍追加一项而不是替换；「批量识别」把多图打包给一个 `capture` job（`input_json.images[]`，`run_capture` 遍历合并 items 并带 `sourceUrl`/`imageIndex`）；入库后按批次标记完成 | `jobs.rs`、`Capture.jsx` |
| 智能裁剪升级 | 保留 Otsu 作快速通道；增加**视觉模型辅助切分**（一次调用返回 bbox，走已有 vision 模型；无模型时不可用并提示）；区域框支持拖拽改大小、删除、合并；点块**不替换整图**而是加入队列，可退回原图重选；深色/彩色背景先做灰度归一化 | `smart.rs`、`overlay.js`、`Capture.jsx` |
| OCR / 文本兜底 | 新增 `ocr` 用途 + 本地纯 OCR 路径（可选：macOS Vision 框架 `VNRecognizeTextRequest`，中文识别，纯本地不联网）；识别出的文字走**文本结构化 job**（`kind=capture_text`，提示词只做题干/选项/答案整理，不发图）；同时在采集台加「粘贴文字取题」入口，纯文字页直接复制粘贴 | `commands.rs`（Vision 通过 objc 桥）、`llm/prompts.rs`、`jobs.rs` |
| 系统浏览器协同 | 内置 WebView 被挡时，点「在系统浏览器打开」；悬浮小控制条常驻（只保留「截取本题」「回采集台」两个按钮 + 热键提示），截图经 `emit_capture_image` 回采集台；不再要求用户记住 ⌘⇧O，但保留热键 | `browserbar.js` 精简 + 新 `floatbar.html` |
| 切题校对 | 识别结果卡片显示原图对应框（bbox 高亮），点卡片跳原图位置；「切歪了」按钮对本块单独重识别，不整批重来 | `JobItemCard.jsx`、`commands.rs` |
| 大图与失败 | 上传前本地缩放长边 ≤2000（保留原图，缩放件仅用于识别）；失败项进队列可重试；OCR/识别任务断点记 `capture_queue.status` | `screen.rs`、`jobs.rs` |
| 回退开关 | 所有新行为（队列、来源记忆、OCR）可在设置里单独关闭，回到旧「一图一识别」流程 | `app_setting` |

### 9.4 与中心端取题的关系

中心的 `QuestionExtractionController`（PDF/整卷解析 + 页码图校对）是**服务端重解析**路径，本地客户端的取题是**视觉模型 OCR** 路径，两者不合并。阶段 0 之后可选加一条：本地队列批量上报到中心走整卷解析（质量更高但要联网 + 上传原卷），由用户在采集台选「本地识别 / 上传中心精解」，默认本地。

### 9.5 验收

1. 常用站点粘贴 URL → 打开 → 连截 5 题（含翻页）→ 队列显示 5 项 → 批量识别 → 一次核对入库 5 题，题库里每题 `source` 正确、按来源筛能筛出。
2. 一张双栏整卷图 → 智能裁剪找到 ≥2 栏区域 → 拖拽微调框 → 多选入队列 → 结果卡片能跳回原图框位置。
3. 无视觉模型时，纯文字截图（或粘贴文字）→ 本地 OCR / 文本结构化 → 正常入库；界面不报「没有视觉模型」死路。
4. 内置浏览器被登录页挡住 → 系统浏览器打开 → 悬浮按钮截取 → 回采集台；全程不用记热键也能完成。

## 10. 分期、依赖与验收总表

| 阶段 | 范围 | 前置 | 主要产出 | 预估人日 |
|---|---|---|---|---|
| **P0** | 取题快车道 §9（不依赖中心，可立即开工） | 无 | 采集台、来源记忆、连拍队列、裁剪升级、OCR 兜底 | 10–14 |
| **P1** | 同步底座 §4 | 无 | `/api/desktop/**`、设备令牌、通用 push/pull、资产上传 | 10–14（含服务端） |
| **P2** | 班级/学生 + 作业/提交/批改 §5 | P1 | 本地表 + 命令 + Classes/Assignments/Grading 页 + 中心绑定 | 12–16 |
| **P3** | 学情 §6 + 拍照批改 §7 | P2 | 统一公式、学生矩阵/趋势/错题、作答附件、拍照链路 | 12–16 |
| **P4** | 自制课件 §8 | P1（可并行，P2 后做布置） | 本地课件库 + 预览 + 轻量编辑器 + 中心动态清单 | 10–14 |
| **P5** | 两端页面补齐与联调 | P2–P4 | 中心 per-student 学情、课件清单动态化、三角色冒烟 | 6–9 |

合计约 **60–83 人日**，不含内容运营、签名公证、Windows 适配与模型费用。P0 与 P1 可并行；P2 是核心闭环，P3/P4 可在 P2 验收后再排。

跨阶段验收口径：

1. **离线可用**：P2–P4 的所有本地功能断网可跑；联网只是补推与取学生数据。
2. **幂等**：同一批次重复推送不产生重复班级/学生/作答/课件；冲突不静默丢数据。
3. **权限**：中心端教师只能管自己班、学生只能看自己；`/api/desktop/**` 无令牌一律 401；本地档案生不产生中心账号。
4. **数据一致性**：学情公式两端同结果；软删除在两端一致；图片 sha256 校验通过才展示。
5. **可回滚**：每个阶段的迁移单独编号，回滚语句写在文件尾部；升级失败保留旧库备份。

## 11. 风险与合规

| 风险 | 影响 | 应对 |
|---|---|---|
| 未成年人数据 | 本地存学生姓名/照片，中心同步涉及隐私 | 本地照片只在老师设备；上报中心默认匿名统计；绑定账号需老师显式操作；文档写明数据边界；不收集使用日志（沿用 D5） |
| 拍照批改主观性 | AI 自动判手写答案易错 | AI 只做「切块 + 抄录建议」，分数与评语必须老师确认；原图永久保留 |
| 学情口径变更 | 中心历史 mastery 数值会变 | 作为一次行为变更公告；保留 `study_progress` 旧值快照一周供对比 |
| 两端同步冲突 | 老师改了班名、学生同时在中心提交 | 不自动合并：保留服务端版本 + 本地冲突副本 + 页面提示选择；沿用现有冲突策略 |
| 课件版权 / 外链 | 导入的 HTML 可能带外部脚本 | 导入时静态扫描外链 `<script src>` / `fetch(http)` / 字体 CDN，拒绝或提示；iframe 用隔离窗口，不进主窗 CSP |
| 取题合规 | 试题网站 ToS / 版权 | 只做用户主动浏览/主动截图；不做无头抓取、不绕登录付费墙；来源默认私有（沿用 §10.3） |
| 设备令牌安全 | 令牌泄露导致跨设备读数据 | HTTPS + 只存哈希 + 令牌按设备隔离 + 管理台可吊销；`/api/desktop/**` 之外的 CSRF 行为不变 |
| 工作量过大 | 五条线一起铺导致都做不完 | 按 §10 分期，P0/P1 先出可验收产物；P2 验收后再排 P3/P4；本文档不承诺同时交付 |

## 12. 决策记录

| # | 决策 | 结论 |
|---|---|---|
| D0 | 旧「不做班级/作业/批改/学情」边界 | 局部推翻；保留本地单用户、学生不登录本地、中心仍是协作端 |
| D1 | 学生身份 | 本地 `local_student` 档案为主；可选绑定中心 `app_user(STUDENT)`；默认不为未成年人批量建账号 |
| D2 | 学情公式 | 两端统一到衰减加权公式，写共享规格 + 同组测试向量；中心 `classMastery` 改为读 `study_progress` |
| D3 | 自制课件 | 先做课件库（收纳/预览/布置），后做表单化 spec 编辑器；不做 WYSIWYG 3D 编辑器；编译器不进安装包本体 |
| D4 | 拍照批改 | 在现有 `attempt/submission` 上挂图片附件，不建平行批改系统；AI 只切块/抄录，分数由老师定 |
| D5 | 取题 | 统一采集台 + 连拍队列 + 来源记忆 + OCR 兜底；中心整卷解析作为可选「精解」，默认本地 |
| D6 | 两端通道 | `/api/desktop/**` 从零建，独立于 `/api/edu/**`；通用实体 outbox，本地是 source of truth |
| D7 | 优先级 | 取题快车道（P0）与同步底座（P1）先做；班级闭环（P2）是核心验收点；学情/课件随后 |

## 13. 仍待定（不阻塞开工，实施前定）

1. 相机入口选 `AVFoundation` 还是系统扫描仪 / 连续拍照是否支持（阶段 2 A′ 前定）；手机扫码上传的 HTTP 栈、二维码生成、令牌策略见 [手机拍照联动规划](local-phone-photo-submission-plan.md) §7。
2. macOS Vision OCR 的中文准确率是否够用，不够是否允许用户配第三方 OCR 端点（不内置权重）。
3. 本地课件编辑器的编译器形态：独立 CLI 还是 Rust 移植最小子集（阶段 3 前定，先做 8.1a 不受影响）。
4. 中心 `coursewareList.js` 动态化后，旧静态清单是保留为 fallback 还是移除。
5. 设备令牌有效期/续期策略是否沿用规划 §11.2 的 180 天。
6. 旧 `sync.rs` 的 outbox 数据迁移：是否重放、如何避免重复推送（阶段 0 前定）。

## 14. 回滚说明

本文档本身无代码/迁移改动。实施时：

- 服务端所有新表放新迁移文件，尾部写 `drop table ...` 逆序回滚；不动 V1–V97。
- 本地迁移按阶段独立成文件，`user_version` 管理（P0 已用 `0003_capture.sql`；后续阶段顺延）；升级失败保留旧库备份，不自动降级。
- 任一阶段回滚只下线该阶段新增的页面/命令，旧题库数据不受影响；同步关闭只退出设备登录，本地数据保留。

## 15. 实施记录

### 15.1 P0 取题快车道（已完成，2026-11-16）

对应 §9.3 的「统一采集台」，代码落在 `local-client`，未动中心系统。

| 项 | 落地 |
|---|---|
| 数据表 | `migrations/0003_capture.sql`：`capture_source`（网址唯一键、`starred`、`capture_count`、`last_at`）、`capture_queue`（`image_path`、`source_id/url`、`status`、`job_id`、`error`、`item_count`） |
| 仓储 | `capture_store.rs`：`remember / sources / star / forget / enqueue / get / list / mark / set_item_count / retry / remove / clear_done` |
| 任务 | `jobs.rs` 拆出 `capture_one_image` 共用：`capture`（单图）/ `capture_batch`（`images:[{path,sourceUrl,queueId}]` 逐张、单张失败不拖累整批）/ `capture_text`（纯文字，走 `chat` 用途，不要求视觉模型） |
| 命令 | `capture_sources`、`capture_source_remember/star/forget`、`capture_queue_list/add/retry/remove/clear_done`、`question_sources` |
| 前端 | `pages/Capture.jsx` 重写为四区采集台；来源卡片、连拍/精细双模式、区域多选批量入队、粘贴文字取题、队列状态与重试；`pages/Library.jsx` 加来源筛选 |
| 来源准确性 | `browser_status` 回传内置浏览器的**当前真实地址**（`window.url()`），站内翻页后取的题也记对来源 |
| 测试 | `flow_tests.rs` 新增 3 个：来源累加/置顶/忘记、队列状态机（含失败重试与清理）、按来源筛选 + 来源统计；迁移版本断言更新为 3。`cargo test` 49 项全过 |

与 §9.3 的差异：

- **OCR 兜底改为「粘贴文字取题」先行**。macOS Vision 中文准确率仍待实测（§13 待定 2），先给一条零依赖、零额度的文字通道；OCR 作为后续增强，不阻塞。
- **没有引入整队共享原图**：`capture_one_image` 给每题写 `capturePath`，`commit` 按题挂各自原图，避免批量场景整队共用第一张。
- **队列不落 crop 记录**：区域裁切产生的临时图直接入队，`capture_queue.image_path` 就是那个临时文件的路径；入库后原图作为 `CAPTURE` 附件归一化，队列只是中转。

仍未做（按 §10 顺延到 P1 及以后）：`/api/desktop/**`、班级/学生/学情、拍照批改、自制课件。

### 15.2 本地文件 / PDF 取题（已完成，2026-11-16）

对应 §9.1 痛点 1 与「拿图方式只有截图/粘贴」的缺口：老师手里的整卷多是下载来的 PDF，
既不在浏览器里，也不在屏幕上，前两条路都拿不到。

| 项 | 落地 |
|---|---|
| 模块 | `src-tauri/src/filein.rs`：扩展名 + 文件头双判类型；图片直读，读不出（HEIC/TIFF/BMP）走系统 `sips` 转 PNG |
| PDF 栅格化 | macOS `objc2-core-graphics`：`CGPDFDocument::with_url` → `CGPDFPage` → `CGBitmapContextCreate`（自备 RGBA 缓冲、白底填充）→ `draw_pdf_page`；长边 1800，按需渲当前页到 `data_dir/tmp`，渲完不删（识别与入库各要再读一次，同 `capture.rs` 约束） |
| 命令 | `file_open(path, page)` → `LocalFile { path, name, kind: image\|pdf, page, pages, image: CaptureResult }`；前端 `fileOpen()` |
| 前端 | 取景卡片「打开本地文件」（图片 + PDF，含「全部文件」兜底）；PDF 自动切精细模式并出现 `‹ N / M ›` 翻页；「框选取题」在预览上拖框，「放大预览」浮层里做精确框选后再裁；「裁下选区入队」复用 `smart_crop`（越界自动夹紧） |
| 拖放 | `getCurrentWebview().onDragDropEvent`：从 Finder 直接拖入，有 PDF 就打开它（PDF 不能当图片排队），纯图片按连拍/精细模式处理 |
| 测试 | `filein` 2 项（文件名/扩展名判定、缺文件报错可读）；`smart` 2 项（手画选区按原尺寸裁、越界夹紧）。`cargo test` 53 项全过 |

与 §9.3 的差异与取舍：

- **不打包 pdfium / mupdf**：那类依赖动辄几十 MB，且要额外分发二进制；CoreGraphics 是系统框架，零分发成本，只牺牲 Windows 支持（Windows 走「另存为图片」或后续接 Direct2D/PDFium）。
- **不把 PDF 当整体图**：整卷栅格化会撞 8MB 识别上限，也看不清；按页渲 + 页内裁剪才是可持续的用法。
- **卡片内预览故意做小**（`max-height: 400px`，A4 约 280×400）：796 高的窗口里一屏能看完整个取景卡片；要看细节走「放大预览」浮层（约 456×644），框选在那里做。
- **PDF 强制精细模式**：连拍对「按页裁」没有意义，自动切模式比让用户自己发现更省事。


### 15.3 题库列表：翻页、标签置顶与工具条改版（已完成，2026-11-16）

§9.1 痛点 2「题库不好用」的界面部分：原先把常用标签放在列表**底部**（要滚到底才看得到）、标签一多就只显示前 80 个且没法搜、分页只有一个「上一页/下一页」且在最底部，翻页后还停在原滚动位置。

| 项 | 落地 |
|---|---|
| 工具条 | `pages/Library.jsx` 顶部改成一张 `lib-bar` 卡片：第一行搜索 + 学科 / 题型 / 难度 / 来源 / 排序下拉 + 只看待复核；第二行**常用标签**（模糊搜索框、命中高亮、默认 14 个、「展开全部 N 个」）；第三行「已筛选」胶囊，逐个 ✕ 或「清空全部」 |
| 标签筛选 | 点标签走 `QuestionQuery.tag` **精确**过滤（原先是把标签名塞进搜索框做全文匹配），再点取消；切换回收站时清空所有条件 |
| 分页组件 | 新增 `components/Pager.jsx`（可复用）：范围文字 + 首尾/上下页 + 页码（页多时首尾 + 当前页前后各一）+ 每页 20/50/100 + 跳页输入 |
| 交互 | 列表头 `list-head` 吸顶（毛玻璃背景），显示题量、当前区间、第 x/y 页，并带一对紧凑 `‹ ›`；翻页 / 改每页后滚回列表顶部；当前页删空或筛选后页码越界自动退到最后一页；新增「全选本页」 |
| 后端 | `repo::tags` 去掉 `limit 80`，按题量降序全量返回（标签云要能搜、能展开）；`sort` 下拉复用已有 `QuestionQuery.sort` |
| 测试 | `cargo test` 53 项全过（本轮无 Rust 逻辑新增）；`npm run build` 通过；DMG 5.1M 校验 VALID，已安装 |

取舍：标签搜索放在**前端**做（一次 `tag_list` 拿全量，本地 `includes` 过滤），数据量是「一台电脑上的题目标签」，没必要为每个按键往返 SQLite；等标签上千再考虑后端 `like` 分页。

### 15.4 阶段 0 同步底座（P1，已完成，2026-11-16）

客户端侧 `sync.rs` 早于服务端写好，一直只能报「服务器没有 /api/desktop/** 接口」。本轮把服务端补齐，
契约以**客户端已发布的字段名**为准（`questions[]` / `results[]` / `items[]` / `nextCursor`），不另起一套。

| 项 | 落地 |
|---|---|
| 迁移 | `V98__desktop_sync.sql`：`desktop_device`（令牌哈希、按设备唯一、180 天、可吊销）、`desktop_question_link`（设备 + 本地 clientId → 中心 question.id）、`desktop_sync_batch`（batchId → 上次响应，重放用） |
| 令牌 | `DesktopTokenService`：32 字节随机数 Base64URL（43 字符），库里只存 SHA-256；同一台设备重复登录只换令牌不新增设备行；角色限 ADMIN / EDITOR / TEACHER，学生与只读账号 403 |
| 鉴权 | `DesktopAuthFilter` 只拦 `/api/desktop/**`：带 Bearer 必须有效（无效 401，不退回匿名），无令牌交给后面的规则；`SecurityConfig` 只加两条 matcher + CSRF 豁免，Web 端会话与 CSRF 行为不变 |
| 接口 | `POST /api/desktop/auth/login|revoke`、`POST /api/desktop/sync/push`、`GET /api/desktop/sync/pull`、`GET /api/desktop/sync/status` |
| push | 按 `(设备, clientId)` 幂等：新建 CREATED / 更新 UPDATED / 撞他人题 CONFLICT / 单题异常 FAILED（不拖垮整批）；`batchId` 重放直接回放上次响应；选项 `{key,text}` ↔ `{label,text}` 互转；标签与考察点按快照整体替换；题目落 `question` 表、`origin=DESKTOP`、状态 DRAFT、归属令牌用户 |
| pull | `(updated_at, id)` 游标（ISO:uuid，兼容毫秒前缀），只下发自己的题，软删除以 `deleted=true` 下发；`limit` 上限 500 |
| 客户端 | `sync.rs` 的拉取不再「只更新已知题」：本地没有的题按中心 id 直接落库（`origin=CENTER`，学科按名字对齐、对不上落第一个），标签整体替换；本地有未推送改动（outbox 待推）时跳过覆盖 |
| 文档化差异 | 知识点路径（教材/章/知识点）本轮**不解析**（两端编码未对齐），push 响应里用 `skippedKnowledge` 报个数；图片资产上传（`/api/desktop/assets`）未做，等作答照片落地再一起做 |

验收（本机真实跑通，2026-11-16）：

1. `demo_creator` 登录 → 拿到 43 字符令牌；同设备再登录不新增 `desktop_device` 行。
2. push 一道题 → CREATED + serverId；同一 batchId 重发 → 原样回放，`question` 表仍 1 行。
3. 换 batchId 改题干再推 → UPDATED，仍是 1 行，`difficulty` 更新、标签不多不少。
4. push `deleted=true` → 中心 `deleted_at` 落值且 `updated_at` 前移，下一次 pull 带 `deleted=true` 下发。
5. pull 无游标 → 全量；`limit=2` 两页不重叠不遗漏；带游标再拉 → 0 条。
6. 无令牌 / 错令牌 → 401；`/actuator/health` 仍 200，Web 端接口不受影响。
7. `cargo test` 54 项全过（新增 `desktop_pull_imports_then_updates_without_duplicates`）；
   服务端新增 `DesktopTokenServiceTest` 5 项全过。

仍未做（P1 尾巴，随 P2 一起）：`/api/desktop/assets` 资产上传、通用实体 outbox（student / class /
assignment / attempt / progress）、`sync_status_v2`、`pages/Sync.jsx` 的分类待同步计数。

### 15.5 阶段 1：班级 / 学生 / 作业 / 批改（P2 本地闭环，已完成，2026-11-16）

按 §5 落地本地优先闭环：不连中心也能建班、导名单、组卷、提交、判分、批改。

| 项 | 落地 |
|---|---|
| 迁移 | `0004_teaching.sql`：`local_student`、`local_class`（邀请码只对未删除班级唯一）、`local_class_member`、`local_assignment`、`local_assignment_question`、`local_attempt`；全部软删除 + 索引，回滚语句在文件尾部 |
| 学生 | `teaching.rs::student_*`：新建 / 编辑 / 归档 / 删除；**手工新建允许同名**（一个班真可能两个张伟），只有批量导入按「同名同年级」去重 |
| 名单导入 | `姓名, 年级, 备注` 三列，逗号 / 制表符 / 分号 / 顿号都认；`student_import_preview` 先给逐行预览并标出重复，`student_import_commit` 单事务写入、跳过重复、逐行报错 |
| 班级 | 6 位邀请码（字符集与中心一致，去掉 I/O/0/1），撞码重试 8 次；成员增减、结课 / 开课、删除、重置邀请码；`class_list` 带人数 / 作业数 / 待批数 |
| 作业 | 从本地题库搜题组卷（`question_list`），逐题设分值，整体替换式保存；草稿 → 发布（至少 1 题）→ 关闭 / 重开；`assignment_detail` 返回题目全文与按学生聚合的成绩单（未交也列出来） |
| 作答 | `attempt_submit` 一次提交一个 `submission_id`；重交把旧记录标 `superseded_at`，成绩单只算最新一次；`late` 按 `due_at` 判定 |
| 自动判分 | `grade_answer`：单选比字母、多选比集合（大小写与顿号都归一）；填空 / 解答返回 `None` 留待人工；空答案记 0 分 |
| 批改 | `attempt_grade` / `attempt_batch_grade`：0 ≤ 分数 ≤ 满分，未显式给对错时按 ≥60% 推导（与中心一致），写 `graded_by`（本地老师名）与 `graded_at`，评语可空、空则保留原值；`attempt_clear` 撤销提交供重做 |
| 前端 | `pages/Classes.jsx`（班级 / 学生两 Tab、班级卡、成员抽屉、名单导入、添加成员）；`pages/Assignments.jsx`（作业卡、组卷弹窗 + 题库选题器、成绩单抽屉、**纸质作答录入**、逐题批改抽屉）；侧栏加「班级」，`#/assignments/:classId` 复用班级高亮；帮助中心加 `classes.md` |

验收（本机跑通）：

1. 导入 3 行名单（含 1 行重复）→ 预览 `total=3 / duplicates=1`，提交后新增 2、跳过 1；表头行不计入。
2. 建班 → 邀请码 6 位且字符集正确；重复加同一批成员返回 0，人数不翻倍。
3. 软删除学生：学生列表与班级成员都看不到，`local_student` 行仍在（可追溯）。
4. 组 1 道单选（2 分）+ 1 道解答（3 分）：草稿状态提交被拒；发布后提交，单选小写 `b` 判对、自动得 2 分，解答题记待批；`attempt_sheet` 两题顺序与判定正确。
5. 解答题给 9 分（满分 3）被拒；给 2.5 分后成绩单待批归零、得分 4.5/5、`gradedBy` 落本地老师名。
6. 未交的学生在成绩单里能看到「录入作答」：选择题点选项、主观题写文本，保存后跟在线提交走同一套判分；撤销提交后作答行归零，可重做；未交的那位仍留在成绩单（`submittedAt=null`）。
7. `cargo test` **60 项全过**（新增 4 项：名单导入 + 成员 / 作业提交判分 + 批改 / 判分归一化）；`npm run build` 通过。
8. UI 用真实组件 + 假数据跑了 7 个状态（班级、学生、成员抽屉、新建班级、作业列表、成绩单、批改、组卷弹窗、添加成员），修正了全局 `input` 样式把复选框撑成大蓝块的问题（`input[type=checkbox]` 现在单独归零宽高与内边距）。

本轮没做（随 P3）：学情矩阵 / 趋势 / 错题本、作答照片附件、教学实体 outbox 推送、学生绑定中心账号（`center_user_id` 列已留）、vCard 通讯录导入（先支持粘贴表格）、`late_hours` 只用于标记迟交、未做「宽限期内仍可交」的强制校验。

### 15.6 本机存储面板与清理（P2 补丁，已完成，2026-11-17）

老师问得最多的两件事：**「SQLite 的账号密码是什么」**、**「截图用久了占几个 G，能删吗」**。
前者是误解（本地库没有账号密码），后者是真实痛点——`tmp/` 里的截图与 PDF 页渲染只增不减，
而 `assets/` 里的题目图一张都不能丢，两者混在一个文件夹里，用户不敢动手。

所以这一步不做「一键清空」，而是**把占用拆开给用户看，只对过程文件开放删除**。

| 目录 | 是什么 | 能不能清 | 判定方式 |
|---|---|---|---|
| `neuralkb.db` + `-wal` / `-shm` | 题目、标签、班级、作业、账号 | 否 | 主文件与 WAL 影子文件一起算 |
| `assets/<题目 id>/` | 题干与解析引用的图（`nbasset:`） | **只有无主图片能清** | 文件不在 `question_asset.relative_path` 里，且落盘超过 10 分钟 |
| `tmp/` | 截图、PDF 页渲染、HEIC 转换、跨页拼接、导入解压 | 是 | 落盘超过 10 分钟；正在识别/预览的图留着 |
| `backup/` | 「导出整库备份」生成的 ZIP | 是（保留最近 3 份） | 按 mtime 排序 |
| 其余根目录文件 | `.DS_Store`、检索索引等 | 否 | 归入「其他」 |

实现要点：

- `storage.rs`：`scan` 递归统计（不跟符号链接，避免把外接盘链进来）；`db_bucket` 三个文件一起算；
  `usage` 返回分类字节数 / 文件数、题目图覆盖题数、受保护的临时文件数、无主图片数与字节数、
  磁盘总量与剩余（macOS `statvfs`，其他平台先返回 `None`，不引入新依赖）。
- 清理三档：`clean_tmp` / `clean_orphan_assets` / `clean_backups`，统一返回
  `{removed, freedBytes, kept, pinned, errors}`；单项失败只计数不中断，`kept` 用来告诉用户
  「有几项太新被留下」，`pinned` 是连拍队列里还等着识别、删了就得重拍的图。
- 保护窗口 `GUARD = 10 分钟`：识别任务与 PDF 预览是「先落盘、再按路径读两次」的链路，删早了就是
  「读不了截图」。窗口取 10 分钟，覆盖一次识别 + 人工核对的时间。
- 队列保护：`capture_queue.image_path` 直接指向 `tmp/` 里的文件，`status in ('PENDING','RUNNING','FAILED')`
  的项在 `clean_tmp` 里整条跳过（`pinned`）；`DONE` 的项不必管——入题时已复制到 `assets/`。
- 边界：只 `read_dir` 三个子目录并删其中条目，绝不跨目录 `remove_dir_all` 用户数据；孤儿清理后
  顺手收掉 `assets/` 下的空题目目录。
- 前端：设置页新增「本机存储」卡片——总占用 + 磁盘剩余进度 + 五段占比条 + 逐行明细
  （题目图 / 临时文件 / 数据库 / 备份 / 其他），每行给结论式文案与唯一动作按钮；
  确认框写明「题目里的图不受影响」，清理完就地刷新。

验收：

1. `storage::tests` 6 项：保护窗口（窗口内 0 删、窗口 0 才删且字节数正确）、清 tmp 不动 assets、
   队列里的图即使无保护窗口也保留、孤儿清理保留被引用图并收空目录、备份留最近 3 份、
   `usage` 分类金额与 `total` 对得上。
2. 真实数据目录：`tmp 5.9M / assets 7.6M / db 4.7M`，面板显示的分类之和等于 `du -sh`。
3. 组件预览（真实 Settings + 假数据）：占比条、五行明细、按钮与数字对齐，无溢出。

顺带把「SQLite 没有账号密码」写进设置页与帮助：`neuralkb.db` 就是一个文件，能打开数据目录就能读；
防的是别人拿到这台电脑（系统账户 + 磁盘加密），不是数据库连接。应用登录密码是另一回事——
argon2id 哈希存在 `local_user` 表里，与数据库本身是否加密无关。

### 15.7 取题体验三修：结果按页缓存 / 选区级跨页 / 题块点选（已完成，2026-11-17）

老师连着提了三件事，都是「取题快车道」用起来以后暴露的真问题：

1. PDF 多页一起处理，认完一张切到下一张，上一张的解析没了，只能重认；
2. 跨页题没真正解决——整两页竖拼，模型常只认一半；
3. 题块框看不清、不好点，「原来不是说点一块就自动框出那道题吗」。

#### 结果按页缓存

缓存键是 `文件路径#页码`，不是渲染出来的临时图路径——PDF 每次翻页都会重新渲一张新
tmp 文件，按图路径缓存等于按一次性的 uuid 缓存，翻回去必然失配。

- `shotCache`（内存 Map）+ `pathKeys`（临时图 → 页键）两张表：打开页时登记映射，识别完成后
  按每道题的 `capturePath` 反查页键，把结果分组写回各自页；
- 切页时只换「右栏显示的那一份」，缓存里的其它页不动；勾选与就地编辑也写回当前页；
- 缓存里**不存 jobId**：恢复旧 jobId 会让轮询再跑一遍，用模型原稿覆盖用户在预览里改过的内容；
- 识别进行中切页不掐任务（`jobRef` 判 RUNNING），结果回来依旧能落到对应页；
- 批量识别若来自连拍队列里的裁块（没有页键映射），结果整体摆出来并提示来源，总比「认完看不到」强。

#### 跨页改成「两段选区拼接」

老做法是 `image_stitch([整页A, 整页B])`：一张超高的图里塞着页眉页脚、选项表和上半页已认过的题，
模型注意力被摊薄，只认一半是常态。

新流程（`stitchNext` → `finishSpan`）：

1. 在上一页框出这道题真正需要的那段（没框就用整页）；
2. 点「跨页拼接」：先把这段裁下来（`smart_crop`），再打开下一页，提示条常驻；
3. 在下一页框出接着的部分（没框就整页）；
4. 「接上并识别」：两段 `image_stitch` 竖拼（中间留白缝）→ 直接 `startJob('capture')`；
5. 还长就「再拼第 N 页」，每一页只取那一段；拼接结果的缓存键是 `文件#起始页-结束页`；
6. 已拼过的图再拼时忽略残留手选框，避免把前两页裁掉一截。

#### 题块：自动找、点一下选中、直接识别

- 打开图片 / 翻 PDF 页后**自动**跑一次 `smart_regions`（`detectRegions`，带 token 丢弃过期结果），
  不用先点按钮；
- 默认不选中任何块（原来默认全选，整页一片高亮，反而看不清）；
- 点一块 = 单选高亮，再点取消；`⌘ / Ctrl / Shift` 点 = 追加多选，用于批量入队；
- 「识别这道」= `smart_crop` 裁下 → 直接识别，不必先进队列再批量；
- 放大浮层里同样有题块层，点中即选中并退回主视图，照顾「看不清楚」的场景；
- 样式：删除旧的 `.region-box.picked { 金色 }` 与 `.region-box:not(.picked){opacity:.45}` 两条覆盖规则
  （「展示不清晰」的根因就是它们），改为未选中 1.5px 淡蓝虚线 + 5% 填充、选中 2px 实线 + 14% 填充 +
  实心编号，一页里只亮一块。

#### Navicat 打不开数据库

本地库是 WAL 模式，`neuralkb.db` 的最新写入还在 `-wal` 里；第三方工具直接开主文件，轻则读到旧数据，
重则因为拿不到 `-shm` 锁直接报错。设置 → 数据与备份新增「导出数据库副本」：
`VACUUM INTO` 生成一个自包含、已整理的普通 `.db`，Navicat / DB Browser 单文件就能打开；
副本是快照，怎么改都不影响题库。

验收（2026-11-17）：

1. `cargo test` **67 项全过**（新增 `database_snapshot_opens_standalone_with_data_and_version`：
   快照可独立打开、题目在、`user_version=4`、不带 `-wal`）。
2. 真实组件预览三种状态：默认自动出框、点第 3 块实线高亮、跨页第二步提示条；
   截图存 `local-client/ui-preview-pick.png` / `ui-preview-span.png`，无重叠与溢出。
3. 打包 5.2M VALID、签名校验通过，已覆盖安装。

### 15.8 P3 前半：学情矩阵 / 趋势 / 错题本（已完成，2026-11-17）

§6 的规划落到本地，公式与中心 `ClassService.refreshMastery` **逐行对齐**（同一份 `weight=1/(1+days/14)`、
`factor = correct ? 0.75+difficulty*0.05 : 0.05`、`min(100, Σwf/Σw*100)`），差别只有存储方式：

| 项 | 中心 | 本地 |
|---|---|---|
| 存储 | `study_progress` 表，每次批改写库 | **不落表**，按需从 `local_attempt` 重算 |
| 一致性 | 写库那刻的值 | 作答记录是完整输入，重算即等价，且不会「改了分忘刷新」 |
| 趋势 | 只有当前值 | 按周回放：把「到该周为止」的作答重算一遍，得到 8 周曲线 |

**为什么趋势不落快照表**：`local_mastery_snapshot` 的用处是「记住当时的数」，但掌握度本身是
`(作答集合, now)` 的纯函数——同一份作答在任何时刻都能复算出同一条曲线，落表只会带来「快照与重算不一致」
的第二种真相。省掉一张表，也省掉「批改后忘写快照」的坑。

#### 后端

- `migrations/0005_insights.sql`：`local_wrong_book`（`(student, question)` 唯一、状态 NEW/REVIEWING/MASTERED、
  `wrong_count`/`review_count`/`next_review_at`）、`local_attempt_asset`（整页照片与题级照片共用一张表，
  为 §7 拍照批改备好）。
- `insights.rs`：`mastery_of`（公式本体）、`class_insight`（知识点列 + 学生行矩阵 + 共性错题 Top5 +
  待批改数 + 得分率）、`student_insight`（知识点列表 + 8 周趋势 + 最近作业 + 待复习数）、
  `wrong_list` / `record_wrong` / `resolve_wrong` / `wrong_review` / `wrong_remove` / `due_review`。
- 只算 `correct is not null` 的作答：主观题待批时既不算对也不算错，单独计数展示；`superseded_at` 的重交不参与。
- 错题本生命周期：判错入本（同日重复只累加次数）→「复习过」按 3/7 天递进 → 做对自动 `MASTERED` 停催 →
  可手动移出（软删除，作答与成绩不受影响）。
- 顺手修掉一个真 bug：`repo::validate` 以前对**草稿**也要求答案非空，导致识别缺答案的题存不进草稿箱
  （`flow_tests::missing_answer_does_not_block_draft_but_blank_stem_does` 一直在挡）。现在只有正式题目强制答案。

#### 前端

`pages/Insights.jsx`（新，侧栏「学情」在「班级」之后）：

- 顶部班级胶囊 + 五个 KPI（班级掌握度 / 得分率 / 已掌握 / 薄弱 / 待批改，待批改带「去批改」直达作业）；
- **知识点矩阵**：行是学生（薄弱的排前），列是作业覆盖的知识点，格子按掌握度分五档上色，
  `–` 表示没作答（不是 0 分）；姓名列 sticky，表头 sticky；点姓名开抽屉；
- **共性错题**：错误率排序 Top5，带知识点与「做错 N / 共 M 人」；
- 学生抽屉：8 周折线（纯 SVG，含 85 分参考线）、知识点进度条、错题本（状态筛选 + 复习/掌握/移出）、最近作业。

#### 验收（2026-11-17）

1. `insights::tests` 4 项：5 组公式向量（全对 90、全错 5、满分封顶 100、一对一错 48、衰减 37）、
   近因加权、空作答、趋势长度与累计作答数。
2. `flow_tests::insights_matrix_and_wrong_book_follow_grading`：建班 → 布置 3 题（2 知识点）→
   交卷（一对一错一待批）→ 错题本 1 条 → 班级矩阵 2 列 2 行、掌握度 43、待批 1 且不计入 →
   批主观题后重算 → 复习一次进 REVIEWING → 重交做对后 MASTERED 且待复习归零 → 移出后清空。
3. `cargo test` **73 项全过**；真实组件预览两张截图（`ui-preview-insights.png` / `ui-preview-student.png`）。

### 15.9 P4：教师课件（讲义式）库 + 生成器（已完成，2026-11-17）

对应 §8（决策 D3′）。**用户澄清**：本地题库的「课件」是**讲义**——知识点讲授 + 例题精讲 +
变式拓展 + 拔高 + 思维训练，与题库题、模型补题打通；不是 `courseware-lab` 那种 3D 仿真课件
（后者用「导入 HTML」进库统一管理）。§8 已按此重写。

| 项 | 落地 |
|---|---|
| 数据表 | `migrations/0006_courseware.sql`：`local_courseware`（`lesson_spec_json` 存讲义骨架、`source` IMPORT/LOCAL_BUILD、`step_count`=段落数、`quiz_count`=题数）、`local_class_courseware`（布置到班级）、`local_courseware_question`（例题/变式/拔高挂题库题，学情可回溯） |
| 模块 | `src/courseware/mod.rs`：导入（体积 ≤16MB、外链 `<script src>` 直拒、其他外链提示、内容哈希去重）、库查询/改名/软删、布置/取消布置、`HandoutSpec` 五道闸（结构与必填 → 题目引用与题干 → 篇幅与体量 → 打印可用 → 产物自检）、编译落库（题库题的题干/答案/解析内联进讲义 + 登记挂接） |
| 渲染 | `src/courseware/render.rs`：单文件 HTML 讲义（知识清单 / 例题精讲 / 变式拓展 / 拔高挑战 / 思维训练 / 小结作业六段，段号按实际显示重排），答案解析折叠（打印时自动展开）、`@media print` A4 分页不断题、三套版式（米纸/科学蓝/黑板绿） |
| 预览 | 自定义协议 `nbasset://localhost/courseware/<id>`（`AppState.courseware_files` 登记表），只回已登记文件，不开放整个数据目录；iframe 隔离加载 |
| AI 补题 | `courseware_ai_draft`（`llm/prompts.rs::handout_draft`，走「AI 出题」用途）：拿例题当锚点补变式/拔高/思维训练**草稿**，老师过目改完再编译 |
| 命令 | `courseware_list/import/update/delete/preview/build/spec/assign/unassign/ai_draft` |
| 前端 | `pages/Courseware.jsx`：课件卡片（封面渐变、知识点、段/题/布置数）、搜索/学科/来源筛选、隔离预览浮层、布置班级（含截止时间）、元信息编辑、**讲义生成器**（基本信息/知识清单/例题/变式/拔高/思维训练/小结作业 + 从题库挑题 + AI 补题 + 五道闸结果）、`help/courseware.md` |
| 清理保护 | `storage.rs` 的无主图片清理跳过 `assets/courseware/`（课件由课件库管生命周期，不当孤儿图删） |

#### 验收（2026-11-17）

1. `courseware::tests` 7 项：闸拦空讲/缺解析/段落过多、编译内联题库题干解析并登记挂接、
   重建升版本、无效 spec 不写盘、导入拒外链脚本且哈希去重、布置/取消布置进列表、改元信息。
2. `cargo test` **88 项全过**；讲义产物实测 16KB、零外链、含 `@media print`，headless Chrome
   截图版式正常（`nb-handout-preview.html`）；课件页卡片/筛选/布置实测截图正常。
3. 断网可用：编译产物双击即开、打印导出 PDF 不依赖网络。

仍未做（顺延）：中心 `desktop_courseware` + 动态清单、讲义同步推送（教学实体 outbox）、
拍照批改（P3 后半）、讲义模板库（按课型预置骨架）。

### 15.10 P3 后半：拍照批改（已完成，2026-11-17）

对应 §7（决策 D4）。**不另起批改系统**：照片是挂在既有 `local_attempt`（一次提交 = 一个
submission_id）上的附件，批改、学情、错题本全走原来那条链路。

| 项 | 落地 |
|---|---|
| 数据表 | `migrations/0007_grading_photos.sql`：给 `local_attempt_asset` 补 `page_asset_id`（题块来自哪张整页）、`bbox_json`（在整页里的位置）；新增 `local_attempt_asset_note`。迁移版本 **7**（flow_tests 两处断言同步改） |
| 模块 | `src/grading.rs`：整页照片落盘（`assets/submissions/<学生 id>/`）、按区域切题块（记来源与 bbox）、挂题/改挂/软删、读图给前端、**纸质代提交**（学生没在线交卷时建一次空提交，全题待批）、AI 抄录建议 |
| 切题 | 复用 `smart.rs` 的区域检测（行投影 + 缩进并回），候选框以带编号的蓝框叠在整页图上；老师可点框，也可**直接在图上拖框自选**（手写卷子自动检测常不准） |
| AI | `llm/prompts.rs::grading_transcribe`（新用途 `grading`「批改抄录」，`llm::candidates` 自动只挑 `supports_vision` 的模型）；输出 `{answer, score, remark, confidence, warning}`，**只返回建议不写库**，老师点「用这个建议」才生效。抄录逐字，看不清写「□」，禁止改写补全 |
| 命令 | `grading_photo_add/list/by_attempt/regions/crop/attach/remove/data`、`grading_paper_submit`、`grading_transcribe` |
| 前端 | `pages/Grading.jsx`（新「批改台」页）：待批队列（待批优先排序、`待批 N/已批/未交` 徽标、📷 照片数）、上一份/下一份、照片面板（整页 / 题块分组、未挂题块可下拉挂题）、切题对话框（可视化框选）、逐题卡片（题干 + 参考答案 + 该题照片 + 得分评语 + AI 建议卡）、底部合计与保存批改；`teaching.rs` 的 `AttemptItem` 补 `photos`、`SubmissionRow` 补 `photo_count` |
| 清理保护 | `storage.rs`：`referenced()` 增读 `local_attempt_asset`（否则作答照片会被当孤儿图删），清理时跳过 `submissions/` 前缀 |
| 入口 | 侧栏新增「批改台」；作业页页头加「拍照批改」；`help/grading.md` |

#### 验收（2026-11-17）

1. `grading::tests` 7 项：照片落盘与列表、切块保留来源页与 bbox、代建提交 + 挂题 + 打分 +
   错题本联动、软删后不再出现、代建幂等、非本班学生拒绝。
2. `flow_tests::photo_grading_flow_cuts_pages_grades_and_updates_insights` 走通整条链路：
   拍照 → 检测题块 → 切题挂到题 → 成绩单里见到照片 → 打分（错→错题本 NEW→改判对→MASTERED，
   掌握度随之变化）→ 删照片后成绩单不再显示。
3. `cargo test` **96 项全过**；真实库副本跑 0005→0007 干净（`user_version=7`、
   `local_attempt_asset` 列齐全、12 张本地表齐）；headless Chrome 截图验证批改台布局、
   切题对话框（编号蓝框 + 手动拖框）与 AI 建议卡片均正常。

仍未做（顺延）：中心 `desktop_courseware` + 动态清单、教学实体 outbox 推送、
讲义模板库、本地 OCR 兜底（无视觉模型时抄录不可用，此时老师手填即可）。
（中心 `submission_asset` 与学生拍照上传、学生绑定中心账号已在 §15.11 完成。）

### 15.11 阶段 2 补全：中心学生拍照提交 → 本地拉取（已完成，2026-11-17）

对应 §7.1 第二条入口。**设计上与原规划有三处不同**，都是实施时发现更合适的做法：

1. **照片不进 `submit` 的 `assetIds[]`**：照片和文字答案互不依赖（先拍后答、答完补拍都合理），
   绑在提交里会逼出「没作答就不能传图」与「交完不能补拍」。改成独立的
   `POST /api/edu/assignments/{id}/assets`。
2. **不走 `sync_pull`**：同步那条是题库的双向幂等合并（outbox + 批次重放），教学数据是
   「教师读自己班级」的只读查询，另开 `/api/desktop/edu/**`，复用 `ClassService` 的班级鉴权，
   身份由设备令牌决定，客户端无法冒充别人。
3. **绑定靠老师选，不靠标题猜**：本地作业与中心作业各自独立创建，自动对齐会悄悄错位；
   教师点一次「中心照片」选班级 + 作业即可。

| 层 | 落地 |
|---|---|
| 中心表 | `V99__submission_asset.sql`：作业 × 学生 × 序号的图片行（MinIO 存本体），`sha256` 去重 |
| 中心服务 | `SubmissionAssetService`：12MB 上限、文件头嗅探、同图幂等、成员/已发布校验、教师看全班、撤回软删 |
| 中心接口 | `/api/edu/assignments/{id}/assets`、`/api/edu/assets/{id}/data`；`/api/desktop/edu/**`（班级 / 作业 / 全班照片 / 图片） |
| 中心前端 | 学生作业详情「作答照片」：拍照/选图、缩略图、撤回；浏览器内缩到长边 2200px 再传 |
| 本地迁移 | `0011_center_submission.sql`：`center_assignment_id(_title)`、`center_asset_id`、`origin`、中心照片唯一索引 |
| 本地模块 | `center_assets.rs`：列举/绑定/拉取；学生匹配先按 `center_user_id`、再按班级内唯一同名，同名不猜；`grading::store_external_page` 落盘登记 |
| 本地界面 | 作业卡「中心照片」：绑定 → 拉取 → 新增/跳过/未匹配；未匹配可下拉直接绑学生后重拉 |

#### 验收（2026-11-17）

1. 中心 `SubmissionAssetServiceTest` 9 项（类型、幂等、越权、未发布、超大/非图、教师权限、名单）。
2. 本地 `center_assets::tests` 5 项（账号优先、唯一同名、同名不猜、无姓名、去重与 `origin`）。
3. 中心 `mvn test` 179 项全过；本地 `cargo test` 132 passed / 2 ignored，0 警告；
   本地 `app:build` 0 警告、dmg 校验通过；迁移版本 11。

仍未做：教学实体 outbox（把本地作业推成中心作业），因此当前需要中心已有一份对应作业；
讲义模板库；阶段 A′（Mac 端相机，两条实现路径都受构建或真机验证限制，未硬做）。

### 15.12 学情双视角 + 钥匙串反复授权（已完成，2026-11-17）

用户两条反馈：①「批量归类每次都要授权」；②「学情不光是班级的，也要看到个人的」。

**① 授权根因不是权限配置，是读得太勤。** Key 存在系统钥匙串（硬规则：SQLite 只存引用），
而 `run_classify` 是「每道题一次 `llm::complete`」，每次 `call_once` 都 `get_key()` 读一次钥匙串——
macOS 每次读都弹一次授权框，30 道题就是 30 次。修法：

| 改动 | 说明 |
|---|---|
| `llm::KEY_CACHE` | 进程内 `OnceLock<Mutex<HashMap<String, Option<String>>>>`，一个进程只问一次钥匙串；`None` 也记账（「确实没存过」），避免每次调用都重试 |
| 失效时机 | `set_key` 写入即更新缓存、`delete_key` 移除；授权被拒（非 `NoEntry`）不记账，用户下次点「允许」仍可用 |
| `has_key` 回填 | 原来每次启动都对 `has_key=0` 的模型读一次钥匙串（本地 ollama 永远为 0 → 每次启动弹一次）；改为 `app_setting.llm.has_key_backfilled` 只做一次 |

**② 学情从「班级里找个人」改成「两栏视角」。** 原实现只有班级矩阵，个人藏在抽屉里，
必须先选班、再找人；学生不在班 / 只想看某个孩子时没有入口。现在：

- `#/insights` 班级视角（矩阵、共性错题、班均 / 得分率 / 待批改，原样）；
- `#/insights/student` 学生名单（新增 `insights_roster`）：跨班、可筛班、可搜姓名年级；
  一行给出掌握度、近两周涨跌、三个薄弱知识点、待复习数、待批数、最近练习；
  四种排序——薄弱优先 / 退步最多 / 待复习多 / 最久没练；顶部六个画像数字；
- `#/insights/student/{id}` 个人整页（`pages/StudentInsight.jsx`）：位置（名次 / 班均 / 近两周变化）
  → 先补哪三个知识点 → 8 周趋势 → 知识点明细 → 错题本 → 最近作业；
- 视角进 URL：从名单进个人再返回，仍停在「学生」，不会跳回班级。

口径上补了三处，都与既有公式一致、不引入第二套算法：
「有数据」= 有已判分作答；「薄弱」= 掌握度 1–39（班级 KPI 同口径）；
「薄弱知识点」= 掌握度 < 70（与「按薄弱点出题」同口径）。个人页的知识点列表顺手修掉一个显示
问题：未作答知识点的 `mastery=0` 原来会排在薄弱点前面，现在统一排到最后并标「未作答」。

| 层 | 落地 |
|---|---|
| 后端 | `insights::roster`（`load_graded_every` / `pending_every` / `due_by_student` / `score_rate_every` 一次查完，不在循环里查库）；`student_insight` 增 `classNames` / `classAvg` / `rank` / `classSize` / `delta` / `weak` |
| 命令 | `insights_roster`（`permissions/local.toml` + `main.rs` 同步登记）+ `api.insightsRoster` |
| 前端 | `pages/Insights.jsx` 拆成壳 + `ClassView` + `RosterView`；新增 `pages/StudentInsight.jsx`；`App.parseHash` 支持第三段路由 |
| 样式 | `app.css`「学情 v2」段：`.stu-hero` / `.stu-grid` / `.weak-*` / `.roster-*`，窄屏两行折叠 |

验收：本地 `cargo test` 141 passed / 2 ignored（新增 `flow_tests::insights_roster_lists_students_and_ranks_inside_class`，
覆盖薄弱优先排序、未交排最后、班均与名次、按班 + 关键字筛选）；`npm run build` 通过；`node scripts/verify.mjs` 全绿。

仍未做：学情导出（PDF / 家长版周报）、跨班合并的学生档案视图、按知识点直接组卷补弱（当前只跳题库）、
「久未练」的提醒推送（只统计不通知）。

### 15.13 学号 · 成绩 · 学情诊断（已完成，2026-11-18）

用户三条要求：①「学生最好有学号」；②「包括成绩的管理」；③「重点在于分析学情，给出合理的安排建议」，
并明确「可以接入 AI」。这一轮把「记录」补成「决策」：数据能落到人（学号）、能落到分数（考试）、
最后能落到「这周做什么」（诊断与安排）。

**一、学号不是装饰，是「对不上人」的兜底。**
真实场景里对名单靠三个东西：学号、姓名、班级。此前只有姓名 + 年级，两个张伟就要靠猜；成绩单更是必须有学号，
否则录分顺序和纸质成绩册对不上。做法：

- `local_student.student_no`，非空时**全局唯一**（部分索引）；空允许（老数据、临时学生）；
- 新建留空 → 自动编号：取现有纯数字学号的最大值 +1，最小 1001；手工填的非纯数字（`2026-01`）不参与计算，
  免得把序号顶到天上；
- 导入**按表头认列**（`学号 / 姓名 / 年级 / 备注`，顺序随意），无表头时先按老格式 `姓名, 年级, 备注`，
  只有「第一列纯数字 + 第二列不是」才右移一位认学号；批内重号、库里已占用都在预览里标出来，不静默合并；
- 学号进搜索、进班级矩阵、进学生名单、进个人页页头、进成绩单与 CSV。

**二、成绩与作业分开，是因为「分从哪来」不一样。**
作业是按题判分的，分数是系统汇总的；考试常常是纸笔考完、整卷一个总分。硬塞进 `local_attempt` 会出现
「一道 120 分的伪题目」这种脏数据。所以：

| 表 | 粒度 | 谁产生 |
|---|---|---|
| `local_attempt` | 一题一行 | 学生提交 / 老师批改 / 拍照批改 |
| `local_exam_score` | 一人一行（整卷总分） | 老师登分 |

口径写死，避免两套算法：

- **缺考 = null**，与 0 分严格区分：不进平均、不进名次、不占分数段；
- 平均 / 中位数 / 最高 / 最低只算有分的人；及格率 / 优秀率的分母也是「有分的人数」；
- **并列同名次**（92、88、88 → 1、2、2）；
- 及格线 / 优秀线留空按满分 60% / 85% 算；改满分不会把已录分数带歪；
- 分数段按满分**百分比**切五档，100 分制与 150 分制形状可直接比。

**三、诊断分两层：规则保底，AI 补解释。**
只上 AI 的问题很清楚：没配模型 / 断网 / 模型抽风时，老师看到的是一个转圈或一句报错，学情页等于废了。
所以先写一版**规则版**，它本身就能用：

- 结论（headline）：带数字，说清「几分、几个薄弱点、先补哪个」；退步 ≥6 分先止损；
- 问题（issues）：每个薄弱点给「为什么」与「做几道题」；题量 <4 的先补题量，不急着下结论；
- 本周安排（plan）：周一错题、周二 / 周四各一个薄弱点、周三错题隔天重做、周五综合、周六回看、周日休息；
  每天 10–25 分钟，两周没练的先恢复手感；
- 给家长的一句话：说清本周在家怎么配合，不制造焦虑。

AI 版只做「在规则之上补解释」：把 `Facts`（掌握度、题量、名次、班均、薄弱点、到期错题、最近考试、闲置天数）
序列化成 JSON 给模型，要求回同一套结构。**硬约束三条**：

1. 只给数字，不给题目原文、不给对话历史；不猜性别性格家庭；
2. 输出必须过 `validate()`：缺字段、天数 <3、任务为空，哪一段不合格就那一段退回规则版；
3. 报告存 `local_insight_report`，带**数据指纹**（快照 JSON 的哈希）：数据没变不重复调模型；
   有新作答 / 新分数后卡片提示「数据已更新，建议重新诊断」，不是自动烧钱。

模型调用走独立连接 + 后台线程（`blocking` + `db::open`），模型慢的时候不占主库锁；
新增用途 `advice`（「学情建议」），没配路由时 `candidates()` 自动退回全部已启用模型。

| 层 | 落地 |
|---|---|
| 迁移 | `0013_scores.sql`：`local_student.student_no` + 唯一部分索引、`local_exam`、`local_exam_score`、`local_insight_report` |
| 学生 | `teaching.rs`：`student_no`、`next_student_no`、`student_no_taken`、表头识别与批内重号拦截；`Student` / `StudentInput` / `ImportRow` / `ImportPreview.conflicts` |
| 成绩 | `scores.rs`：`exam_save` / `exam_list` / `exam_sheet` / `exam_scores_save` / `exam_delete` / `student_exams`，名次与统计在 Rust 侧算（不写 SQL 窗口函数，便于测试） |
| 诊断 | `advice.rs`：`Facts` 快照、`rule_plan` / `rule_issues` / `rule_strengths` / `rule_parent_note`、`generate`（AI + 校验 + 兜底）、指纹缓存 |
| 提示词 | `llm/prompts.rs::advice`：限定只依据给到的数字、不推荐教辅、每天 10–30 分钟、固定 JSON 结构 |
| 命令 | `score_exams` / `score_exam_sheet` / `score_exam_save` / `score_exam_delete` / `score_save` / `score_student_exams`、`insight_advice`（读）、`insight_advice_generate`（异步生成） |
| 前端 | 新 `pages/Scores.jsx`（考试卡 + KPI + 分数段 + 登分表 + CSV）；`pages/StudentInsight.jsx` 加诊断卡与「考试成绩」卡；`Classes.jsx` 表单 / 表格 / 导入预览加学号；`Insights.jsx` 名单与矩阵显示学号；`App.jsx` 新增「成绩」导航与路由 |

验收：本地 `cargo test` 152 passed / 2 ignored（新增 3 项：学号导入与自动编号、考试成绩统计与名次、
诊断规则版与无模型兜底），0 警告；`npm run build` 通过；`node scripts/verify.mjs` 全绿。

仍未做：按分数段直接组卷、班级层面的 AI 诊断（当前只到个人）、跨班学生档案合并。
（合并趋势与家长周报已在 §15.14 补齐。）

### 15.14 合并趋势 + 家长周报（已完成，2026-11-18）

上一节把「记录」补成了「决策」，但决策还锁在老师的屏幕里：家长看不到，老师也没有一条能横向看的曲线。
这一节做两件事——把两类分数合成一条时间轴，再把整页学情压成**一张 A4**。

**一、合并的前提是换单位，不是换表。**
作业分数是「按题判分后汇总的得分率」，考试是「整卷一个总分」，量纲都不一样：10 分小练、100 分小测、
150 分月考放在同一根轴上，线条的起伏只反映纸面大小。所以 `progress.rs` 只做一件事——**统一折算百分制**
（`得分 / 满分 × 100`），再按日期混排：

- 作业取每个作业**最新一次提交**的汇总（`superseded_at` 已排除重交的旧版本）；
- 考试取 `local_exam_score` 的整卷分，班均同样折成百分制，跨 100 / 150 分制可以直接比；
- **缺考不进时间轴**：`null` 不是 0 分，画上去会把趋势线拉出一个假的谷底；同理「作业还有题待批」时
  得分率天然偏低，带 `pending` 出去，让界面和报告都标明「部分待批」，不把它读成退步；
- 「最近三次 vs 之前三次」的差（百分点）给一句话结论，样本少于四次不下判断。

**二、周报是「给家长看的一页纸」，不是学情页的截图。**
硬约束是**一页 A4**：两页的报告家长只会看第一页。为此做了三处取舍：

1. **前端拼自包含 HTML**，复用试卷打印那条链路（写 `prints/*.html` → 系统浏览器 → ⌘P 打印 / 存 PDF）。
   预览与打印是同一份 HTML，不会出现「屏幕上挺好、打出来错位」；也不为一个周报引 PDF 库。
2. **图表是内联 SVG**，不引图表库：掌握度折线（85 分目标线）+ 作业 / 考试混排（圆点作业、方块考试、60 分及格线），
   两张图并排占半栏，坐标文字按半栏宽重新定 viewBox（`330×96` / `330×106`），否则按整栏宽缩放会把字压成 6px。
3. **删掉重复信息**：不再单列「最需要先补的知识点」表（「需要补的地方」里已经带掌握度、原因和做法），
   明细表只留最近 4 条（图里已经能看到全部点位），换来的是留白与可读性。

版式：校名位（学情周报）→ 姓名 / 学号 / 班级 / 数据截至 → 四格 KPI（掌握度、班内位置、得分率、待复习错题）→
一句话结论 → 两列（已站住 / 需要补）→ 两张趋势图 → 最近记录 → 本周安排（含总时长与日均）→
给家长的话 → 家长反馈 / 签字 → 口径脚注。

**三、两条安全边界。**
姓名、知识点、作业标题全部过 `esc()`——周报是拼字符串，一个 `<` 就能把版式打坏；
周报里**只有数字和结论，没有题目原文、没有聊天记录**，与 AI 诊断的护栏同一条线。

| 层 | 落地 |
|---|---|
| 趋势 | 新 `progress.rs`：`ProgressPoint` / `Progress`、`assignment_points`、`exam_points`、`delta_of`、`note_of`（百分制折算、缺考跳过、正序混排、涨跌结论） |
| 报告 | 新 `src/lib/report.js`：`buildReportHtml`（KPI、两列结论、两个 SVG 图表、明细表、周计划、签字栏）+ 打印态样式（`@page A4`） |
| 前端 | `pages/StudentInsight.jsx`：新增「作业与考试」卡（`ProgressChart`，圆点 / 方块 + 图例）、页头「家长周报」按钮；`styles/app.css` 加 `.prog-*` |
| 命令 | `student_progress(studentId, limit)` → `progress::student_progress`（默认 12 次，夹在 3–60） |
| 文档 | `help/insights.md` 补「作业与考试」「家长周报」；`README.md` 功能表与验收第 22 条 |

验收：`cargo test` 155 passed / 2 ignored（新增 `progress_mixes_assignments_and_exams_on_percent_scale`：
百分制换算、正序混排、缺考跳过与班均、两次均值、涨跌方向），0 警告；`npm run build` 通过；
周报样例用 headless Chrome 实测：`--print-to-pdf` 为**一页**（`.sheet` 内容高 1025 px < A4 可印 1047 px），
目检两张图与表格对齐、无裁切；`node scripts/verify.mjs` 全绿。

仍未做：按分数段 / 薄弱点直接组卷（当前只跳题库）、班级层面的 AI 诊断、跨班学生档案合并、
周报的图片（PNG）导出（现在走浏览器「存为 PDF」，够用）。
