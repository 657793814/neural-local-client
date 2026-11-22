# 手机拍照 → 本地批改联动规划（阶段 2 深化）

> 状态：**阶段 A / B 均已实施（2026-11-17）**｜阶段 A′（Mac 相机）仍为规划
>
> 阶段 A 落地记录见 §9（`photo_upload_server.rs` + 3 条命令 + `Grading.jsx` 手机上传弹窗）；
> 阶段 B 落地记录见 §10（中心 `V99__submission_asset.sql` + `/api/desktop/edu/**` + 本地 `center_assets.rs`）。
>
> 关联：[本地优先教学能力规划](local-first-teaching-plan.md) §7（拍照批改，本文是其 §7.1 第二条入口的深化）、
> [本地题库客户端规划](local-question-bank-client-plan.md) §18（拍照批改已实现）、
> [主观题批改与班级作业](../../neural-knowledge-base/docs/product/subjective-grading-and-class-assignments.md)

## 1. 先说结论

现在 `local-client` 的批改台已经能：**导入整页照片 → 自动/手动切题块 → 挂到学生 → 逐题给分 → 学情联动**（`grading.rs`、`pages/Grading.jsx`，见客户端规划 §18 功能表）。缺的只是「照片怎么从手机到这台 Mac」。

规划目标：让老师**在手机上拍学生作业，照片直接进入本地批改台**，尽量少手动搬运。推荐方案是 **局域网扫码上传（阶段 A）+ 中心中转（阶段 B，可选）** 两条腿：

- **阶段 A（本地、断网可用、零服务器）**：Mac 起一个只绑局域网、带一次性令牌的临时 HTTP 服务，手机扫码后选图/拍照上传，照片走现有 `grading_photo_add` 落 `assets/submissions/<学生>/`，批改台刷新即可见。
- **阶段 B（可中心端学生在家用）**：中心新增 `submission_asset` 与上传接口，本地 `sync_pull` 拉图片——这条就是总规划 §7.1 第二条入口，依赖中心端，单独排期。

两条腿共用同一个落盘与切题链路，阶段 A 不依赖阶段 B，可先落地。

---

## 2. 现状盘点（以代码为准，2026-11-17）

| 环节 | 现状 | 代码位置 |
|---|---|---|
| 照片落盘 | `store_page()` 收 data URL，落 `assets/submissions/<学生 id>/<uuid>.<ext>`，登记 `local_attempt_asset(kind='PAGE')` | `grading.rs::store_page` |
| 现有导入入口 | 批改台「导入照片」→ Tauri 文件选择框（多选）→ `fileOpen()` 读成 data URL → `gradingPhotoAdd()` | `pages/Grading.jsx:410-441` |
| 切题 | 整页 `detect_regions()` 找候选块 + 手工拖框 → `crop_from_page()` 出题级图 | `grading.rs::detect_regions/crop_from_page` |
| 给分 | 逐题 / 批量给分，写 `attempt`，学情重算 | `teaching.rs`、`insights.rs` |
| AI 抄录 | `grading_transcribe` 视觉模型给建议，分数必须老师确认 | `grading.rs::transcribe_with` |
| 手机入口 | **无** | — |

关键约束（来自 `AGENTS.md` / 客户端规划）：

1. **数据只在本机**：本地客户端不共享中心数据库，离线优先，不收集日志。任何传输方案都不能把学生作业照片悄悄传到第三方。
2. **不建平行批改系统**：照片最终必须落到现有 `local_attempt_asset`，复用同一套切题/给分/学情，不能另建一套「手机作业」表和批改界面。
3. **前端只访问同源 `/api/**`、不加载外部脚本**（中心前端规矩）；本地客户端 CSP 也只允许 `'self'` + Tauri IPC。这决定了**手机上传页不能塞进主 WebView**，要用独立通道（见 §3）。
4. **AI 只给建议，分数老师定**：抄录建议不能自动生效，照片才是评分依据。

---

## 3. 手机 → Mac 的传输方案取舍

| 方案 | 断网可用 | 用户体验 | 实现成本 | 合规/风险 | 结论 |
|---|---|---|---|---|---|
| **A1 局域网扫码上传**（手机浏览器打开临时页选图/拍照） | ✅ | 扫一次码，选多张传 | 中：Rust 起本地 HTTP + 一个静态上传页 | 令牌+限时+仅局域网绑定，照片不出内网 | **推荐（阶段 A）** |
| A2 系统隔空投送 / AirDrop | ✅ | 系统原生 | 低 | 无法程序化接收、无法自动落到批改台，只能手动另存再导入 | 不采用（可作兜底说明） |
| A3 手机连数据线（文件 App） | ✅ | 传线麻烦 | 低 | 同上，手动 | 不采用 |
| A4 local-client 自带相机插件（`photo_capture`） | ✅ | 老师在 Mac 上拍 | 中 | 只解决「Mac 端拍照」，不解决「学生在家手机拍」 | 与总规划 §7.1 第一条合并，另做 |
| **B 中心端学生拍照上传 + `sync_pull` 拉取** | ❌（需联网） | 学生在家就能交 | 高：中心建表+接口+同步 | 走中心既有鉴权，数据进中心库 | **推荐（阶段 B，独立排期）** |
| C 第三方图床 / 微信 / 邮件中转 | ✅/❌ | 顺手 | 低 | **照片出内网，违反「数据只在本机」** | 明确不做 |

推荐 A1 作为阶段 A 的核心：它是唯一同时满足「断网可用 + 自动落到批改台 + 数据不出内网」的方案。A2/A3 只作为「不想开服务时的手动兜底」写进帮助文档。

---

## 4. 阶段 A：局域网扫码上传（详细设计）

### 4.1 端到端流程

```
老师：批改台选学生/作业 → 点「手机上传」 → Mac 生成一次性令牌 + 起临时 HTTP 服务（只绑局域网 IP）
   → 界面显示二维码（含 http://<局域网IP>:<端口>/u/<令牌>?student=<id>&assignment=<id>）
学生/老师：手机扫码 → 打开轻量上传页 → 选图或直接调起相机拍照 → 上传一张/多张
Mac：校验令牌 → 收图 → 走 grading::store_page() 落盘登记 → 批改台轮询/监听刷新照片列表
老师：点「切题」 → 自动/手动框 → 挂题 → 给分 → 学情联动
结束：老师点「停止上传」或令牌过期/用满 → 服务关闭 → 二维码失效
```

### 4.2 本地 HTTP 服务

- 用轻量 Rust HTTP（如 `tiny_http` 或 `axum` 的最小配置），**只绑局域网网卡 IP**（不绑 `0.0.0.0` 暴露公网），端口随机高位或可配置；开机不常驻，**仅老师点「手机上传」时启动**，停止即关。
- 服务只暴露 3 个端点：
  - `GET /u/<token>` → 返回上传页 HTML（内联，无外部资源，符合 CSP 精神）
  - `POST /u/<token>` → multipart 收图，`Content-Length` 限 12MB/张、总数限 30 张
  - `GET /u/<token>/status`（可选）→ 返回已收张数，便于上传页反馈
- 令牌：随机 24 字节 base64url，**一次性会话**（一次开启可传多张，老师关闭即失效），**有效期 15 分钟**，绑定本次的 `student_id` + 可选 `assignment_id`。令牌只存内存，不落库、不进日志。
- 上传页是**独立浏览器页面**，不是 Tauri WebView：手机自带浏览器即可，不需要装 App、不碰本地 CSP。

### 4.3 与现有链路对接（关键：不建平行系统）

- 服务端收到图后**不自己写库**，而是把图片字节转成 data URL，调用与前端 `gradingPhotoAdd` 相同的后端函数 `grading.rs::store_page()`（入参 `PhotoInput { studentId, assignmentId, dataUrl }`）。保证落盘路径、`sha256` 去重、`local_attempt_asset` 登记与现有的「导入照片」完全一致。
- 上传完成后通过 Tauri 事件（`app.emit("grading-photo-arrived", …)`）通知批改台刷新；前端 `Grading.jsx` 监听事件或短轮询 `gradingPhotoList` 更新照片墙。
- 照片一进来就是 `kind='PAGE'` 的整页图，后续切题/给分/学情全部复用现状，零改动。

### 4.4 客户端改动清单（预估）

| 层 | 改动 |
|---|---|
| Rust 依赖 | 加轻量 HTTP + multipart 解析 + 二维码生成（或前端生成）crate |
| 新模块 | `photo_upload_server.rs`：起/停服务、令牌管理、上传页 HTML、收图转 data URL 调 `store_page` |
| `Cargo.toml` | 新增依赖（ Mesh: `tiny_http` / `axum`、`qrcode` 任选，实施时定，见 §7 待定） |
| 命令 | `photo_upload_start(studentId, assignmentId?) -> {url, qrDataUrl}`、`photo_upload_stop()`、`photo_upload_status()` |
| 前端 API | `api.js` 加 3 个封装 |
| `Grading.jsx` | 「导入照片」旁加「手机上传」按钮 + 二维码弹窗 + 收图刷新 + 停止控制 |
| 样式 | 二维码弹窗、上传中状态（复用现有 `Modal` / `notify`） |
| CAPABILITY | 新增的本地 IPC 命令要登记到 `permissions/local.toml`；上传服务是本机进程，不走 WebView 权限 |

### 4.5 安全与隐私

- 令牌一次性 + 15 分钟过期 + 仅局域网绑定 + 关闭即失效；不落库、不进日志。
- 手机上传页无外部脚本/字体/统计，纯静态内联。
- 图片只写到本地 `assets/submissions/`，不上传任何第三方；断网全程可用。
- `content_type` / 扩展名用 `text::sniff_image` 从字节嗅探，不信文件名（与 `store_page` 现状一致）。
- 令牌里带 `studentId`，但上传页不显示学生隐私信息（只显示「上传作答照片」），二维码被旁人扫到也只知道一个临时上传入口。

---

## 5. 阶段 B：中心端学生拍照提交（可选，独立排期）

总规划 §7.1 第二条入口的落地，依赖中心端。**实施时改了两处设计**（原因见括号）：

1. 中心建 `submission_asset` 表（学生作答图片）。上传走 `POST /api/edu/assignments/{id}/assets`（body 是 `{dataUrl}`），**不从 `submit` 里传 `assetIds[]`**——照片与文字答案互不依赖，先拍后答、先答后补拍都成立；绑在 submit 上会逼出「没交答案就不能传图」和「交完就补不了」两种别扭状态。
2. 中心给客户端加一组只读接口 `/api/desktop/edu/**`：班级、作业、全班照片、单张图。**没走 `sync_pull`**——push/pull 是题库的双向幂等合并，教学数据是「教师读自己班级」，语义不同，硬塞进 outbox 会多出一堆没人写的实体类型。
3. 本地 `center_assets.rs` 拉图：中心账号 → 本地学生按「已绑定的 `center_user_id` → 班级内唯一同名」匹配，命中后落 `assets/submissions/<本地学生 id>/center-<中心附件 id>.<ext>`，登记 `local_attempt_asset(kind='PAGE', origin='CENTER', center_asset_id=…)`；`center_asset_id` 唯一索引保证重复拉取不翻倍。
4. 对不上号的学生不猜：以 `UnmatchedStudent` 返回，教师在弹窗里选一个本地学生绑上去（`student_bind_center`），重拉即可。
5. 复用同一套切题/给分/学情——照片在批改台里与阶段 A 的完全一样。

这条让学生「在家就能交作业」，需要联网与中心端配合，不阻塞阶段 A 落地。

---

## 6. 分期与验收

| 期 | 范围 | 依赖 | 验收 |
|---|---|---|---|
| **A（本地扫码，先做）** | §4 全部：临时服务 + 二维码 + 上传页 + 对接 `store_page` + 批改台刷新 | 无（纯本地） | 手机与 Mac 同一 WiFi：批改台选学生 → 扫码 → 手机传 3 张 → 批改台看到 3 张整页图 → 切题给分 → 学情更新；关服务后二维码失效；全程断网可用；照片只在本机 |
| **A′（Mac 端拍照，可选）** | `photo_capture` 相机入口（总规划 §7.1 第一条） | A | 无相机时降级为选图/粘贴；连续拍照进同一学生 |
| **B（中心中转，已完成）** | §5 全部 | 中心端 `/api/edu` + `/api/desktop/edu`（设备令牌） | 学生在家手机经中心上传 → 教师本地绑定中心作业 → 拉取 → 批改台可见；断网本地功能不受影响 |

阶段 A 验收片段（可写进 `flow_tests`）：起服务 → 用 HTTP client 带令牌 POST 一张测试图 → 断言 `local_attempt_asset` 多一条 `kind='PAGE'`、文件落 `assets/submissions/<学生>/`、sha256 正确；令牌过期/错误令牌 POST 返回 403。

---

## 7. 实施决策（阶段 A 已定）

1. **HTTP 栈**：**都不用**——纯标准库 `TcpListener` 手写最小 HTTP。上传页把图片读成 data URL 以 JSON POST，天然绕开 multipart 解析，零新增网络依赖，包体更小、更贴合离线客户端。
2. **二维码生成**：Rust `qrcode` crate（0.14，仅 `svg` feature）生成 SVG 字符串，前端用 `data:image/svg+xml` 显示，不注入 HTML。
3. **局域网 IP**：UDP `connect("8.8.8.8:80")` 取默认路由网卡 IP（不实际发包，断网也能拿到）。拿不到时提示降级「用数据线 / 隔空投送」。不绑 `0.0.0.0`。
4. **相机入口**（A′，未实施）：阶段 A 靠手机原生 file input 的拍照/相册选择，不需要 Mac 端相机插件。A′ 再议。
5. **令牌有效期/张数上限**：沿用初值 15 分钟 / 30 张，写死在 `photo_upload_server.rs`，实测再调。

---

## 8. 明确不做

- 不做第三方图床/微信/邮件中转（照片出内网，违反「数据只在本机」）。
- 不为手机端做本地账号登录（本地客户端仍是单用户，学生不是本地账号，见总规划硬原则 3）。
- 不让 AI 抄录建议自动给分（分数必须老师确认，见硬原则 4）。
- 阶段 A 不做公网访问、不做常驻服务、不做学生自主查询——老师是唯一操作者。

---

## 9. 阶段 A 实施记录（2026-11-17）

### 9.1 改了什么

| 层 | 文件 | 改动 |
|---|---|---|
| Rust 依赖 | `src-tauri/Cargo.toml` | 新增 `qrcode = { version = "0.14", features = ["svg"] }`（只用来生成二维码 SVG）；HTTP 用标准库，未引第三方 server/multipart crate |
| 新模块 | `src-tauri/src/photo_upload_server.rs` | 临时 HTTP 服务、令牌管理、内联上传页、收图转 data URL 调 `grading::store_page`、事件通知；含 5 个单测（令牌、data URL 校验、上传页自包含、端到端 HTTP 收图落盘、停止失效） |
| 命令 | `src-tauri/src/commands.rs` | 新增 `photo_upload_start` / `photo_upload_stop` / `photo_upload_status` |
| 注册 | `src-tauri/src/main.rs`、`permissions/local.toml` | 模块 + 3 条命令登记（ACL 自检测试通过） |
| 前端 API | `src/lib/api.js` | 3 个命令封装 |
| 前端 UI | `src/pages/Grading.jsx` | 照片墙加「手机上传」按钮；`PhoneUploadModal` 弹窗显示二维码/地址/已收张数/剩余时间，2 秒轮询 + 事件监听自动刷新照片墙，离开页面自动停服务 |
| 样式 | `src/styles/app.css` | 弹窗布局（桌面并排、窄屏堆叠） |

### 9.2 实际链路（与 §4 设计一致）

1. 老师打开某学生的批改台 → 点「手机上传」→ `photo_upload_start(studentId, assignmentId)`。
2. 后端取默认路由网卡 IP，绑随机端口，生成 24 字节令牌，返回 URL + 二维码 SVG；服务仅在内存持有会话，15 分钟过期。
3. 手机扫码打开内联上传页（CSP 严格、无外部资源），选图/拍照后读成 data URL 以 JSON POST 到 `/u/<token>`。
4. 后端校验令牌与过期 → 转 `PhotoInput` 交 `grading::store_page()` → 落 `assets/submissions/<学生>/`、登记 `local_attempt_asset(kind='PAGE')`，与「导入照片」完全同源。
5. 后端发 `nb://grading-photo-arrived` 事件 → 批改台照片墙自动刷新；上限 30 张、单张 body ≤ 12MB。
6. 老师点「停止上传」或 15 分钟到期或离开批改台 → 服务关闭、令牌立即失效。

### 9.3 验收

- 单测：错误令牌 403、非图片 400、合法图 200 且落盘登记（`local_attempt_asset` 多一条 PAGE、文件存在、路径与学生绑定）、停止后会话失效；共 5 项通过。
- 全量 `cargo test`：124 passed / 2 ignored，0 警告。
- `npm run build`：通过，0 警告。
- `npm run app:build`：完整打包 + 签名 + dmg 校验，0 警告；dmg 5.9M，`hdiutil verify` VALID，SHA-256 `f9cf7cd7375e74b5bfb417eec4440324d1a637734f63697b288497783487faca`。

### 9.4 仍未做

- **阶段 A′**：Mac 端相机入口（见 §6），未实施。
- **阶段 B 已完成**（见 §10）；仍缺的是「教师把本地作业推成中心作业」（教学实体 outbox），目前需要中心侧已有一份同名/对应的作业由学生提交。
- 真实双设备（手机 + Mac 同 WiFi）端到端需人工实测；单测已覆盖服务端收图落盘与鉴权，但无法验证扫码/手机浏览器兼容性。建议首次使用前用一台手机走一遍 §6 验收步骤。

---

## 10. 阶段 B 实施记录（2026-11-17）

### 10.1 中心端（`server/`）

| 层 | 文件 | 改动 |
|---|---|---|
| 迁移 | `V99__submission_asset.sql` | `submission_asset`（assignment_id / user_id / ord / storage_key / content_type / byte_size / sha256 / created_at / deleted_at）+ 作业与学生的两个索引。不记 `submission_id`：照片是「这份作业的作答原件」，与提交批次无关 |
| 实体/仓库 | `edu/SubmissionAssetEntity.java`、`SubmissionAssetRepository.java` | 与 `question_asset` 同一套写法：图片在 MinIO，行里只存 key、大小、指纹 |
| 服务 | `edu/SubmissionAssetService.java` | 学生上传（限 12MB、按文件头嗅探 jpg/png/webp/gif、同图按 sha256 去重、只能给自己班已发布作业传）、教师看全班、按权限读图、学生/教师撤回（软删，MinIO 对象保留） |
| Web 接口 | `edu/SubmissionAssetController.java` | `POST/GET /api/edu/assignments/{id}/assets`、`GET /api/edu/assets/{id}/data`、`DELETE /api/edu/assets/{id}` |
| 客户端接口 | `desktop/DesktopEduController.java` | `GET /api/desktop/edu/classes`、`/classes/{id}/assignments`、`/assignments/{id}/assets`、`/assets/{id}/data`；身份直接取设备令牌签发的教师，复用 `ClassService` 的班级鉴权 |
| 单测 | `SubmissionAssetServiceTest` | 9 项：类型嗅探、落库、同图幂等、非成员拒绝、未发布拒绝、非图片/超大拒绝、非任课教师不能看全班、读他人照片拒绝、名单带用户名 |

### 10.2 中心前端（`front-src/`）

- `neural/LearningFlow.jsx`：学生作业详情新增「作答照片」区块——`拍照 / 选图`（`accept="image/*" capture="environment" multiple`）、缩略图、撤回；教师视图同区块按学生显示，只读。
- 上传前在浏览器里缩到长边 2200px / JPEG 0.85（手机直出 4–8MB → 通常 <1MB），`createImageBitmap` 不可用时退回原图。
- `api/client.js`：`eduAssignmentAssets` / `eduUploadAssignmentAsset` / `eduDeleteAssignmentAsset` / `eduAssetUrl`。

### 10.3 本地客户端

| 层 | 文件 | 改动 |
|---|---|---|
| 迁移 | `0011_center_submission.sql` | `local_assignment.center_assignment_id` + `center_assignment_title`；`local_attempt_asset.center_asset_id` + `origin`；中心照片唯一索引（部分索引，本地照片不受影响） |
| 落库 | `grading.rs::store_external_page` | 字节已在内存的整页照片落盘，带 `center_asset_id`、`origin='CENTER'`；已拉过返回 `Ok(None)` |
| 拉取 | `center_assets.rs` | 列举中心班级/作业、绑定、拉取；学生匹配「先 `center_user_id`，再班级内唯一同名」，同名的两个不猜；未匹配以 `UnmatchedStudent` 上报 |
| 命令 | `commands.rs`、`main.rs`、`permissions/local.toml` | `center_classes`、`center_assignments`、`center_binding`、`center_bind_assignment`、`center_pull_assets`、`student_bind_center` |
| 前端 | `pages/Assignments.jsx`、`lib/api.js`、`styles/app.css` | 作业卡新增「中心照片」：绑定中心班级+作业 → 拉取 → 结果（新增/跳过/未匹配）；未匹配的学生给下拉直接绑 |
| 单测 | `center_assets::tests` | 5 项：按中心账号优先、唯一同名匹配、同名不猜、无姓名未匹配、拉取去重与 `origin`/文件落盘 |

### 10.4 验收

- 中心 `mvn test`：**179 项全过**（新增 9 项）。
- 本地 `cargo test`：**132 passed / 2 ignored**，0 警告；迁移版本断言同步到 11。
- 本地 `npm run build`、`npm run app:build`：0 警告、签名与 dmg 校验通过。
- 中心 `npm run build`：通过（主 chunk 体积提示是既有现象）。

## 11. 真机失败修复（2026-11-18）

真机实测：手机能打开上传页，点「开始上传」后提示「上传失败：网络中断或服务已关闭」。页面能打开说明局域网地址、防火墙都通，问题只在 POST。

**根因（两处叠加）**：

1. **macOS 的 `accept()` 会继承监听套接字的 `O_NONBLOCK`。** 监听套接字为了不阻塞主循环设了 `set_nonblocking(true)`，接出来的连接也跟着是非阻塞的（Linux 不会）。请求头几百字节往往已经在缓冲区里，一次 `read` 就读到了，所以页面能打开；而几 MB 的图片 body 要走很多个 TCP 段，缓冲区一空就返回 `WouldBlock`，老代码当成错误直接 `return`——连接被关掉，手机端 `fetch` 失败。单测用 64×48 的图（约 200 字节）跟请求头一起到达，所以一直是绿的。
2. **请求体只做了一次 `read_exact`、失败就静默断连**；超过 12MB 时也是直接 `return Err` 而不回话，手机端同样只能显示「网络中断」。

**改了什么**：

| 层 | 改动 |
|---|---|
| `serve()` | `accept()` 后显式 `set_nonblocking(false)`，不依赖平台语义 |
| `read_request()` | 改成分块累积请求头（8KB buf + 找 `\r\n\r\n`），body 用「已读到的剩余字节 + 循环 `read` 补齐」；新增 `ReadError::Respond / Drop` 两类结果 |
| 错误回话 | 超过上限回 `413` + 「这张照片太大了（N MB，上限 12MB）…」；超时/传一半断掉回 `408 / 400`；断连只在连请求行都没读全时才发生 |
| 手机上传页 | 先 `createImageBitmap`/`<img>` 解码 → canvas 缩到长边 2200px → `toDataURL('image/jpeg', 0.82)` 再发；缩不了退回原图。不再按 `f.type` 过滤文件（部分安卓/微信浏览器给空 type，老写法会静默跳过），单张失败重试一次并给可行动文案 |
| 单测 | 新增 `segmented_upload_body_still_succeeds`（60ms 后按 8KB 分段发 400×400 噪声图，断言 200 且落库一条）与 `oversized_body_is_rejected_with_a_readable_error`（断言 413 + 文案含「太大」）；上传页自检加 `createImageBitmap` / `toDataURL` |
| 守护 | `scripts/verify.mjs` 加三条断言：必须 `set_nonblocking(false)`、必须有 413、必须有 `find_head_end` 与手机端压缩 |

验收：`cargo test` 154 passed / 2 ignored（新增 2 项），`npm run build`、`node scripts/verify.mjs` 全绿。

## 12. 阶段 B 已知边界

- 「教师把本地作业推成中心作业」仍未做（教学实体 outbox）。当前前提是**中心已有一份对应作业**，学生在那份作业下拍照提交，教师在本地选它绑定。要在本地建作业就直接出现在学生端，还需要作业双向同步。
- 中心网页端上传的照片与本地拉取的照片是两份副本（中心一份、本机一份），这是「离线优先」的代价：断网批改必须靠本机那份；不做「在线时才读中心图」的远程模式。
- 真实双端联调（中心整套服务 + 本地客户端）未在沙箱内跑；单元测试覆盖了鉴权、匹配、去重与落盘，扫码/登录/网络这些环节按 §6 和第 16 条手工验收步骤走一遍即可。

## 13. 仍未做

- 阶段 A′：Mac 端相机入口（见 §6）。
- 多网卡 / 代理环境下的地址选择：`lan_ip()` 仍取默认路由网卡，开着 VPN/代理时可能给出 `198.18.x.x` 这类手机到不了的地址。后续可枚举私有网段候选并在弹窗里列出备用地址。
- 教师把本地作业推成中心作业（教学实体 outbox），见 §12。
