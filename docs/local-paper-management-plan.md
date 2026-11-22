# 本地题库 · 试卷管理与可视化排版规划

> 状态：**P0 已实现（2026-11-18）；P1 的蓝图组卷 / 换题撤销 / 打印记录已实现，模板工作室与 PDF 导出未做**｜最后更新 2026-11-18｜目标：`local-client/`（Tauri 2 + React 19 + SQLite，离线优先）
>
> 参照项目：[`worm-vue3-print`](../../worm-vue3-print)（Vue 3 可视化打印模板设计器 + 表达式引擎 + 同构渲染管线，MIT）
>
> 关联文档：[本地题库客户端规划](local-question-bank-client-plan.md)、[题库与试卷（中心系统）](../../neural-knowledge-base/docs/product/question-bank-and-papers.md)、[学习闭环](../../neural-knowledge-base/docs/product/learning-loop.md)、[主观题批改与班级作业](../../neural-knowledge-base/docs/product/subjective-grading-and-class-assignments.md)

---

## 1. 结论先说

**一句话**：在本地题库里新增独立的「试卷」模块，用**结构化组卷 + 流式排印**生成卷面，用**模板设计器**设计封面 / 卷头 / 答题卡等固定版式部件，最终输出可打印的 HTML（浏览器打印 / PDF）。

三条关键取舍：

1. **试卷正文不做自由画布**。一套卷 20–30 题、题干含公式与图片、题目块必须整块跨页、题号与分值自动编排——这本质是**长文档流式排版**，不是"拖 30 个绝对定位元素"。自由画布做不出可靠的跨页，也不会有老师愿意手动摆。
2. **可视化设计分两层**：卷面本身**所见即所得**（实时预览 + 版面参数），固定版式部件（封面 / 答题卡 / 卷头 / 听力材料 / 答案页）交给 `<print-designer>` 自由设计。两层共用同一套渲染与打印链路。
3. **打印先用"打印 HTML + 系统浏览器打印"落地**，PDF 导出与静默打印作为后续增强。理由：Tauri 的 WKWebView 不保证 `window.print()`，而"生成 HTML → 系统浏览器打开 → 打印/另存 PDF"零依赖、跨平台、当天能验收；PDF 直出需要 Rust 侧 `WKWebView.createPDF` / WebView2 `PrintToPdf`，属于技术验证项，不该挡住 MVP。

### 1.1 已确认的产品口径（2026-11-17）

| 问题 | 决定 | 落地要点 |
|---|---|---|
| 纸型 | **可选，默认 A4** | 版面面板提供 A4 / A3 / B5 / 8K / 16K / 自定义（mm）；新建默认 A4 纵向，切换后按新纸宽重算分页 |
| 答案页位置 | **卷末，且必须新开一页** | 教师版在正文最后一页之后强制分页，答案标题不与最后一道题的题尾同页；分页器预留整页，不做"接着排" |
| 答题卡 | **可选，默认 A4 双栏** | 默认生成 A4 纵向、双栏涂卡区（选择题按题号 4 列 × N 行，填空/解答分块）；可切换 A3 对折、单栏，也可关掉不打印 |
| 分值 | **默认按标准卷统一分值，总分固定** | 大题统一分值（如单选每题 5 分）；允许逐题微调，但 `sum(题分) == 卷面总分` 才能定稿；差额实时显示，不做静默四舍五入 |

> 这四条直接决定了 §5 的字段、§6.2 的分值校验和 §8 的组装顺序，后续实现不得擅自改成"按难度自动给分"或"答案接排"。

---

## 2. 需求与范围

### 2.1 用户故事

| # | 角色 | 场景 | 优先级 |
|---|---|---|---|
| U1 | 教师 | 从题库按知识点 / 题型 / 难度挑题，组成一套单元测，调好分值与顺序 | P0 |
| U2 | 教师 | 一眼看到卷面长什么样（A4、题号连续、选项分栏、解答题留白），改完即时生效 | P0 |
| U3 | 教师 | 打印学生版（无答案）与教师版（含答案解析），页眉页脚、页码、装订线正确 | P0 |
| U4 | 教师 | 按"双向细目表"自动组卷：每个知识点几道、什么难度、多少分 | P1 |
| U5 | 教师 | 某道题不合适，换一道同知识点同题型的题，能撤销 | P1 |
| U6 | 教师 | 用可视化设计器做封面、答题卡、卷头，存成模板复用 | P1 |
| U7 | 教师 | 把已定稿试卷发布成班级作业，批改后回填得分 | P2 |
| U8 | 教师 | 一次打印多个班级 / 多份，或走静默打印不弹系统对话框 | P2 |

### 2.2 明确不做（本期）

- **不做**在线作答、自动判分（那是作业 / 批改台的职责，见 `local_assignment`）。
- **不做** Word 导出（HTML/PDF 已覆盖打印需求；docx 版式不可控）。
- **不做**多校 / 多租户、云端协同排版。
- **不做**把题库所有题拖成一张自由画布（见 §1 第 1 条）。
- **暂不做**A/B 卷选项乱序、听力音频嵌入（P2 评估）。

---

## 3. 参照项目分析

### 3.1 它能给什么

| 能力 | 包 / 位置 | 对本项目的价值 |
|---|---|---|
| 零框架设计器 `<print-designer>`（Custom Element + Shadow DOM） | `@worm-vue3-print/common` | **React 可直接挂载**，不需要引入 Vue 运行时；素材台 / 画布 / 属性台 / 图层面板开箱即用 |
| 模板 JSON 契约（`TemplateData` / `ElementOptions`，单位 mm） | `@worm-vue3-print/core`、`contract` | 模板可版本化、可校验、可迁移；13 种内置元素足够做封面与答题卡 |
| 同构渲染管线（绑定 → 测量 → 分页 → 出 HTML） | `core/render`、`core/browser` | 一套代码同时服务预览与打印；分页、页码、水印、条码都有现成实现 |
| 多页面模板 + 批量数据 | `renderHtmlPages(tpl, data[])` | 封面 / 正文 / 答题卡可拼成一份文档；批量份数一次出纸 |
| 静默打印客户端 + 浏览器 SDK | `clients/print-client`、`core/client` | P2 可选；不引入也不影响 MVP |
| 表达式与字段绑定 | `core` 表达式引擎 | 卷头 `{paper.title}`、`{paper.totalScore}`、`{paper.date}` 直接绑定 |

### 3.2 它不能直接给的

| 需求 | 现实 | 结论 |
|---|---|---|
| 整卷自动流式排版（题目块跨页、题号连续、选项 2/4 列、解答题留白） | `flowList` 是"小格子流式列表"（标签 / 短文本），不是任意富文本块；`autoHeight` 元素不会在页间重新流动 | **自研试卷排印器**，不复用其分页做正文 |
| 数学公式（KaTeX） | core 有 `html` 元素，但不了解题库的 LaTeX 归一化规则 | 复用本地 `splitLooseMath` + `latexToPlain`，预渲染成 HTML 再交给模板 |
| 题库语义（知识点、题型、难度、分值、换题） | 完全无 | 本地实现 |
| 打印对话框 | 浏览器 `window.print()`；Electron 客户端另有链路 | Tauri 下需单独验证（见 §9.3） |

### 3.3 依赖可用性（已核实，2026-11-17）

```text
npm view @worm-vue3-print/common version      → 1.4.8   ✅ 可安装（deps: core / jsbarcode / qrcode / dompurify）
npm view @worm-vue3-print/core   version      → 1.4.8   ✅
npm view @worm-vue3-print/plugin-kit          → 404     ❌ 未发布
```

> **坑**：`common@1.5.0` 的 `dependencies` 里有 `@worm-vue3-print/plugin-kit@^1.5.0`，而该包未发布到 npm，**直接 `npm i @worm-vue3-print/common` 会解析失败**。因此锁 **`1.4.8`**（`"@worm-vue3-print/common": "1.4.8"`，精确版本，不用 `^`）。
>
> 备选：若后续需要 1.5.x 的原生插件能力，从本地 monorepo `npm run build -w @worm-vue3-print/common` 后以 `file:` 依赖引入；但上游仓库已公告进入维护状态并迁移到 `@worm-print-platform/*`，**接入层要包一层 adapter**（§9.4），换包时只改一个文件。

---

## 4. 总体方案

### 4.1 分层架构

```
┌─────────────────────────────────────────────────────────────────┐
│  React 页面层                                                     │
│  Papers(列表)  PaperWizard(组卷)  PaperEditor(编辑)              │
│  PaperPreview(打印预览)  PaperTemplates(模板工作室)              │
└───────────────┬─────────────────────────────┬───────────────────┘
                │ 结构化文档                    │ 固定版式模板
                ▼                             ▼
┌──────────────────────────────┐   ┌───────────────────────────────┐
│ PaperDoc（试卷文档模型）      │   │ TemplateStudio                │
│ 卷头 / 大题 / 题目 / 分值     │   │ <print-designer> (Custom El.) │
│ 版面参数（纸型/分栏/留白）    │   │ 封面 / 答题卡 / 卷头 / 答案页 │
└──────────────┬───────────────┘   └──────────────┬────────────────┘
               │ 编译                              │ renderHtmlPages
               ▼                                   ▼
┌─────────────────────────────────────────────────────────────────┐
│ 排印引擎 paper-render（前端 TS）                                 │
│  1. 题目 → HTML 块（KaTeX 预渲染、选项分栏、答题留白）            │
│  2. 分页：测量块高 → 切页（块不跨页）→ 注入页眉页脚/页码          │
│  3. 组装：封面(可选) + 正文 + 答案页(教师版，卷末新页) + 答题卡(默认开) │
│  4. 打印样式：@page / 装订线 / 双面奇偶 / 灰度                    │
└──────────────┬──────────────────────────────────────────────────┘
               ▼
        iframe 预览 ──► 系统浏览器打印 ──► 纸张
               └────► 导出 HTML / PDF（P1）
```

### 4.2 为什么这样切

| 方案 | 优点 | 致命问题 | 结论 |
|---|---|---|---|
| A. 全部交给 `<print-designer>` 自由画布 | 一套技术栈、可视化最强 | 长文档无流式容器；跨页要手工拆；30 题手工摆不可能 | ❌ |
| B. 全部自研（连模板设计也自己写） | 完全可控 | 重复造轮子，属性面板 / 素材台 / 图层 / 撤销至少 2 周 | ❌ |
| **C. 结构化正文 + 模板设计器做固定部件（推荐）** | 正文可靠、模板灵活、工作量可控 | 需要一层"模板插槽"协议 | ✅ |

「模板插槽」：试卷文档在固定位置预留 `cover`、`header`、`answerSheet`、`answerKey`、`footer` 五个插槽，每个插槽绑定一个模板 ID；没配模板时用内置默认版式（保证零配置也能打出像样的卷子）。

---

## 5. 数据模型

新增迁移 `src-tauri/migrations/0012_papers.sql`（当前 `user_version = 11`，新增后为 12；照既有风格在文件尾部写注释版回滚 SQL）。

### 5.1 表结构（草案）

```sql
-- 试卷主表：只存"编排"，不存题目内容（内容在快照里）
create table if not exists local_paper (
    id            text primary key,
    title         text not null,
    subject_id    text references subject (id) on delete set null,
    grade         text not null default '',        -- 年级，如「高二」
    exam_type     text not null default 'EXAM',    -- EXAM 考试卷 / PRACTICE 练习卷 / QUIZ 小测
    status        text not null default 'DRAFT',   -- DRAFT / FINAL / ARCHIVED
    -- 卷面总分：创建时按标准卷模板定死（如 150），之后只允许在「重新设置总分」里改，
    -- 改完必须重新配平；定稿要求 sum(题分) 与它完全相等，见 §6.2。
    total_score   real not null default 0,
    score_scheme  text not null default '',        -- 标准卷分值方案，如 'XGK1_MATH'
    duration_min  integer not null default 0,
    question_count integer not null default 0,
    layout_json   text not null default '{}',      -- 版面参数，见 §6.4
    blueprint_json text not null default '{}',     -- 组卷蓝图，便于"按原参数再组一套"
    slots_json    text not null default '{}',      -- 模板插槽：cover/header/answerSheet/answerKey
    note          text not null default '',
    created_at    text not null,
    updated_at    text not null,
    finalized_at  text,
    deleted_at    text
);
create index if not exists ix_local_paper_list on local_paper (status, updated_at desc, deleted_at);

-- 大题（一、选择题 / 二、填空题 …）
create table if not exists local_paper_section (
    id         text primary key,
    paper_id   text not null references local_paper (id) on delete cascade,
    ord        integer not null default 0,
    title      text not null default '',
    note       text not null default '',          -- 大题说明，如「每小题 5 分，共 40 分」
    kind       text not null default '',          -- 题型约束，可空表示混合
    -- 分值口径：UNIFORM = 本大题统一分值（默认，如单选每题 5 分）；
    --           PER_ITEM = 逐题自定（仍受卷面总分约束）。
    score_mode text not null default 'UNIFORM',
    unit_score real not null default 0,           -- UNIFORM 模式下每题分值（改它即整题重算）
    layout_json text not null default '{}'        -- 分栏数、选项排列、留白行数（覆盖卷级）
);
create index if not exists ix_local_paper_section on local_paper_section (paper_id, ord);

-- 试卷题目：快照 + 分值 + 顺序
create table if not exists local_paper_question (
    paper_id     text not null references local_paper (id) on delete cascade,
    section_id   text not null references local_paper_section (id) on delete cascade,
    ord          integer not null default 0,
    question_id  text references question (id) on delete set null, -- 允许原题被删，卷仍可打
    score        real not null default 0,
    snapshot_json text not null,                  -- 题干/选项/答案/解析/难度/知识点/图片
    snapshot_at  text not null,
    flags_json   text not null default '{}',      -- 是否锁定、是否隐藏、换题前记录
    primary key (paper_id, section_id, ord)
);
create index if not exists ix_local_paper_question_q on local_paper_question (question_id);

-- 固定版式模板（封面 / 答题卡 / 卷头 / 答案页）
create table if not exists local_paper_template (
    id            text primary key,
    name          text not null,
    kind          text not null default 'COVER',  -- COVER / HEADER / ANSWER_SHEET / ANSWER_KEY / FULL
    template_json text not null,                  -- <print-designer> 保存的模板 JSON
    thumb         text,                           -- 封面缩略图（可选，base64 / 本地路径）
    builtin       integer not null default 0,     -- 内置模板，不可删
    created_at    text not null,
    updated_at    text not null
);

-- 打印记录（谁、什么时候、打了哪版、几份）
create table if not exists local_paper_print (
    id         text primary key,
    paper_id   text not null references local_paper (id) on delete cascade,
    variant    text not null default 'STUDENT',   -- STUDENT / TEACHER / ANSWER_SHEET
    copies     integer not null default 1,
    pages      integer not null default 0,
    channel    text not null default 'BROWSER',   -- BROWSER / PDF / SILENT
    printer    text not null default '',
    created_at text not null
);
```

### 5.2 快照策略（关键）

- **加入试卷即快照**：把 `stem / options / answer / analysis / difficulty / type / knowledge 名称 / 图片引用` 拷进 `snapshot_json`。
- **题库改动不自动影响历史卷**：老师改题不应让上周已发的卷子变形。编辑卷面时提供「同步题库最新版」按钮，逐题或全量刷新并提示差异。
- **定稿冻结**：`status = FINAL` 后禁止换题、禁止改分值（要改先 `reopen` 回 `DRAFT`，并在 UI 明示"已打印过的卷子不会跟着变"）。
- **题被删也能打**：`question_id` 置空，快照仍在；卷面标一个极小的「原题已删除」角标（仅编辑态可见）。
- **快照体积**：单题 JSON 约 2–8 KB，30 题 < 300 KB，可直接放 SQLite；图片只存引用，不重复拷贝二进制。

### 5.3 状态机

```
        ┌──────────── reopen ────────────┐
        ▼                                │
   DRAFT（草稿） ── finalize ──► FINAL（已定稿） ── archive ──► ARCHIVED
        │                                                        │
        └──────────────── 删除 → 回收站（deleted_at）◄───────────┘
```

- `DRAFT`：可增删题、换题、改分值、改版面。
- `FINAL`：只读（可打印、可复制为新卷、可归档）。
- `ARCHIVED`：列表默认隐藏，可恢复。
- 与既有题库一致：删除是软删，列表/统计默认带 `deleted_at is null`。

---

## 6. 组卷与编辑

### 6.1 手动组卷（P0）

入口两条：

1. **试卷页 → 新建 → 从题库挑题**：右侧抽屉，复用 `listQuestions` 的筛选（学科 / 知识点 / 题型 / 难度 / 关键词 / 来源），多选后「加入第 X 大题」。
2. **题库页多选 → 更多 → 加入试卷**：延续题库现有批量操作条，选已有草稿卷或新建。

规则：

- 已在卷内的题在候选列表置灰并标「已在本卷」，不允许重复加入（同一题出现两次在考试卷里几乎总是错误）。
- 加入时不按难度猜分，而是**跟随所在大题的统一分值**（新建大题时按标准卷方案预填，见 §6.2）；老师可在题目行上逐题改，改动立刻反映到"已分配 / 总分"差额条。
- 题号在渲染时**动态生成**，不落库（`ord` 只表示顺序）；删除一题后自动重排，避免出现"第 7 题缺失"。

### 6.2 分值体系（总分固定，2026-11-17 确认）

四条规则，实现时按这个顺序校验：

1. **总分先定**。新建试卷选"标准卷方案"（内置：新高考Ⅰ卷数学 150 分、通用 100 分、单元测自定义），`total_score` 落库后即视为**合同值**；卷面显示「总分 150」不可直接编辑，改总分走单独的「重新设置总分」并二次确认（已有分值会全部重算）。
2. **默认按大题统一分值**。每个大题有 `score_mode = UNIFORM` 与 `unit_score`：改 `unit_score` 就把本大题所有题重算（如 8 道单选 × 5 分 = 40 分）。大题说明自动生成「本大题共 8 小题，每小题 5 分，共 40 分」，不用手写、不会写错。
3. **允许逐题微调**。切到 `PER_ITEM`（或直接改某一题）后，该题独立记分；调完必须仍然满足 `sum(题分) == total_score`。
4. **差额可见、定稿卡死**。
   - 编辑器顶部常驻差额条：`已分配 148 / 总分 150 · 还差 2 分`，正数绿色、负数红色。
   - `DRAFT` 允许不平衡（组卷中途本来就会差几分），但**顶部一直提示**。
   - `finalize` 时后端强校验：`abs(sum - total_score) < 0.001` 才通过；不通过返回具体差额与"哪几个大题可调"，前端高亮对应行。
   - 不做静默四舍五入、不自动把差额塞给最后一道题——分值是要印在卷面上的，必须由人确认。

标准卷方案（`score_scheme`）只是一组默认值，不是硬约束：

```json
{
  "id": "XGK1_MATH",
  "name": "新高考Ⅰ卷 · 数学",
  "totalScore": 150,
  "durationMin": 120,
  "sections": [
    {"title": "一、单项选择题", "kind": "SINGLE", "count": 8,  "unitScore": 5},
    {"title": "二、多项选择题", "kind": "MULTI",  "count": 3,  "unitScore": 6},
    {"title": "三、填空题",     "kind": "FILL",   "count": 3,  "unitScore": 5},
    {"title": "四、解答题",     "kind": "CALC",   "count": 5,  "unitScore": 0, "scoreMode": "PER_ITEM"}
  ]
}
```

> 解答题默认走 `PER_ITEM`（各题 12–17 分不等），选中方案后仍要老师逐题确认；方案只预填结构，不锁定分值。

### 6.3 蓝图组卷（P1）

"双向细目表"式组卷：

```json
{
  "subjectId": "subj-MATH",
  "scoreScheme": "XGK1_MATH",
  "totalScore": 150,
  "sections": [
    {"kind": "SINGLE", "count": 8, "scoreMode": "UNIFORM", "unitScore": 5, "difficulty": {"1": 2, "2": 4, "3": 2}},
    {"kind": "FILL",   "count": 4, "scoreMode": "UNIFORM", "unitScore": 5, "difficulty": {"2": 2, "3": 2}},
    {"kind": "CALC",   "count": 6, "scoreMode": "PER_ITEM", "unitScore": 0}
  ],
  "knowledgeQuota": [
    {"knowledgeId": "kp-...", "count": 3},
    {"knowledgeId": "kp-...", "count": 2}
  ]
}
```

算法（确定性，不引入模型）：

1. 每个（大题 × 知识点 × 难度档）算出配额；知识点未指定时按该学科题库分布加权。
2. 逐格抽题：只取 `PUBLISHED`、未在本卷、`deleted_at is null`；按"使用次数少优先 + 时间新优先"排序，取一条。
3. 某格不足 → 记录缺口 `{section, knowledge, difficulty, need, got}`，**不跨知识点顶替、不静默凑数**；生成完成后弹出缺口报告，允许「按缺口放宽难度」二次尝试。
4. 抽完输出为 `DRAFT`，蓝图写入 `blueprint_json`，可「按此蓝图再组一套」（B 卷 / 补考卷）。
5. **总分先校验后抽题**：`Σ(section.count × unitScore) + Σ(PER_ITEM 预估分)` 必须等于方案总分，不等直接报错，不给"先抽再算"的模糊结果；抽完若个别题缺失导致达不到总分，回滚整次生成（与缺口一起报）。

> 与中心系统一致：只取已发布题、缺口明确报出、不落库不凑数。区别是本地用确定性算法而非服务端模板表——离线、可解释、可复现（同一种子同结果）。

### 6.4 卷面编辑（P0）

三栏布局，复用现有设计语言（`PageHeader` / `Drawer` / `Field` / `Pill` / `Empty`）：

```
┌ 顶栏：卷名 · 状态 · 学生版/教师版 · 预览 · 打印 · 导出 ─────────────┐
├──────────────┬────────────────────────────────┬─────────────────┤
│ 结构树        │ 卷面实时预览（A4，可缩放）      │ 属性            │
│ 一、选择题 8  │  ┌──────────────────────────┐  │ 卷面：纸型/分栏  │
│  · 1 题 …     │  │ 2026 学年第一学期 …      │  │ 页边距/装订线    │
│  · 2 题 …     │  │ 一、选择题（每题 5 分）   │  │ 字号/行距/题距   │
│ 二、填空题 4  │  │ 1. 已知抛物线 …   [A][B] │  │ 答题留白         │
│  · …          │  │ …                        │  │ 页眉/页脚/页码   │
│ [+ 新增大题]  │  └──────────────────────────┘  │ 水印/密级        │
└──────────────┴────────────────────────────────┴─────────────────┘
```

版面参数（`layout_json`）：

| 参数 | 默认 | 说明 |
|---|---|---|
| `paperSize` | **`A4`（默认，可改）** | A4 / A3 / B5 / 8K / 16K / 自定义（mm）；切换时按新纸宽重算分页，不改字号 |
| `orientation` | `portrait` | 纵向 / 横向（横向多用于 A3 双栏） |
| `margins` | 上 20 / 下 20 / 左 20 / 右 20 mm | 单独可调；装订侧自动 +10 mm |
| `columns` | 1 | 1 / 2 栏；2 栏时题目块不跨栏（先按栏内高度切） |
| `fontFamily` / `fontSize` | 系统宋体 / 10.5pt | 正文、题干、选项、解析分别可覆盖 |
| `lineHeight` | 1.6 | |
| `questionGap` | 6 mm | 题与题之间 |
| `optionColumns` | 4（选择题）/ 1（其他） | 选项排布 2 / 4 / 自动 |
| `answerSpace` | 自动 | 解答题留白行数（按题型默认 6–10 行），可逐题覆盖 |
| `header` / `footer` | 内置 | 页眉：卷名 + 班级姓名栏；页脚：第 X 页 / 共 Y 页 |
| `bindingLine` | 关 | 装订线（虚线 + "装订线内不要答题"） |
| `watermark` | 关 | 文字水印（如"内部资料"），复用 core 的矢量水印 |
| `answerKeyPage` | 教师版开 | **卷末强制新开一页**（`break-before: page`），答案标题不与上一题题尾同页；封面/答案页不计入学生版页码 |
| `answerSheet` | **开，A4 双栏（默认，可关）** | 答题卡独立成页（P1），默认 A4 纵向双栏：选择题 4 列涂卡网格 + 填空/解答分块；可切 A3 对折 / 单栏 / 关闭 |

新建试卷写入的默认 `layout_json`（可整体被用户配置覆盖，字段只增不改）：

```json
{
  "paperSize": "A4",
  "orientation": "portrait",
  "margins": {"top": 20, "right": 20, "bottom": 20, "left": 20},
  "columns": 1,
  "fontSize": 10.5,
  "lineHeight": 1.6,
  "questionGap": 6,
  "optionColumns": 4,
  "answerSpace": "auto",
  "bindingLine": false,
  "watermark": null,
  "answerKeyPage": true,
  "answerSheet": {"enabled": true, "paperSize": "A4", "columns": 2}
}
```

### 6.5 换题与撤销（P1）

- 仅 `DRAFT` 可用；点换题**先出候选**，不直接替换。
- 候选：同学科 + 与本题至少共享一个知识点 + 同题型 + `PUBLISHED` + 排除本卷已用；可按难度、关键词、使用次数筛选。
- 替换保留题号、分值、大题归属。
- 撤销栈（内存 + `flags_json` 记录上一题 id 与快照），支持连续撤销到本次会话开始；关闭编辑器即清空（不做跨会话撤销，避免状态膨胀）。

---

## 7. 可视化设计（模板工作室）

### 7.1 设计器接入方式

依赖：

```bash
npm i @worm-vue3-print/common@1.4.8
npm i @worm-vue3-print/core@1.4.8       # 仅当需要直接调渲染管线 / 静默打印 SDK
```

一次性注册（`main.jsx`）：

```js
import { registerAll } from '@worm-vue3-print/common'
registerAll()   // 注册 <print-designer> 自定义元素
```

React 封装 `src/components/print/PrintDesignerHost.jsx` 的要点（照抄参照项目 `demo-common/apps/react`）：

| 事项 | 做法 |
|---|---|
| 挂载 | `<print-designer ref={elRef} is-edit="true" />`；`is-edit` 是字符串开关，可写 JSX 属性 |
| 注入模板 / 字段 | **必须走 property**：`el.template = obj`、`el.fields = [...]`；写 attribute 会被字符串化成 `[object Object]` |
| 字体声明 | `el.fonts = fontsOf(template)`，随模板保存 |
| 事件 | `addEventListener('save' \| 'preview' \| 'help' \| 'dblclick-element', fn)`，**cleanup 必须 remove**（StrictMode 双挂载会重复注册，导致一次保存两次） |
| 保存前校验 | `el.validateTemplate()` 返回 `{message}[]`，有错不落库 |
| 取模板 | `el.getTemplateJson()`（字符串）；或直接用 `save` 事件 detail |
| 主题 | 覆盖 `--pd-*` 令牌（`--pd-accent` 等）到本地配色，不穿透 Shadow DOM 写选择器 |

### 7.2 模板类型与字段契约

| kind | 用途 | 建议纸型 | 绑定字段 |
|---|---|---|---|
| `COVER` | 封面：学校、考试名称、科目、时间、考生信息、密封线 | A4 | `paper.title` `paper.subject` `paper.grade` `paper.duration` `paper.totalScore` `paper.date` |
| `HEADER` | 卷头：标题栏 + 姓名/班级/学号填写栏 + 注意事项 | A4 | 同上 + `paper.notice` |
| `ANSWER_SHEET` | 答题卡：选择题涂卡区、填空/解答答题框、题号网格；默认 A4 双栏，可切 A3 对折 / 单栏 | A4（默认）/ A3 | `paper.title` `paper.questionCount` `paper.sections`（明细数组，用表格元素） |
| `ANSWER_KEY` | 答案页版式（可选，默认用内置）；**卷末独占新页**，不接题尾 | A4 | `paper.title` `questions`（明细） |
| `FULL` | 整卷套模板（高级：需要用户自己保证分页） | 任意 | 全部 |

业务字段（`PrintBusinessField`）按扁平列表传入，`fieldKey` 用点分路径（`paper.title`），设计器会自动按首段分组；拖字段即插入 `{paper.title}` 表达式。

```js
const FIELDS = [
  { fieldKey: 'paper.title',        fieldLabel: '试卷名称', fieldType: 'string', sortOrder: 1 },
  { fieldKey: 'paper.subject',      fieldLabel: '学科',     fieldType: 'string', sortOrder: 2 },
  { fieldKey: 'paper.grade',        fieldLabel: '年级',     fieldType: 'string', sortOrder: 3 },
  { fieldKey: 'paper.duration',     fieldLabel: '考试时长', fieldType: 'number', sortOrder: 4 },
  { fieldKey: 'paper.totalScore',   fieldLabel: '总分',     fieldType: 'number', sortOrder: 5 },
  { fieldKey: 'paper.questionCount',fieldLabel: '题目数',   fieldType: 'number', sortOrder: 6 },
  { fieldKey: 'paper.date',         fieldLabel: '日期',     fieldType: 'string', sortOrder: 7 },
  { fieldKey: 'paper.school',       fieldLabel: '学校',     fieldType: 'string', sortOrder: 8 },
  { fieldKey: 'paper.examNo',       fieldLabel: '试卷编号', fieldType: 'string', sortOrder: 9 },
  { fieldKey: 'paper.sections',     fieldLabel: '大题列表', fieldType: 'list',   sortOrder: 10 },
]
```

### 7.3 模板与试卷的关系

- 模板独立成库（`local_paper_template`），一份模板可被多份试卷引用。
- 试卷只存 `slots_json = { cover: 'tpl-1', header: 'tpl-2', answerSheet: null }`；**不内联模板 JSON**，改模板对所有引用它的草稿卷生效；已定稿卷在 `finalize` 时把模板 JSON 快照进 `slots_json`（冻结）。
- 内置 4 套默认模板（封面 / 卷头 / 答题卡 / 答案页）随安装包分发，`builtin=1`，可"另存为"后改。
- 模板工作室入口：试卷编辑器右侧「版式」→「设计封面 / 设计答题卡」，或独立页 `#/papers/templates`。

### 7.4 正文为什么仍走排印引擎（再强调一次）

模板设计器的画布是**毫米绝对定位 + 固定页**。一道解答题题干可能是 8 行文字 + 一张图，下一道是 2 行选择题——只有"先测量、再流式堆叠、超页换页"才能稳定排版。设计器里的 `autoHeight` 只解决"元素自身变高"，不解决"后续元素跟着往下推、跨页续排"。因此正文必须是代码生成，而不是画布摆放。

> 折中方案（已评估后放弃）：把每道题生成一个 `longText` 元素，按 Y 坐标手工堆叠。题目增删一次就要重算全卷坐标，且跨页仍要手工拆块——复杂度高于自研排印器。

---

## 8. 打印与导出

### 8.1 打印 HTML 生成（`src/lib/paper/render.js`）

输入：`PaperDoc`（试卷 + 快照 + 版面参数）+ `variant`（STUDENT / TEACHER）。

步骤：

1. **题目块渲染**：按题型生成 HTML 片段
   - 题干：`Rich` 同源的 `splitLooseMath` → KaTeX `renderToString`（`throwOnError: false` + `latexToPlain` 兜底，与阅读端完全一致，避免"屏幕正常、打印红色报错"）。
   - 选项：`optionColumns` 决定 2 / 4 列网格；选项内公式用行内模式。
   - 答题留白：解答题插入 `.answer-space`（高度 = 行数 × 行高），教师版改为显示答案与解析。
   - 图片：优先 `convertFileSrc()` 的 `asset://`；打印前若检测到跨域/加载失败，回退 base64 内嵌（离线更可靠）。
   - 分值：题号后统一输出 `（5 分）` 样式，分值取 `local_paper_question.score`，与大题说明里的"每小题 X 分"必须同源，不允许两处各算一遍。
2. **分页**：在隐藏 iframe 中以真实字体测量每块高度（`getBoundingClientRect`），按内容区高度贪心切页；`break-inside: avoid` 兜底；页眉/页脚高度先扣除；双栏时先按栏切再按页切。
3. **页码回填**：先分页得到总页数，再把 `第 {n} 页 / 共 {m} 页` 写进每页页脚（不要依赖 CSS `counter(page)`，WKWebView 支持不完整）。
4. **组装**：`封面模板(可选) → 卷头 → 正文分页 → 答案页(教师版) → 答题卡(默认开)`，全部内联 CSS + base64 字体。
   - **答案页强制独占新页**：正文最后一个分页结束后插入 `break-before: page` 的容器，答案标题从页首开始；即使正文最后半页是空的也不回收——宁可留白，也不让答案接在题尾（2026-11-17 确认）。
   - **答题卡默认开启、A4 双栏**：作为独立整页追加，不在正文后接排；关闭时前端仍保留版式参数，下次新建沿用。
5. **输出**：单个 HTML 字符串（自包含、无外链、可直接双击打开打印）。

### 8.2 公式与字体（离线关键）

- KaTeX CSS 从 `node_modules/katex/dist/katex.min.css` 读取并内联；其中 `@font-face` 的 `url(fonts/...)` 改写成 base64（约 20 个 woff2，总体积 ~300 KB，可接受）。
- 中文字体不内嵌（体积不可控）：打印 HTML 声明 `font-family: "Songti SC", "SimSun", serif`，由系统提供；macOS / Windows 都自带宋体。若后续要求跨端像素一致，再评估打包思源宋体子集。
- 打印前 `await document.fonts.ready`，否则测量高度会偏小、分页错位。

### 8.3 输出通道

| 通道 | 实现 | 阶段 | 说明 |
|---|---|---|---|
| 应用内预览 | iframe + `srcdoc`，缩放档位、页码跳转 | P0 | 与最终打印同一份 HTML，所见即所得 |
| 系统浏览器打印 | 写临时 HTML 到数据目录 → `open` / `start` 打开 → 用户 `⌘P` | P0 | 最稳；macOS/Windows 都支持"另存为 PDF" |
| 导出 HTML | 保存对话框选路径 | P0 | 便于发给同事 / 归档 |
| 导出 PDF | macOS：Rust 侧 `WKWebView.createPDF` 或 `NSPrintOperation`；Windows：WebView2 `PrintToPdf` | P1（需 spike） | 先做 macOS，Windows 降级为"浏览器另存 PDF" |
| 静默打印 | 接 `print-client`（Electron，127.0.0.1:17521）+ `core/client` SDK | P2 | 需另装一个桌面客户端，与 Tauri 并存；仅对"每天打几百份"的场景值得 |

> **技术验证项（P0 第 1 天，0.5 人日）**：在 Tauri WebView 里执行 `window.print()` 是否弹窗。已知 WKWebView 默认不实现 `print:`，大概率无反应 → 因此默认走"系统浏览器打开"。验证结果写进 `docs/product/local-paper-print-spike.md`。

### 8.4 打印质量清单

✅ = P0 已验｜❌ = P0 未做（见行内说明）

- ✅ A4 实际尺寸 210×297 mm，四边页边距与设置一致（打印对话框关闭"缩放以适应"）。`.paper-page` 按 mm 定死宽高、`@page { margin: 0 }`，页边距走内层 padding，不靠浏览器边距。
- ✅ 题目块不跨页；选择题选项不拆行；图片不跨页。分页以「块」为最小单位，图片与选项本来就在块内。
- ✅ 页脚页码连续且总页数正确（正卷 / 答案页 / 答题卡共用一条编号）。P0 无封面，所以「封面不计页」这条暂时用不上。
- ❌ 双面打印时奇偶页页边距镜像。P0 只在**左侧**统一加 10 mm 装订区（`bindingLine` 时 `contentBox` 减 10 mm），没做奇偶镜像；双面装订的老师会看到反面装订边落在切口侧，P1 补。
- ✅ 教师版答案页**从新一页页首开始**（不接题尾），题号与卷面一致；上一页留白不回收。
- ✅ 答题卡默认 A4 双栏、涂卡区列数与题号对得上；关掉后不残留空白页（无块可排时不分页）。
- ✅ 公式无红色报错、无裸反斜杠；上下标、分式、根号、向量箭头正常（`throwOnError` + `latexToPlain` 兜底）。
- ✅ 灰度打印可读（水印 7% 透明度的浅灰斜排字，不压题干）。

---

## 9. 页面、接口与工程结构

### 9.1 页面

| 路由 | 文件 | 内容 |
|---|---|---|
| `#/papers` | `pages/Papers.jsx` | 列表：搜索、学科/状态筛选、卡片（卷名/题量/总分/更新时间/缩略图）、新建、复制、归档、删除 |
| `#/papers/new` | `pages/PaperWizard.jsx` | 组卷向导：基本信息 → 选题方式（手动/蓝图）→ 生成 |
| `#/papers/:id` | `pages/PaperEditor.jsx` | 三栏编辑（结构树 / 卷面预览 / 属性），换题、调分、排版 |
| `#/papers/:id/preview` | `pages/PaperPreview.jsx` | 全屏打印预览：学生版/教师版切换、缩放、打印、导出 |
| `#/papers/templates` | `pages/PaperTemplates.jsx` | 模板工作室：模板列表 + `<print-designer>` |

侧栏新增一级入口「试卷」（`hint: 组卷 · 排版 · 打印`），放在「题库」之后；`App.jsx` 的 `navActive` 与 `route.page` 增加 `papers` 分支（`#/papers/:id` 需支持二级路由解析）。

### 9.2 Tauri 命令（`commands.rs`）

```text
paper_list(query)                 -> Page<PaperSummary>
paper_get(id)                     -> PaperDetail（含大题、题目快照、版面）
paper_create(input)               -> PaperDetail        # input.scoreScheme 决定初始总分/大题结构
paper_update(id, patch)           -> PaperDetail        # 标题/版面/备注（不含总分）
paper_set_total(id, total, confirm=true) -> PaperDetail  # 改总分：必须显式确认，后端重算/校验
paper_apply_scheme(id, schemeId)  -> PaperDetail        # 套用标准卷分值方案（保留已有题则重排分值）
paper_score_check(id)             -> {total, assigned, delta, perSection[]}
paper_delete(id) / paper_restore(id)
paper_duplicate(id)               -> PaperDetail
paper_finalize(id) / paper_reopen(id) / paper_archive(id)  # finalize 强校验 sum(score)==total

paper_section_save(paperId, section)                      # 含 score_mode / unit_score
paper_section_delete(paperId, sectionId)
paper_section_reorder(paperId, ids)

paper_add_questions(paperId, sectionId, questionIds[])   # 自动快照
paper_remove_question(paperId, sectionId, ord)
paper_reorder_questions(paperId, sectionId, ords[])
paper_set_scores(paperId, items[{sectionId, ord, score}])
paper_refresh_snapshot(paperId, items[] | all)
paper_replace_question(paperId, sectionId, ord, questionId)  # 换题，返回可撤销 token
paper_undo_replace(paperId, token)

paper_compose_blueprint(blueprint) -> {paper, gaps[]}    # P1
paper_gap_report(blueprint)        -> gaps[]

paper_render_html(id, variant, opts) -> {html, pages, warnings[]}   # 前端也可直接渲染
paper_export_html(id, variant, path)
paper_export_pdf(id, variant, path)                       # P1
paper_print_log(id, variant, copies, channel)

paper_template_list(kind?) / paper_template_get(id)
paper_template_save(input) / paper_template_delete(id)
paper_template_builtin()          -> 首次启动写入内置模板
```

> 渲染优先放前端（`renderHtmlPages` / 自研排印器都在浏览器里测量），Rust 只负责取数据、存快照、导出文件。这样预览与打印共用一份实现，避免两端排版漂移。

### 9.3 前端目录

```text
src/lib/paper/
  model.js          # PaperDoc 类型、默认版面、分页参数
  compose.js        # 手动/蓝图抽题、缺口计算
  score.js          # 标准卷方案、统一分值重算、差额校验（总分固定）
  scheme.js         # 内置分值方案（XGK1_MATH / 通用 100 分 / 自定义）
  render.js         # 题目块 → HTML → 分页 → 组装
  print.js          # 打印通道（iframe / 系统浏览器 / 导出）
  fields.js         # PrintBusinessField 契约
src/components/print/
  PrintDesignerHost.jsx   # <print-designer> 的 React 封装（property 注入 / 事件桥）
  TemplateSlot.jsx        # 插槽选择与预览
  PrintPreviewFrame.jsx   # iframe 预览 + 缩放
src/pages/
  Papers.jsx  PaperWizard.jsx  PaperEditor.jsx  PaperPreview.jsx  PaperTemplates.jsx
src/styles/paper.css        # 卷面样式（同时用于预览与打印，独立于 app.css）
```

### 9.4 依赖隔离（换包保险）

所有 `@worm-vue3-print/*` 的引用只允许出现在 `src/components/print/` 与 `src/lib/paper/vendor.js` 两个位置：

```js
// src/lib/paper/vendor.js —— 唯一出口，换包只改这里
export { registerAll, createDefaultTemplate } from '@worm-vue3-print/common'
export { renderHtmlPages, browserCodeRenderer } from '@worm-vue3-print/core/browser'
```

理由：上游已迁移到 `@worm-print-platform/*`，未来换包 / 锁版本只动一个文件。

---

## 10. 实施计划

### P0 · 可用的试卷（约 8–10 人日）

| # | 任务 | 产出 | 验收 |
|---|---|---|---|
| 0 | 技术验证：WebView 打印、`<print-designer>` 在 React 19 挂载、KaTeX 字体内联 | spike 结论 | 三项各有明确结论与降级方案 |
| 1 | `0012_papers.sql` + 模型 / repo / commands + 迁移测试 | 题库可建卷 | `cargo test` 绿，旧库升级不丢数据 |
| 2 | 试卷列表 + 新建（含标准卷方案 choose）+ 基本信息 | `#/papers` | 增删改查、复制、归档可用；新建即带总分与大题结构 |
| 3 | 手动选题 + 大题结构 + 分值体系（统一分值 / 逐题微调 / 差额条 / 定稿校验） | 编辑器左栏 + 顶部 | 30 题增删排序无错乱；差额实时正确；差 1 分时定稿被拒并指明位置 |
| 4 | 卷面排印引擎（题目块 / 选项分栏 / 留白 / 分页 / 页码） | `render.js` | 30 题 A4 分页正确，无块跨页 |
| 5 | 打印预览 + 系统浏览器打印 + 导出 HTML | `PaperPreview` | 打出纸质卷，公式/图片/页码正确 |
| 6 | 学生版 / 教师版（答案页卷末强制新页） | variant 切换 | 答案页从页首开始，题尾留白也不接排；题号对齐 |
| 7 | 版面参数面板（纸型默认 A4 可改 / 边距 / 字号 / 栏数 / 装订线 / 水印） | 右栏 | 改参数即时反映，重开保持；切纸型分页正确 |
| 8 | 试卷入口 + 题库「加入试卷」 | 导航与批量操作 | 两处入口都能落卷 |

#### P0 实施记录（2026-11-18）

全部 9 项已落地，`cargo test` 144 项全绿（其中 `paper_*` 10 项）、`npm run build` 无警告。产物与方案对齐情况：

| 任务 | 实际产物 | 与规划的一致性 |
|---|---|---|
| 0 技术验证 | 结论：**排版全部在浏览器侧算**（隐藏宿主按真实字体量高 + 按栏贪心分页），产物是自包含单文件 HTML；KaTeX 的 CSS 运行时从 `document.styleSheets` 抽、字体由 `assetsInlineLimit` 内联，**没有引入 `@worm-vue3-print/*` 依赖** | 与 §8.2 一致；§9.4 的 `vendor.js` 单点出口留到 P1 接设计器时再建 |
| 1 数据层 | `migrations/0012_papers.sql`、`src/paper.rs`（~1300 行）、24 个 Tauri 命令（`permissions/local.toml` 全登记，`main.rs` 的 ACL 单测同时扫 `commands::` 与 `paper::`） | 表结构按 §5.1；`local_paper_question` 主键 `(paper_id, section_id, ord)` |
| 2 列表 / 新建 | `pages/Papers.jsx`、`components/paper/AddToPaper.jsx` | 状态页签（全部/草稿/定稿/归档/回收站）、复制、归档、恢复、软删；新建即带方案大题结构与总分 |
| 3 组卷与分值 | `pages/PaperEditor.jsx` 左栏、`src/lib/paper/score.js` | 统一分值改 `unit_score` 一键重算全大题；单题改分自动切 `PER_ITEM`；顶部差额胶囊实时算；定稿被拒时给「还差 N 分 / 可调整：X（n 题，现 m 分）」 |
| 4 排印引擎 | `src/lib/paper/render.js`、`src/lib/paper/model.js` | 题目块整体不跨页；**选项列数按栏宽自动收敛**（每列 ≥ 26 mm，4 个选项在窄栏里排 2×2 不留孤项，列数走内联 `grid-template-columns` 而非固定类名）；A4/A3/B5/8K/16K × 横竖；1/2 栏按「左栏排满再排右栏」；页脚页码跨正卷/答案页/答题卡连续 |
| 5 预览与打印 | `components/paper/PaperPreview.jsx`、`src/lib/paper/print.js` | 预览用的**就是**打印那份 HTML（`renderPaperHtml` 单一出口）；打印 = 写 `prints/*.html` + `open_path` 系统浏览器 ⌘P；另有导出 HTML；`local_paper_print` 记打印留痕 |
| 6 学生版 / 教师版 | `renderPaperHtml(doc, 'TEACHER')` | 答案页 `answerLayout` 强制单栏并**从新页页首开始**，上一页题尾留白不回收；答题卡可关，关掉不残留空白页 |
| 7 版面面板 | 右栏 | 纸型 / 方向 / 四边距 / 栏数 / 字号 / 行距 / 题距 / 选项列 / 答案留白 / 装订线 / 水印 / 答案页 / 答题卡；改动 350ms 防抖重排、600ms 防抖存库，重开保持 |
| 8 入口 | `App.jsx` 导航、`pages/Library.jsx` 批量条 | `#/papers`、`#/papers/{id}`、`#/papers/{id}/preview`；题库批量「加入试卷」可选目标卷与大题，已在卷内的题标「已在本卷」并禁选 |

三处**有意偏离**规划：

1. **没接 `<print-designer>`**（规划 §7 的模板工作室）：P0 的卷面版式由版面参数面板覆盖，封面/答题卡插槽的价值集中在 P1 的模板 CRUD 上，先接入会为了一个下拉框背上整个 Vue 设计器与 `plugin-kit` 依赖风险（R1）。
2. **PDF 导出推迟**（§8.3 的通道 B）：系统浏览器「打印 → 存储为 PDF」已经能拿到 PDF，Rust 侧 `createPDF` 属于重复投入，等 P1 与「静默打印」一起做。
3. **打印留痕只记不追溯版本**：`local_paper_print` 记录 `variant / copies / pages / channel / printer` 与时间，不回存生成的 HTML——版面参数在 `local_paper.layout_json` 里，同一份卷可原样重排，没必要把产物（几十 MB 的内联图）堆进库。

实施过程中修掉的 5 个真问题（都会直接印在纸上，所以记在这里）：

1. **选项占位符漏渲染**：`optionsHtml` 一开始用 `esc(option.text)`，卷面印出裸 `$\sqrt{1}$`；改走 `richHtml` 后才与题干同一套 KaTeX。
2. **单栏多推一个空列**：`nextColumn()` 无条件开新页，导致 `.paper-body.cols`（flex）在单栏下出现两个子元素，选项被挤成 61 px 竖排；改成「推完判断 `page.length >= columns` 才开新页」+ 末尾 flush 加空判断。
3. **测量宿主缺 CSS 变量**：隐藏宿主没带 `--q-gap` / `--q-line`，`box-sizing` 也只作用在 `.paper-doc *`，量出来的高度比真实排版矮一截，页码会串；两处都补齐。
4. **选项列数写死成类名**：`.cols-1/2/4` 三个类名一旦算出 3 列就没有 grid 模板，整排选项退化成逐行堆叠、还带出莫名其妙的横线；改成内联 `grid-template-columns`，并加「不留孤项」的列数选择（4 个选项优先 4 或 2 列）。
5. **`katex.min.css?inline` 重复打包字体**：包体从 855 kB 涨到 2.3 MB；改成运行时从 `document.styleSheets` 抽含 `.katex` 的规则，零重复。**副作用**：自检页面如果不 import `katex/dist/katex.min.css`，抽出来的 CSS 是空的，根号横线会拉成一整行长线——看着像排版 bug，其实是自检环境缺样式。

### P1 · 好用（约 8–12 人日）

#### P1 实施记录（2026-11-18：3 项完成、1 项部分、3 项明确不做）

| 条目 | 状态 | 落点 |
|---|---|---|
| 蓝图组卷 + 缺口报告 + 「按此蓝图再组一套」 | ✅ | `paper.rs` 的 `blueprint_validate / blueprint_apply / blueprint_fill / blueprint_again`；`components/paper/BlueprintModal.jsx`；列表「再组一套」。**先校验 `Σ(题数×分值) == 总分`（差 0.5 也拒），再确定性抽题**：`order by 使用次数 asc, created_at desc, id asc`；只取 `PUBLISHED`、卷内不重题；任一格缺题 → 整次回滚（`drop(tx)`）并逐条报缺口，前端给「放宽难度再组一次」。难度配额支持精确档（`difficulty`）与易/中/难（`bands`：1-2 / 3 / 4-5）两种写法。 |
| 换题候选 + 撤销 | ✅ | `replace_candidates / replace_question / undo_replace`；`components/paper/ReplaceDrawer.jsx`。候选 = 同学科 + 同题型 + 至少共享一个知识点 + 已入库 + 不在本卷（共用知识点为空时退回「同题型同科」，不返回空列表）；替换保留题号与分值，上一题（含 `snapshot_json`）记进 `flags_json.prev`，一步撤销后清空。 |
| 答题卡自动生成 | 🟡 | P0 已有 A4 双栏涂卡格 + 填空/解答分块；P1 补**单栏 / 双栏**切换。**A3 对折没做**：一份 HTML 只有一个 `@page` 尺寸（`@page` 命名页在 WKWebView 里不保证生效），混纸型会把其中一种打歪，宁可先不给这个选项。 |
| 打印记录 | ✅ | `local_paper_print` 记录 `variant/copies/pages/channel/printer`；`PaperSummary` 带 `print_count` / `last_printed_at`，列表卡片直接显示「打印 N 次（日期）」。 |
| 试卷缩略图 | ❌ | 列表要为每份卷都跑一次真实测量才能出图（一份 30 题卷要上百次 `getBoundingClientRect`），成本和收益不成比例；等 P2 做「批量打印」时顺手把首屏截图缓存下来再说。 |
| PDF 导出（macOS 优先） | ❌ | 系统浏览器「打印 → 存储为 PDF」已经能拿到 PDF；Rust 侧 `WKWebView.createPDF` 属于重复投入，等「静默打印」一起做。 |
| 模板工作室（`<print-designer>`） | ❌ | 见 §7 与 R1：要引入 `@worm-vue3-print/common@1.4.8` 与 Vue 运行时，只为一个封面/答题卡插槽不值；等「封面模板」真的成为需求再做。 |

**蓝图组卷的两个设计取舍**（都是照着「不静默凑数」这条原则推的）：

1. **`PER_ITEM` 大题也要求填一个预估分**：否则没法在抽题前把总分算平。抽完这档仍是 `PER_ITEM`，老师逐题改分即可——预估分只是让卷子一落地就是配平状态，不是最终分值。
2. **`replace` 与 `append` 两种模式**：编辑器里默认「清空现有题目再组」（老板子会被换掉），取消勾选就是「只补空缺」。`blueprint_again` 用 `replace=true` 且排除原卷用过的题，所以 B 卷不会和 A 卷撞题。

P1 验收：`cargo test` 152 passed / 2 ignored、0 警告（新增 5 项：蓝图分值不平拒、缺口整次回滚、难度档抽题 + B 卷不撞题、易中难配额、换题保分值 + 一步撤销）；`npm run build` 892 kB 无警告；`npm run app:build` 出 dmg 6.3M、`hdiutil verify` VALID（sha256 `dea9a4ce…52bf`）。界面目检：蓝图弹窗（两行大题、Σ = 100 · 与总分持平）、换题抽屉（3 条候选 + 换成这道）、试卷列表卡片（打印 3 次）。

- 蓝图组卷 + 缺口报告 + 「按蓝图再组一套」。
- 换题候选 + 撤销。
- 模板工作室：`<print-designer>` 接入、内置 4 套模板、封面/卷头/答题卡插槽、模板 CRUD。
- 答题卡自动生成：**默认 A4 纵向双栏**（选择题按题号 4 列涂卡网格，填空/解答按大题分块），可切 A3 对折 / 单栏 / 关闭。
- PDF 导出（macOS 优先）。
- 打印记录 + 试卷缩略图。

### P2 · 提效（探索）

- AI 组卷：把"本次考试范围 + 难度 + 题型分布"交给模型产出蓝图，人审后走确定性抽题（**模型只出蓝图，不直接出题**，避免不可控）。
- 批量打印 / 静默打印（接 `print-client`）。
- 发布为班级作业（写 `local_assignment` + `local_assignment_question`），批改后回填得分 → 题目得分率反哺组卷推荐。
- A/B 卷（选项与题目顺序变体）、听力材料模板。

> 每个阶段结束必须 `npm run build` + `cargo test` 全绿，并更新 `local-client/README.md` 的能力表与 `src/help/` 对应章节。

---

## 11. 风险与对策

| # | 风险 | 影响 | 对策 |
|---|---|---|---|
| R1 | `@worm-vue3-print/common@1.5.0` 依赖未发布的 `plugin-kit`，装不上 | 阻塞 | 锁 `1.4.8`；`package.json` 写精确版本；CI 加 `npm ci` 冒烟 |
| R2 | 上游仓库维护状态、迁移到 `@worm-print-platform/*` | 长期维护 | `vendor.js` 单点出口；模板 JSON 自行校验，不依赖其运行时校验 |
| R3 | Tauri WKWebView `window.print()` 无效 | 打印不可用 | 默认走系统浏览器打印；PDF 走 Rust 侧 `createPDF` |
| R4 | 分页测量受字体/图片未就绪影响 | 页码错、内容截断 | 测量前 `document.fonts.ready` + 图片 `decode()`；失败重试一次并提示 |
| R5 | KaTeX 红色报错进打印稿 | 卷面不可用 | 复用 `latexToPlain` 兜底；打印前扫描 `katex-error` 并阻止导出 |
| R6 | 大卷性能（80 题 + 200 张图） | 卡顿 | 编辑区虚拟化；渲染离屏一次性完成；图片懒解码但打印前全部预载 |
| R7 | 快照与题库不一致引起困惑 | 数据信任 | UI 明示「本卷使用 X 月 X 日快照」，提供差异对比与手动刷新 |
| R8 | 模板设计器被误用做整卷 | 返工 | 模板工作室只允许 COVER/HEADER/ANSWER_SHEET/ANSWER_KEY 四类，`FULL` 隐藏 |
| R9 | 许可与字体 | 合规 | 上游 MIT 可用；不打包商业字体，只用系统字体 |

---

## 12. 验收清单（P0 完成定义）

✅ = 已验（自动测试或目检）｜🟡 = 已实现、待人工点一遍

| # | 条目 | 状态 | 证据 |
|---|---|---|---|
| 1 | 旧数据库升级后自动建表，已有题目 / 作业 / 批改数据不受影响 | ✅ | `paper_migration_creates_tables_without_touching_questions`（`user_version` 升到 12、5 张表齐、迁移后仍能存题） |
| 2 | 能从题库挑 30 题组成一套卷，调分、排序、删题、复制 | 🟡 | 挑题 / 排序 / 删题 / 复制链路全通（`QuestionPicker`、`paper_add_questions`、`reorder_*`、`duplicate`），但一次 30 题的排印只跑到 8 题的样例卷 |
| 3 | 新建默认 A4；切成 A3 / B5 后分页正确，版面参数重开仍在 | ✅ | `create` 断言 `layout.paperSize == 'A4'`；20 题样例卷跑了纸型 × 栏数矩阵（A4 1/2 栏 3/3 页、A3 3/2 页、B5 4/3 页，页盒按 210×297 / 297×420 / 176×250 mm 换算无误），页内栏数与选项列数逐项核对；「重开仍在」见第 10 条 |
| 4 | 默认按大题统一分值，改 `unit_score` 后整题重算；逐题微调后差额条显示正确 | ✅ | `paper_uniform_score_rescores_every_question`、`paper_per_item_adjustment_switches_section_mode` |
| 5 | 总分差 0.5 分时点「定稿」被拒绝，并明确指出差在哪个大题；配平后才能定稿 | ✅ | `paper_finalize_rejects_unbalanced_scores_then_accepts`（判平阈值 0.001，报错带「还差 N 分」+ 可调整大题清单） |
| 6 | 卷面预览与学生版打印稿一致（同一份 HTML） | ✅ | 预览 iframe `srcDoc` 与打印文件都出自 `renderPaperHtml()` 单一出口 |
| 7 | A4 打印无跨页断题、无红色公式、页码连续 | ✅ | 6 页 A4 样例卷目检：题块整块不跨页、选项 4 列不折行、`katex-error` 有 `latexToPlain` 兜底、页脚「第 n 页 / 共 N 页」正卷与答案页连续 |
| 8 | 教师版答案页**从新一页页首开始**，上一页题尾留白也不接排；答案与题号一一对应 | ✅ | 教师版目检：答案页独立成页、页首即「参考答案与解析」，答题卡在其后 |
| 9 | 修改题库里的一道题，已定稿试卷内容不变；草稿卷有「同步最新」入口 | ✅ | `paper_snapshot_survives_question_edit_and_duplicate_copies_it`；「同步题库最新」走 `paper_refresh_snapshot` |
| 10 | 关掉应用重开，试卷、版面、模板插槽全部保持 | 🟡 | 版面在 `local_paper.layout_json`、插槽在 `slots_json`，编辑器 600ms 防抖落库；未做「关掉重开」的人工回归 |
| 11 | `npm run build` 无警告，`cargo test` 全绿，`npm run app:build` 能出安装包 | ✅ | `npm run build` 95 模块 / 855 kB JS 无警告；`cargo test` 144 passed / 0 failed（2 ignored 是既有探针）；`npm run app:build` 出 dmg 6.1M、`hdiutil verify` VALID（sha256 `f2be5b8b…7f37`，含排印修复后的重打） |

---

## 13. 产品决策记录

### 13.1 已确认（2026-11-17）

| # | 问题 | 决定 | 影响章节 |
|---|---|---|---|
| 1 | 默认纸型 | 可选；**默认 A4 纵向**，另提供 A3 / B5 / 8K / 16K / 自定义 | §6.4、§8.1 |
| 2 | 答案页位置 | **卷末**，且**必须新开一页**，不接在最后一道题题尾 | §6.4、§8.1、P0-6 |
| 3 | 答题卡 | **可选**，**默认 A4 双栏**；可切 A3 对折 / 单栏 / 关闭 | §6.4、§8.1、P1 |
| 4 | 分值口径 | **默认按标准卷统一分值，总分固定**；允许逐题调整，但必须配平才能定稿 | §5.1、§6.2、P0-3 |

### 13.2 仍待确认（不阻塞 P0）

1. **8K 具体尺寸**：国内常见 260×370 mm 与 270×390 mm 两种，学校油印设备不统一——先做成"8K（大）270×390 / 8K（小）260×370"两个预设，等实际打印反馈再收敛。
2. **答案页与答题卡是否同份输出**：当前默认"教师版 = 正文 + 答案页"，答题卡单独一份；若老师希望一次打齐（卷 + 卡），加一个"合并输出"开关即可，不改数据模型。
3. **分值是否允许 0.5 分**：当前 `real` 存储允许，UI 输入限制为 0.5 的整数倍；若学校只用整数分，可把输入步长改成 1。
4. **P2 的作业联动**：试卷定稿后是否必须发布成作业才能批改，还是允许"只打印不批改"（当前按后者设计，发布是可选动作）。

---

## 附录 A · 与中心系统（Java）方案对照

| 维度 | 中心系统 | 本地客户端（本方案） |
|---|---|---|
| 公式渲染 | JLaTeXMath + PDFBox 内嵌 | KaTeX 预渲染 HTML + 字体 base64 |
| PDF 生成 | 服务端 PDFBox | 浏览器打印 / WKWebView（P1） |
| 组卷 | 服务端模板表 + 配额 | 本地蓝图 + 确定性抽题 |
| 快照 | `paper_question` 行内快照 | `snapshot_json` 列 |
| 状态 | `DRAFT / FINAL / PUBLISHED` | `DRAFT / FINAL / ARCHIVED`（发布交给作业模块） |
| 版式 | 固定模板（HTML 拼接） | 结构化排印 + 可视化模板插槽 |

## 附录 B · 参考资料

- `worm-vue3-print/docs/中文/指南/模板设计器.md`（Props / 事件 / 模板模型）
- `worm-vue3-print/docs/中文/指南/渲染管线.md`（两遍渲染与分页）
- `worm-vue3-print/packages/print-common/src/host-contract.ts`（宿主契约，版本化）
- `worm-vue3-print/demo-common/apps/react/src/App.jsx`（React 19 宿主真实接入代码）
- `local-client/src/lib/Math.jsx`、`latexMath.js`、`latexPlain.js`（公式渲染唯一入口）
