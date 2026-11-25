# NeuralKB 本地题库 · 产品与实现说明

本文是面向维护者与开发者的产品文档：记录**已实现的全部功能**、与最初方案的差异、尚未实现的部分、隐私设计，以及人工验收清单。
普通用户请先看仓库根目录的 [README.md](../README.md)（安装与快速上手）。

---

## 已实现（P0–P4）

| 能力 | 位置 | 说明 |
|---|---|---|
| 题目 CRUD / 搜索 / 标签 / 知识点 / 难度 / 软删除 | `repo.rs`、`pages/Library.jsx` | 删除进回收站；同步过的题不允许硬删 |
| 题目包导入 JSON / JSONL / ZIP | `import.rs`、`pages/ImportPack.jsx` | 逐题校验、四种状态、批次撤销、ZIP 路径安全；重复 / 相似题默认不选，手动勾选后还需显式确认才导入；取消或替换预览会清理解包临时文件；题干 / 选项 / 解析里的图都会落 `assets/`，同 id 覆盖不重复 |
| 题目包导出 JSON / 带图 ZIP | `export.rs`、`pages/Library.jsx` | ZIP 含 `questions.json` + `images/`，能原样再导入（题干 / 选项 / 解析的图都改写）；JSON 只带文字，图留 `missingImages` 占位 |
| 截图识别 | `screen.rs`、`overlay.rs`、`jobs.rs`、`pages/Capture.jsx` | ⌘⇧O 置顶选区 + 标注工具条 → 识别 → 一图多题 → 右侧逐题核对、缺项就地改 → 存草稿；识别方式可选**自动 / 本机 OCR / AI 识图**；保存前整批校验原图，任一原图不可用则不写半批题目，附件异常会列出条目与题目 id，有题目未存时不自动翻页；可见工具条移植自桌面壳 |
| 本机 OCR（离线） | `ocr.rs`、`jobs.rs::capture_one_image` | macOS 自带 Vision（`objc2-vision`）读图，不需要模型 Key、断网可用；图片先做 EXIF 转正，HEIC / TIFF 走 `sips` 兜底；在本机按题号切题、拆选项、认「答案 / 解析 / 知识点 / 难度」，认不出题号时至少保留整段题干；自动模式下失败才回退视觉模型，并把原因写进题卡 |
| 内置浏览器取题 | `browser.rs`、`commands.rs::browser_collect`、`pages/Capture.jsx` | 试题网页**内嵌在录入页**（主窗口子 WebView，标签 `site`），不再另开窗口和工具条；`browser_bounds` 跟随占位框摆放，切页签时隐藏、回来保留登录态。页面注入**结构化抓取**：逐题「收这题」+ 浮动「识别本页 N 题」，只提取当前页可见题干 / 选项 / 答案 / 解析 / 知识点 / 难度（公式取 MathJax / KaTeX 源码），前后端均限制一次最多 60 题；可见图片只提示截图补图、不自动下载；来源由后端读取该视图真实地址，不信任网页上报。抓到的题落成 `capture_web` 草稿任务回主窗核对；找不到题块时整页文字只填入文本框，用户再点「整理成题目」才调用模型。第三方页面只能调 `browser_collect` / `emit_browser_text`（`capabilities/remote-browser.json` 用 `webviews: ["site"]` 精确授权，`permissions/browser.toml`），本机命令在 `permissions/local.toml` 里另列，`src/main.rs` 有单测盯着别漏 |
| 智能裁剪 | `smart.rs`、`commands.rs`、`pages/Capture.jsx` | 整卷图本机 Otsu + 行列投影自动找题块，默认全选、点框可取消，多块一次裁切入队；纯本地、不联网 |
| 图形框校对 | `components/FigureCropDialog.jsx`、`components/job.jsx`、`jobs.rs::extract_figures`、`llm/prompts.rs::capture` | 模型裁出来的图经常圈多了（带进相邻文字、题号、整页留白）。识别结果里每题带「图1 调整框选」：左边原图可拖框 / 拖四角，右边实时预览裁出来的样子，确认后按新框重裁（`smart_crop`）并换掉旧图；「＋ 从截图补一张图」可把漏掉的图补上（自动把 `![](图N)` 追加进题干）。`extract_figures` 现在把归一化 `box` 一起回给前端，提示词也改成「框要**紧贴图形本身**，文字和相邻题目不许圈进来」 |
| 本地文件取题 | `filein.rs`、`smart.rs::stitch`、`commands.rs::file_open` / `image_stitch`、`pages/Capture.jsx` | 打开 / 拖入本机图片、**PDF**、**Word**（.docx/.doc）与**纯文本**（.txt/.md 等）：PDF 用 CoreGraphics 逐页栅格化（1800px 长边、白底）；Word/txt 走文字链路（.docx 解 zip 内 document.xml，.doc/.rtf 用系统 textutil，纯文本按 UTF-8→GB18030 兜底），顶部三个页签选来源，下方左原文右结果两栏工作台，带页码条；**打开即自动找题块**（`smart_regions`），点一下某块高亮选中、点「识别这道」直接裁下并识别，`⌘` 点可多选批量入队；**跨页题按选区拼**：上一页框一段 + 下一页框一段竖拼（`image_stitch`，页间留白缝），不再整两页硬拼；**识别结果按页缓存**，翻页回来原样恢复。HEIC / TIFF / BMP 走系统 `sips` 兜底 |
| 采集台（来源 / 连拍 / 批量） | `capture_store.rs`、`jobs.rs`、`migrations/0003_capture.sql`、`pages/Capture.jsx` | `capture_source` 记来源与采集次数（置顶、忘记）；`capture_queue` 排队，`capture_batch` 一次识别整队、单张失败可重试、每题挂自己那张原图；`capture_text` 走纯文字取题，不需要视觉模型。单张「识别这张」也带 `queueId` 回写队列：待识别 → 识别中 → 已识别 N 道 / 失败原因，取景框角标直接显示这条状态（以前单张识别不更新队列，看着像没生效） |
| 题库按来源筛选 | `repo.rs::sources`、`pages/Library.jsx` | 采集来源自动汇总成下拉，按网站看题；`question_sources` 返回来源与题量 |
| 题库工具条与翻页 | `pages/Library.jsx`、`components/Pager.jsx`、`components/QuestionCard.jsx`、`lib/Math.jsx`、`repo.rs::tags` | 搜索 / 学科 / 题型 / 难度 / 来源 / 排序集中在一张卡片；**常用标签置顶**，标签框可模糊搜索（命中高亮）、可展开全部（`tag_list` 不再截前 80 个），点标签走精确 `tag` 过滤；「已筛选」胶囊可逐个去掉。列表头吸顶显示题量与页码并带动 `‹ ›`，底部分页条支持页码、跳页、每页 20/50/100，翻页自动回顶，删空当前页自动退到最后一页。卡片 / 公式 / KaTeX 输出三层 memo + 缓存，翻页不再重算整页公式；标签云与来源只在题库内容变化时刷新 |
| 试卷管理与打印 | `paper.rs`、`migrations/0012_papers.sql`、`pages/Papers.jsx`、`pages/PaperEditor.jsx`、`src/lib/paper/` | **试卷 = 大题（section）× 题**：套方案（新高考Ⅰ卷·数学 150 / 通用 100 / 自定义）建卷，从题库挑题时把题干、选项、答案、解析**冻结成快照**（`snapshot_json`），改题库不会偷偷改已排好的卷，需要时点「同步题库最新」。**总分是硬契约**：`sum(题分) == 总分`，大题默认统一分值、改单题自动切到「逐题分值」，定稿前 `score_check` 不过就给出差额与「该加/该减多少」的建议，绝不静默四舍五入。**排版在浏览器里算**：隐藏宿主按真实字体量每个题块的高度，再按栏贪心分页（1/2 栏、题干/选项/留白都参与计算），输出自包含单文件 HTML——KaTeX 的 CSS 与字体、图片（`nbasset://` 转 data URL）全内联，断网也能打印。版式可选 A4/A3/B5/8K/16K、横竖、边距、分栏、字号行距、选项列数、答案留白（auto/固定）、装订线、水印。**选项列数会按栏宽自动收敛**：每列至少留 26 mm，2 栏的 A4 里 4 个选项自动排成 2×2（不留孤零零的 D），列数走内联 `grid-template-columns`，1/2/3/4 列都成立。**教师版答案页强制新起一页**（不接在最后一题题尾），答题卡可选：**与试卷分开设计，试卷只绑定模板**；内置通用、高考数学 / 语文 / 英语 / 理综 / 文综模板，覆盖涂卡区、填空横线、解答题答题框，可复制成自定义模板改卷头、栏数、分区顺序、题型归属、涂卡列数与作答区高度。页码在正卷 / 答案页 / 答题卡之间连续编号。打印走「写 HTML 到 `prints/` → 系统浏览器 ⌘P」，另可导出 HTML / 打印留痕（`local_paper_print`，列表上直接显示「打印过几次、上次什么时候」）。**P1 已加**：①**蓝图组卷**——双向细目表（题数 × 每题分值 + 易/中/难配额 + 知识点配额），先校验 `Σ(题数×分值) == 总分` 再确定性抽题（只取已入库、卷内不重题、用过的少优先、新的优先），缺题则**整次回滚**并逐条报缺口，可一键「放宽难度再组」；蓝图存 `blueprint_json`，列表「再组一套」按同一蓝图抽一份**不撞题的 B 卷**。②**换题与撤销**——行尾 ⇄ 出候选（同学科 + 同题型 + 共用知识点 + 已入库 + 不在本卷），替换保留题号与分值，上一题连同旧快照记进 `flags_json`，行尾 ↩ 一步撤销。③答题卡模板独立维护并可被试卷绑定，模板自带卷头、栏数与分区规则（纸型跟主卷一致：一份 HTML 只能有一个 `@page` 尺寸，混纸型会把其中一种打歪）；预览按纸分页显示，页间留灰缝 |
| 模型池与用途路由 | `llm/mod.rs`、`pages/Settings.jsx` | 不内置模型；按用途路由（含课件生成 / 课件整理）；429 冷却；连通性测试 |
| AI 出题 / 批量归类 | `jobs.rs`、`pages/Generate.jsx` | 异步任务 + 轮询；题型按勾选的中文名写进提示词、生成后再校验纠正（要解答题不会给选择题）；缺答案 / 解析 / 知识点只标「待补」不挡存草稿，预览里可逐题补齐。批量归类**只列还没归类的草稿**，题干走公式渲染，处理过的不再出现 |
| AI 制图（场景渲染） | `diagram.rs`、`diagram_scene.rs`、`llm/prompts.rs::diagram`、`components/DiagramDialog.jsx`、`pages/DiagramTab.jsx`、`migrations/0015_diagram_purpose.sql` | 按题干画矢量示意图，产物是内嵌 SVG，确认后走 `asset_add` 存成题目配图。**模型不写 SVG 了，只给场景**：`{points, segments, faces, curves, circles, marks, labels}` 的 0–100 坐标结构，由 `diagram_scene` 自己渲染——标签位置按「背离图形中心 + 背离相邻棱」算出来再逐个避让，所以不会压线、不会重叠；模型给的数值标注（如 `∠BAD=60°`）也会避开点名，放不下就不画。**中点用 `midpoint` 声明**，坐标由本地求平均（模型心算 A₁D 中点时经常错成 A₁D₁）；`A1`/`A₁` 统一按同一名字去重、显示成下标，不再出现「A₁ A1」重影；重复线段合并、坐标越界夹住、引用了不存在的点整条丢掉、面缺顶点整面不画。提示词要求直棱柱底面 AB/CD 水平、侧棱竖直、只画题目用到的辅助线、默认不填色、被挡住的棱才用虚线。输出长度只有整张 SVG 的 1/6，`max_tokens` 截断基本不会再有；万一断在字符串中间，JSON 补齐逻辑照样能救回来 |
| 编辑题目 / 公式预览 | `components/QuestionEditor.jsx`、`components/DraftEditor.jsx`、`components/MathPreview.jsx`、`lib/latexMath.js::looksLikeMath` | 题干 / 选项 / 答案 / 解析仍存 LaTeX 源码（要能改、能原样回写），但输入框下面跟一块 **KaTeX 实时预览**：只在这段文字真的含公式时出现，`![](...)` 配图引用显示成「［图］」占位，不去解草稿阶段还没有的 `nbasset`。编辑几何 / 函数题时不用靠脑补 `\frac{\sqrt{3}}{3}` 长什么样 |
| 图片放大 | `components/ImageLightbox.jsx`、`src/styles/app.css`（`.img-zoom*`） | 点页面上任何一张正文图就进大图模式：滚轮以指针为中心缩放、拖动平移、双击 1×/2×、`1:1` 按像素比还原、Esc 关闭。做成 `document` 捕获阶段的全局监听，而不是逐个组件包 —— 图散在题面、选项、解析、复核面板、课件、答题预览里，改一处漏十处；判据是「`img` 且自然宽高 ≥64」，图标和头像不受影响。层级 220，高于弹窗，所以 AI 制图预览也能放大 |
| 题库统计 | `src-tauri/src/stats.rs`、`pages/Stats.jsx`、`commands.rs::question_stats` | 按学科 / 题型 / 难度 / 月度新增看结构，再加一段**补题建议**。统计全是本地 `group by`，口径与题库列表一致（软删除不算、草稿单列），不叫模型。建议按影响排序：零题学科（high）→ 某学科缺整类题型 / 难度挤在中档（mid）→ 没挂知识点、空章节、待复核、草稿、近 30 天无新增（low），每条都带「去出题 / 去整理」，跳过去时学科已经预选好（`#/library?subject=`、`#/generate?subject=`） |
| 制图显示 | `commands.rs::DiagramResult`、`diagram.rs::ensure_intrinsic_size` | `DiagramResult` 补 `#[serde(rename_all = "camelCase")]` —— 漏这一行后端吐 `data_url`、前端读 `dataUrl`，`<img src={undefined}>` 永远空白，SVG 生成成功但界面上什么都没有。另外 SVG 当 `<img>` 用时必须有固有宽高：只给 `viewBox` + `width="100%"` 浏览器算不出高度，所以新图直接写死 `width/height`，老图在 `asset_data` 读取时按 `viewBox` 补齐 |
| 帮助中心 | `src/help/*.md`、`pages/Help.jsx` | 纯中文、随包离线 |
| 本地账号与登录 | `auth.rs`、`migrations/0002_local_user.sql`、`pages/Profile.jsx`、`components/LoginGate.jsx` | 首次设账号，之后启动需登录；argon2id 哈希存本机，可改密/改资料；会话内存态 |
| 草稿箱与复核 | `repo.rs::review`、`jobs.rs::update_item`、`pages/Library.jsx` | **存草稿 ≠ 入库**：录入 / 生成 / 手工录题先落成 `DRAFT + needs_review`，进题库的「草稿箱」；复核通过才 `PUBLISHED`（正式入库），驳回退回草稿并写明原因，支持批量。题库拆成**四个互斥列表**：题库列表（已入库）/ 草稿箱 / 驳回列表 / 回收站，已驳回的不再混进题库列表。存草稿前可在预览里就地改题（`job_item_update`）；**草稿允许缺答案 / 解析 / 知识点**，只有「没认出题干」才勾不上；直接入库与复核统一要求三项齐全，单选答案只能对应一个真实选项，多选答案不能含不存在的选项，批量复核任一项不合格会整批回滚；结果列表有「全选 / 清空」批量勾选 |
| 统一归类语义 | `migrations/0010_merge_exam_points.sql`、`repo.rs`、`components/QuestionEditor.jsx`、`components/KnowledgePicker.jsx` | **取消「考察点」**：旧 EXAM_POINT 标签在迁移里归并进知识点（同名复用、不同名新建），界面只剩一个知识点选择器，最多 6 个；AI 出题 / 归类 / 导入 / 同步都只产出知识点 |
| 首次向导 | `pages/Wizard.jsx` | 5 步，可跳过模型 |
| 备份 | `commands.rs` | db + assets 打 ZIP；换目录自动复制并重启 |
| 数据库副本 | `commands.rs::db_export`、`pages/Settings.jsx` | `VACUUM INTO` 导出自包含 `.db` 快照，给 Navicat / DB Browser 打开看数据；库本身是 WAL 模式，直接开主文件常读到旧数据甚至打不开 |
| 本机存储面板 | `storage.rs`、`pages/Settings.jsx` | 设置页把 `assets/`（题目图）、`tmp/`（过程图）、数据库、备份、其他分开算，并给磁盘剩余与占比条；**只有临时文件 / 无主图片 / 旧备份可清**，最近 10 分钟的临时图受保护，题目引用的图没有删除入口 |
| 同步（左下角菜单里的可选项） | `sync.rs`、`pages/Sync.jsx` | 不占主导航，收进左下角账号菜单，带待同步数字；设备令牌（进钥匙串）、outbox 推送、游标拉取；服务端 `/api/desktop/**` 已实现（中心 `V98__desktop_sync.sql`）。中心新加的题会在拉取时落成本地题（`origin=CENTER`，用中心 id 当本地 id），本地有未推送改动时跳过覆盖 |
| 班级与学生 | `teaching.rs`、`migrations/0004_teaching.sql`、`pages/Classes.jsx` | 学生档案（**学号** / 年级 / 关注科目 / 备注 / 归档，软删除，不要求中心账号）；**学号**全局唯一（空号新建时自动 1001 起编号），导入按表头认列（`学号 / 姓名 / 年级 / 备注` 顺序随意），无表头时 `2026001,张三,七年级` 也能认；班级（邀请码 6 位可重置、成员增减、结课）；名单批量导入逗号 / 制表符 / 顿号都认，**先预览**、同名同年级与学号冲突当场标出 |
| 作业与批改 | `teaching.rs`、`compose.rs`、`review.rs`、`pages/Assignments.jsx`、`components/AssignmentComposer.jsx` | **按知识点 + 难度配额组卷**：勾知识点（多选，带题库可用量）→ 每档填题数（易 1–2 / 中 3 / 难 4–5）→ 生成预览后逐题「换一题」/ 上下移 / 改分值 / 移除，手选题兜底；抽不满如实报缺口不硬顶替；分值按题型画像预填（`app_setting compose.scoreProfile`）。草稿 → 发布 → 关闭；提交时**单选 / 多选自动判分**（大小写与顿号不影响），填空与解答题留给老师逐题打分与**批改备注（仅本机）**；**讲评清单**按题汇总错题人数、错误名单、得分率与备注，给「先讲这三道」排序，可打印 A4 / 导出 CSV |
| 教师课件（讲义式） | `src/courseware/`、`migrations/0006_courseware.sql`、`migrations/0014_courseware_import.sql`、`pages/Courseware.jsx`、`pages/CoursewareAi.jsx`、`pages/CoursewareImport.jsx` | **两条 AI 入口共用一个 `HandoutSpec` 内核**：①AI 生成课件——常规课 / 专题课变量、自定义提示词、参考素材与受管本地图片，可复制提示词并打开图片目录、调用「课件生成」模型或解析外部 JSON，手改提示词不被变量变化覆盖；②导入现有讲义——DOCX/DOCM、PPTX、PDF、Markdown、TXT、HTML 批量导入，默认只统一格式，原稿与位置可追溯，逐文件状态、判重、失败重试，AI 不可用仍有本地草稿。两条链路都先校对再入库，保留手动新建、AI 补题、题库挂接、五道闸、班级布置和旧 HTML 导入。可导出 HTML / DOCX / PPTX / PDF / JSON：Office 为真实 OOXML 且内嵌图片，PDF 用本机 Chromium，缺浏览器时降级系统打印，同名文件不覆盖；`assets/courseware/**` 受无主资源清理保护 |
| 中心照片（阶段 B） | `src/center_assets.rs`、`migrations/0011_center_submission.sql`、`pages/Assignments.jsx` | 学生在中心交作业时拍的手写原图，教师在本地点「中心照片」选一次对应的**中心班级 + 中心作业**（两边作业各自独立创建，不做自动猜测），照片按「绑过的中心账号 → 班级内唯一同名」匹配学生后下载落盘；同一张中心照片只拉一次（`center_asset_id` 去重），对不上号的学生列出来让老师手动绑（`student_bind_center`），绑完再拉一次即可；拉下来的与手机扫码传的是同一种整页照片，切题 / 给分 / 学情完全一致 |
| 拍照批改 | `src/grading.rs`、`migrations/0007_grading_photos.sql`、`pages/Grading.jsx` | **纸质卷子拍照批改**：整页照片导入 → 自动候选题块（编号蓝框）+ **手动拖框自选** → 挂到第几题 → 逐题给分；照片挂在既有 `attempt` 上，学情与错题本照常联动；学生没在线交卷时代建空提交（照片才有地方挂）；AI 只做**抄录建议**（逐字抄、给建议分与评语），**分数必须老师点「用这个建议」才生效**；「批改抄录」是独立的模型用途，只挑支持视觉的模型；作答照片受存储清理保护，不会被当孤儿图删 |
| 成绩（考试登分） | `scores.rs`、`migrations/0013_scores.sql`、`pages/Scores.jsx` | **整卷总分单独一张表**，与按题判分的作业分开：建考试（名称 / 班级 / 类型 / 满分 / 及格线 / 优秀线 / 日期）→ 按学号顺序登分（Enter 跳下一个人，空 = 缺考，与 0 分区分），也能**粘贴 / 导入 CSV 批量登分**（按学号自动名单匹配、逐行预览可跳过，只填草稿，点保存才生效）→ 实时平均 / 中位数 / 最高最低 / 及格率 / 优秀率 / 五档分数段（按满分百分比切，100 与 150 分制可直接比）/ 并列同名次 / 与班均差 → 导出 CSV。名次与统计只算有分数的人，缺考不拉低平均也不占位；改满分不动已录分数 |
| 学情与错题本 | `insights.rs`、`migrations/0005_insights.sql`、`pages/Insights.jsx`、`pages/StudentInsight.jsx` | **两个视角**：班级（知识点矩阵、共性错题 Top5、班级掌握度 / 得分率 / 待批改）与学生（跨班名单：掌握度、近两周涨跌、薄弱知识点、待复习、久未练，可按班筛 / 按姓名搜 / 四种排序）；点任意学生进**个人整页**（班内名次与班均对比、先补哪三个知识点、8 周趋势、知识点明细、错题本、最近作业）。**掌握度公式与中心逐行一致**（`weight=1/(1+days/14)`、`factor=correct?0.75+难度*0.05:0.05`、85 分掌握线），本地按需从 `local_attempt` 重算不落表；错题本自动入本（做对自动结案，NEW/REVIEWING/MASTERED 三档 + 1/3/7 天复习节奏）。路由 `#/insights`、`#/insights/student`、`#/insights/student/{id}`，返回时仍停在原来的视角上。**学情诊断**：规则版把薄弱点 / 到期错题 / 考试分数翻成一周安排（每天 10–25 分钟，不联网可用），点「AI 诊断」再让模型补「为什么卡在这里」「怎么跟家长说」；输出过结构校验，缺字段退回规则版，报告按数据指纹缓存 |
| 趋势与家长周报 | `progress.rs`、`src/lib/report.js`、`pages/StudentInsight.jsx` | **合并趋势**：作业（按题汇总的得分率）与考试（整卷总分）统一折算成**百分制**后按日期混排——100 分小测、150 分月考、10 分练习放在同一根轴上不失真；缺考不进时间轴（不画断点、不伪装 0 分），作业有题待批时单独标 `pending`，得分率偏低不会被当成退步；「最近三次 vs 之前三次」给一句涨跌结论。**家长周报**：个人页一键把学情压成一张 A4 —— 位置 KPI、一句话结论、优势 / 短板、掌握度走势与作业考试趋势（内联 SVG）、最近记录、本周安排、给家长的话，末尾留家长反馈与签字；自包含 HTML 交给系统浏览器，⌘P 打印或存 PDF，与试卷打印同一条链路。所有姓名 / 知识点 / 作业标题都做 HTML 转义，页内不含题目原文 |
| 听写台（P0） | `src-tauri/src/dictation.rs`、`pinyin.rs`、`ocr.rs`、`migrations/0017_dictation.sql`、`pages/Dictation.jsx`、`components/DictationReview.jsx` / `DictationPlayer.jsx` | 本机 OCR 识别今日听写清单（选图 / 拖入 / 粘贴）；本地拼音库词组优先 + 多音字候选**人工定音，未定音不许开播**；`speakAs` 同音字提示 + 逐条试听；系统离线语音（Web Speech）播报，**念完才开始 5s 计时**；自动 / 按钮 / ⌘⇧N / ⌘⇧R 推进，状态机单owner；播完保留当前批次**词条 + 拼音清单供人工核对**，不判分、不出报告、不写学情 |


---

## 与方案不一致的地方（有意的）

1. **选区层是移植而非抽共享 crate**：把 `desktop/` 的置顶选区 + 标注工具条整套搬进 `screen.rs` / `overlay.rs` / `public/overlay.*`，没有抽 `crates/nb-capture`——为了不动 desktop 已验收的热键链路。两套代码各自修 bug，但界面完全一致。
2. **提示词共享段落随仓库分发**：`shared/prompts/question.system.json.md` 与 `question-pack-skill.md` 用 `include_str!` 编译进来。识别 / 出题 / 归类三段是客户端专用（一图多题先切分、账号级归类），服务端 `PromptCatalog` 未改——两边行为不同，硬抽会把服务端改坏。共享原则写在 `shared/prompts/README.md`。（项目独立成仓时这段目录从原 monorepo 复制而来；若服务端提示词有更新，需要同步覆盖这里。）
3. **智能裁剪用本地投影而非视觉模型 bbox**：方案 §9.2 的 bbox 裁图走模型返回坐标；这里用 Otsu + 行列投影在本机找题块，默认全选、点框取消，多块依次裁成新图入队，不额外花模型额度。原截图仍整体存 `CAPTURE`，裁剪块作为新图走识别任务。
4. **PDF 自己栅格化，不引入第三方二进制**：`objc2-core-graphics` 直接调 `CGPDFDocument` + `CGContextDrawPDFPage`，按需渲当前页到 `tmp`，不打包 pdfium / mupdf 这类几十 MB 的依赖；长边 1800 是「模型看得清」与「data URL 不拖垮界面」的折中，细看走「放大预览」。
5. **同步分两条腿**：题库走 `V98__desktop_sync.sql` 的 push/pull（outbox 里只有 `question`，题单实体已端到端验证）；教学数据（班级 / 作业 / 作答）**不上传**，只有一条只读的 `/api/desktop/edu/**` 供本地拉学生作答照片——教师的班和作业本来就在本地建，缺少的只是「学生在家交的图怎么下来」。教学实体的双向 outbox 仍未做。
6. **「入库」指的是复核通过，不是保存**：录入与生成只负责把题变成草稿（`DRAFT + needs_review`），草稿箱里复核通过才转 `PUBLISHED`。这样 AI 与 OCR 的产物永远不会未经人眼就进正式题库；相应地，保存前的预览必须可编辑（`job_item_update`），否则「有问题的题只能丢弃」。**进草稿箱的门槛只设一道**：空题干（`blocking`）拦下，缺答案 / 解析 / 知识点只记 `issues` 当待补。手工直接入库与草稿复核共用正式门槛：三项必须齐全，选择题答案必须对应真实选项且单选只有一个答案。
7. **来源记忆与题库「来源」字段是一套值**：采集时把内置浏览器的真实地址写进 `question.source`，题库下拉直接按它分组；没有另建来源实体，避免同一网址两种写法。
8. **本机 OCR 用系统 Vision，不打包 PaddleOCR / Pix2Text**：`local-client-my` 的方案是 Python OCR + 公式模型，搬进 Tauri 要么带几百 MB 运行时，要么依赖用户装环境。这里改用 macOS 自带 `VNRecognizeTextRequest`：离线、零配置、安装包不膨胀；代价是公式图不如专用模型，所以保留「AI 识图」做公式与复杂版面，自动模式下本机失败会回退。
9. **网页取题走结构化 DOM，整页文字必须人工确认后才交给模型**：站点题块本来就是 HTML，直接抓可见题干 / 选项 / 答案 / 解析，比「整页文本 → 模型拆题」准得多，也不花额度。没有结构化题块时只把整页可见文字填进文本框，用户检查后再次点击「整理成题目」才走文字模型；不自动爬站、翻页、下载图片或绕过访问限制。


---

## 未做

教学实体的双向同步推送、中心课件动态清单、讲义模板库（按课型预置骨架）、GeoGebra 制图、基础修图、自动更新、Windows 包、SQLCipher、PPT 整卷导入（PDF / Word / txt 已支持）、公式图专用 OCR（本机 OCR 只负责文字，公式图建议切「AI 识图」）。（内置浏览器取题、本地智能裁剪、来源记忆与连拍队列、本地文件 / PDF 取题、班级 / 学生 / 作业批改、学情矩阵 / 错题本、教师课件（讲义式）已实现，见上表。）


---

## 安全与隐私

- API Key / 同步令牌只进系统钥匙串；SQLite 只存引用，所有返回值不带 Key。
- 钥匙串**一个进程只读一次**（`llm::KEY_CACHE`）：macOS 每次读都弹授权框，而批量归类是每道题一次调用，不缓存会连弹 N 次；换 Key / 删模型会同步失效缓存，不会留下旧值。`has_key` 的回填也只在首次启动做一次（`app_setting.llm.has_key_backfilled`），否则没配 Key 的本地模型会让每次启动都弹一次授权框。
- **Key 读到一次就落本地仓库**（`llm::vault_*`，`~/Library/Application Support/com.neuralkb.local/secrets/keys.json`，0600，异或遮罩 + 每机随机 seed）：登录钥匙串把「信任哪个程序」记在条目 ACL 里，自签名的 app 每次重打包 CDHash 都变，ACL 跟着失效，于是**每打包一次、点一次「识别」就弹一次授权框**，而点「允许」只对当次有效。现在一律先读本地仓库，仓库里没有才去钥匙串取（取到即写入），所以正常使用最多在第一次迁移时问一次；换 Key / 删模型两边一起清。仓库放在配置目录而不是数据目录，避免 Key 随数据目录被 iCloud 同步出去。
- 模型调用一律在 Rust 侧发起，WebView 拿不到 Authorization。
- CSP 允许 `'self'` 与 Tauri IPC，不加载外部脚本、字体或统计。
- 不收集使用日志（方案 D5）。断网时除 AI 外的功能全部可用。


---

## 手工验收清单

1、4、6、7 的数据链路（以及带图包的导出回导）已由 `cargo test` 里的 `flow_tests` 覆盖，改代码后跑一遍就知道有没有坏；
带 AI 与界面的几条仍需人工点一遍（没有模型 Key 就没法验）。

1. 冷启动 → 向导 5 步 → 进入题库。`flow_tests::cold_start_seeds_curriculum` 验了建库/迁移/教材快照。
2. 手工录一道带公式的题，列表和详情都能正确渲染 `$...$`。
3. ⌘⇧O 截一张多题试卷 → 识别 → 至少 8/10 题结构完整 → 入库 → 列表可见。
4. 导入 `shared/question-format/cases.json` 里「选择题带紧凑选项」那个包 → 预览 1 道可导入 → 入库 → 撤销整批 → 题目进回收站。
   `flow_tests::import_accepts_compact_options_then_undo_goes_to_trash` 全流程覆盖（含题干判重与恢复）。
   带图包的「导出 ZIP → 再导入」由 `flow_tests::zip_export_round_trips_images_and_knowledge` 覆盖（图、知识点、图注、覆盖语义都在里面）。
5. 设置里加一个不支持图片的模型 → 「录入」提示没有视觉模型，并指向设置。
6. 设置 → 导出整库备份 → 解压后含 `neuralkb.db` 与 `assets/`。
   `flow_tests::backup_zip_contains_db_and_assets` 验了包内容、跳过了 `backup/` 与 WAL 影子文件。
7. 断网：题库、搜索、导入、备份全部可用。`flow_tests::manual_question_is_searchable_offline` 验了 FTS 检索与 LaTeX 反斜杠保真。
8. 班级 → 批量导入 3 行名单（留一行重复）→ 预览标出重复 → 导入 2 人 → 建班 → 添加成员。
   `flow_tests::roster_import_builds_class_and_soft_delete_hides_students` 覆盖导入去重、邀请码字符集、重复加成员与软删除。
9. 建作业（单选 2 分 + 解答 3 分）→ 发布 → 给未交的学生「录入作答」（单选点 B、解答写过程）→ 客观题自动得分、主观题待批 → 批改给 2.5 分 → 成绩单 4.5/5。
   `flow_tests::assignment_submit_auto_grades_objective_and_teacher_finishes_subjective` 覆盖草稿不可交、分数越界、撤销重做。
10. 设置 → 本机存储：五类占用之和与 `du -sh` 对得上 → 清临时文件后 `tmp/` 变小、`assets/` 不变 →
    无主图片清理不动被题目引用的图。
    `storage::tests` 7 项覆盖保护窗口、assets 隔离、队列保护、题目 / 学生照片 / 课件资产保护、空目录回收、备份留 3 份与分类统计边界。
11. 打开一份 4 页 PDF：自动出现题块框 → 点第 3 块变实线 → 「识别这道」只认这一题 →
    翻到第 2 页认一遍再翻回来，第 1 页结果与勾选状态都还在。
    跨页：第 1 页框住题的下半段 → 「跨页拼接」→ 第 2 页框住其余部分 → 「接上并识别」。
12. 设置 → 「导出数据库副本」→ 用 Navicat / DB Browser 打开这个 `.db`，能看到题目表与 `user_version=14`（随迁移递增，以 `src-tauri/src/db.rs` 的 `MIGRATIONS` 条数为准）。
    `flow_tests::database_snapshot_opens_standalone_with_data_and_version` 覆盖快照可独立打开、数据与版本都在。
13. 班级作业交一份（一对一错一待批）→ 学情页矩阵出现该生一行、待批 1 不计入掌握度 →
    批完主观题数字变化 → 错题本 1 条 → 点「复习过」进复习中 → 重交做对后自动「已掌握」。
    `flow_tests::insights_matrix_and_wrong_book_follow_grading` 覆盖整条链路。
    再切到「学生」标签：名单里薄弱的排前面、没交作业的在最后；搜姓名能定位到人；
    点一行进个人整页，能看到班内名次、班均对比与「先补这几个」，返回后仍停在「学生」。
    `flow_tests::insights_roster_lists_students_and_ranks_inside_class` 覆盖名单排序、班均与名次、按班 + 关键字筛选。
14. 课件 → 手动新建：课题 + 知识点 + 1 条知识 + 2 道例题（一道用「挑题」从题库带出）→
    五道闸全绿 → 预览里题干解析已内联 → 布置到一个班。再打开「AI 生成课件」调整常规 / 专题变量、
    添加一张图并手改提示词，确认变量变化不覆盖手改内容；无模型时可复制提示词并打开图片目录、粘贴外部 JSON 校对。
    「导入现有讲义」多选文件，检查逐项状态、原稿对照、重复确认与失败重试；校对后导出 HTML / DOCX / PPTX / PDF / JSON。
    `courseware::prompt/source/office/ingest` 与 `flow_tests::courseware_import_without_ai_preserves_original_and_retries_independently`
    覆盖模板、抽取、Office 包、无 AI 降级、原稿保护、判重、重试和入库；真实五格式冒烟单独跑
    `cargo test export_all_formats_from_same_built_spec_with_image -- --ignored --nocapture`。
15. 批改台 → 拍照批改：给一个没在线交卷的学生建一份纸质作业 → 导入整页照片 → 「切题」里点编号蓝框
    （或直接拖框）→ 挂到第 1 题 → 点「AI 抄录这一张」看建议 → 「用这个建议」→ 保存批改。
    这道题在成绩单里能看到照片，打错分会进错题本，改判做对就结案，掌握度跟着变。
    `flow_tests::photo_grading_flow_cuts_pages_grades_and_updates_insights` 覆盖整条链路。
16. 中心照片（需两端都开着）：学生在中心作业详情里拍/选一张作答照片 → 本地点作业卡「中心照片」→
    选中心班级与对应作业 → 「绑定并拉取」→ 提示新增 1 张 → 「拍照批改」里能看到该生这张整页图。
    再点一次「拉取照片」应提示跳过（同一张不重复占空间）；把学生名改成中心里没有的名字再拉，
    会列出「没匹配到」并给出下拉，绑到本地学生后重拉即落地。
    `center_assets::tests` 5 项覆盖按中心账号/唯一同名匹配、同名不猜、未匹配上报、拉取去重与 `origin`。
17. 试卷 → 新建「新高考Ⅰ卷·数学」→ 左侧两大题默认各 5 分 / 0 分 → 从题库挑 6 + 2 题 →
    顶部差值胶囊提示「还差 N 分」→ 改大题统一分值一键重算 → 单题改分看到大题自动变「逐题」→
    分值不平点「定稿」被拦下并给出该加/该减多少 → 配平后定稿 → 预览切「教师版」，
    确认答案页是**新的一页**、答题卡在最后、页码连续 → 「打印」在系统浏览器 ⌘P 打出 A4 卷，
    公式、图片、页脚都在。改题库里某道原题再回编辑器，卷面不变；点「同步题库最新」才更新。
    `flow_tests::paper_*` 14 项覆盖建卷套方案、统一分值重算、定稿拦截与放行、逐题切换、定稿后只读、
    改总分需确认、快照不被题库改动影响且复制成新卷、归档后默认列表不再显示；P1 另有
    蓝图分值不平拒 / 缺口整次回滚 / 难度档抽题 + B 卷不撞题 / 易中难配额 / 换题保分值 + 一步撤销。
18. 编辑器点「按蓝图组卷」→ 单选 4 题 × 10 分 + 解答 2 题 × 30 分，易 1 中 2 难 1 →
    Σ 显示 100 分与总分持平才能点组卷 → 组完卷子自动配平；把某档题数调大再组，
    应弹出缺口红框且**卷子没变**；点「放宽难度再组一次」能凑满。
    再回列表点「再组一套」，新卷的题与原来完全不同（不撞题）。
    题目行尾 ⇄ 换一道 → 抽屉里点「换成这道」→ 题号分值不变 → 行尾 ↩ 撤销回原题。

19. 成绩 → 新建「第一次月考」（满分 100，及格 60，优秀 90）→ 按学号登分：甲 92、乙 88、丙空着
    → 保存后平均 90（丙不算）、优秀率 50%、甲第 1 乙第 2、丙无名次、90+ 段 1 人
    → 导出 CSV 用 Excel 打开不乱码 → 个人学情「考试成绩」出现这一条并显示「第 1/2 · 班均 90」。
    `flow_tests::exam_scores_rank_and_stats_exclude_absentees` 覆盖缺考不进平均、并列名次、
    及格 / 优秀线边界、分数段与超分拦截。
20. 学生 → 新建学生时不填学号 → 自动得到 1001；批量导入粘贴带表头的「学号,姓名,年级」，
    预览里空号显示「自动」、重号显示「学号冲突」，导入后学号与名单一致；手改学号撞到别人会被拒。
    `flow_tests::student_numbers_are_imported_auto_assigned_and_guarded` 覆盖表头识别、
    自动编号、批内重号拦截与老格式兼容。
21. 个人学情 → 顶部「学情诊断与本周安排」：没配模型时应直接给出规则版周计划（7 天、指向最薄弱的知识点、
    给出依据）；点「AI 诊断」配了模型时补「为什么」和「给家长的一句话」，模型报错/断网则退回规则版且不弹错。
    `flow_tests::advice_falls_back_to_rules_and_targets_weakest_point` 覆盖规则版结构、指向薄弱点与无模型兜底。
22. 个人学情 → 「作业与考试」卡：交两次作业（60%、80%）再录一次 150 分制考试（120 分）→
    三个点按日期排开、考试点显示 80%、班均按百分制、缺考的同学不占位；四次之后出现「最近三次比之前高 / 低 N 个百分点」。
    点页头「家长周报」→ 浏览器打开一张 A4，两张趋势图、最近记录、本周安排都在，⌘P 预览应为**一页**。
    `flow_tests::progress_mixes_assignments_and_exams_on_percent_scale` 覆盖百分制换算、正序混排、缺考跳过与涨跌方向。
23. 作业 → 「布置作业」：勾两个知识点、易 3 / 中 4 / 难 2 → 「生成预览」→ 题量与分档对得上，
    分值按题型带出（选择 3 / 填空 4 / 解答 10）；点某题「换一题」换掉且不重复、上下移改顺序；
    故意把某档要成 20 题 → 提示「只有 N 道」且预览只出 N 道（不拿别的难度顶替）。
    `compose::tests` 覆盖分档配额、缺口上报、知识点过滤、题型过滤与换题不重复。
24. 批完一份作业点「讲评清单」→ 每题列出错的人数 / 得分率 / 错误名单 / 批改备注，
    顶部「先讲这三道」按错人数排序；「打印 / 存 PDF」出 A4 一页，「导出 CSV」一行一题。
    `review::tests` 覆盖按题汇总、最近一次提交、半分算错但不空白、讲评优先级。

