# Windows 移植落地文档

> 状态：阶段一已实施（代码适配 + 编译验证）｜提出：2026-11-22｜目标：`local-client`（Tauri 2 + React 19 + SQLite）
> 原则：**macOS 行为零改动**——所有平台差异用 `#[cfg]` 门控，macOS 路径的代码与行为保持原样，每个阶段结束必须 `cargo test` 全绿。

## 1. 现状盘点

代码并非完全 macOS 硬编码，已经有相当一部分预埋了门控：

| 模块 | 门控状态 | 说明 |
|---|---|---|
| `filein.rs`（文件取题） | ✅ 已有 `#[cfg(target_os = "macos")]` | sips / textutil / CoreGraphics PDF 都有非 macOS stub（报错或降级） |
| `ocr.rs`（本机 OCR） | ✅ 已有门控 | Vision 在 `recognize_image` 内部门控，非 macOS 返回错误 → 自动模式回退「AI 识图」（链路已存在） |
| `screen.rs`（截屏抓帧） | ✅ `mod display` 已门控 | 跨平台部分（缩放、裁剪、标注）本来就不依赖 macOS |
| `overlay.rs`（选区窗口） | ✅ 纯 Tauri API | WebviewWindow + monitor 几何，跨平台 |
| `keyring`（Key 存储） | ✅ 天然跨平台 | `keyring` crate 已开 `windows-native` feature，Windows 落凭据管理器；本地 Key 仓库是纯文件实现 |
| `.docx` 解析 | ✅ 纯 Rust | 解 zip 读 `word/document.xml`，无系统依赖 |
| SQLite / FTS5 / argon2 / 备份 | ✅ 纯 Rust（bundled） | 零改动 |
| 试卷 / 课件打印 | ✅ 走系统浏览器 | Windows 有 Edge / Chrome |
| `.rtf` / `.doc` / `.odt` | ❌ macOS textutil | Windows 需要替代（见下） |
| PDF 栅格化 | ❌ macOS CoreGraphics | Windows 需要 pdfium（见下） |
| HEIC 图片 | ❌ macOS sips | Windows 需要替代（见下） |

## 2. 依赖映射与替代方案

| 能力 | macOS 现状 | Windows 方案 | 选型理由 |
|---|---|---|---|
| 本机 OCR | Vision `VNRecognizeTextRequest` | **阶段二**：`Windows.Media.Ocr`（WinRT，`windows` crate） | 免费离线、系统自带、中文质量可用；接口形态与 Vision 相近（都是「图 → 行列表 + 置信度」） |
| 本机 OCR 兜底 | 失败回退 AI 识图 | **阶段一即生效**：`recognize_image` 报错 → 走既有 AI 识图回退 | 零新代码，Windows 用户配一个视觉模型即可录题 |
| PDF 渲染 | CoreGraphics | **pdfium**（`pdfium-render` crate） | Chrome 的 PDF 引擎，逐页栅格化 + 指定 DPI 与现有接口一一对应；运行时需要 `pdfium.dll`（随包分发，见 §5） |
| 图片格式兜底 | sips（HEIC/TIFF/BMP） | `image` crate 加 `tiff`/`bmp` feature（跨平台原生解码）；HEIC 在 Windows 明确报错引导转 JPG/PNG | TIFF/BMP 直接消灭 90% 的 sips 用例；HEIC 在 Windows 无系统解码器，装 libheif 得不偿失 |
| .rtf 转文字 | textutil | **内置 `rtf_to_text`**（纯 Rust 迷你解析器，`filein.rs`，含单测） | 控制词剥离 + `\uNNNN` Unicode + `\'hh` 按代码页（GBK/CP1252）解码，取题场景够用；不引第三方解析器，macOS 上也能跑单测 |
| .doc / .odt / webarchive | textutil | 明确报错：「请先另存为 .docx」 | 老格式没有像样的纯 Rust 解析器，报错优于硬猜 |
| 截屏抓帧 | CGWindowList + 帧缓冲直取 | **阶段三**：DXGI Desktop Duplication 或 GDI BitBlt（`windows` crate / `scrap` crate） | 抓屏是唯一必须重写的平台层；选区窗口（overlay）跨平台，不用动 |
| ⌘⇧O 全局热键 | tauri-plugin-global-shortcut | 同一插件，改注册键为 Ctrl+Shift+O | 插件本身跨平台 |
| 安装包 | dmg（hdiutil 自压） | `msi` + `nsis`（tauri.windows.conf.json 覆盖） | Tauri 原生支持，不需要 scripts/package.sh（那条链路写死了 macOS） |
| 代码签名 | 自签稳定证书 | 阶段四（可选）：EV 证书 / Azure Trusted Signing | 未签名时 SmartScreen 会拦，需「仍要运行」；不影响功能 |

## 3. 分阶段实施

### 阶段一：最小可用 Windows 版（本次已实施）

范围：不新增大依赖的前提下让 Windows 能编译、能跑主链路。OCR 暂走 AI 识图，截图选区暂不可用，文件取题 / 手工录题 / 题包导入导出 / 试卷 / 作业 / 学情全部可用。

改动清单（全部 `#[cfg]` 门控，macOS 无感）：

1. `Cargo.toml`：`image` 加 `tiff`、`bmp` feature；新增 `[target.'cfg(not(target_os = "macos"))'.dependencies]`：`pdfium-render`。
2. `filein.rs`：
   - 非 macOS 的 `pdf` 模块改为 pdfium 实现（`pdfium.dll` 缺失时报可操作错误，而不是假装一页）；
   - `via_sips` 非 macOS 报错文案改为「HEIC 暂不支持，请转 JPG/PNG」；
   - `via_textutil` 非 macOS：`.rtf` 走内置 `rtf_to_text`（新单测覆盖），`.doc`/`.odt`/`.webarchive` 报引导性错误，`.html` 走内置标签剥离。
3. `main.rs` / `commands.rs`：截图热键与截屏入口在非 macOS 返回明确提示「截图选区暂不支持 Windows，请用文件取题 / 手工录题」（阶段三移除）。
4. 打包：`src-tauri/tauri.windows.conf.json` 覆盖 `bundle.targets = ["msi", "nsis"]`。
5. 验证（分两层）：
   - **Linux 容器交叉检查**：`cfg(not(target_os = "macos"))` 的全部代码（RTF 解析器、HTML 剥离、Windows 错误分支等）在 `rust:latest` Linux 容器里 `cargo check` 通过——这部分代码与 Windows 目标共用同一批 `not(macos)` 分支；
   - **Windows 完整检查**：`cfg(windows)` 专属代码（pdfium_backend 等）需要在 Windows 机器或 CI 上验证——本机 mingw-w64 因 ghcr.io 网络故障与 macOS sandbox-exec 限制装不上，已备好 `.github/workflows/windows-check.yml`（windows-latest 跑 `cargo check --all-targets` + `cargo test`），仓库有远端后 push 即自动验证。
   - macOS 侧：`cargo test` 217 项全过（新增 RTF / HTML 单测 5 项），`npm run build` 通过，无回归。

### 阶段二：Windows 本机 OCR

- `windows` crate 引入 `Media_Ocr` + `Graphics_Imaging` + `Storage_Streams` features；
- `ocr.rs::recognize_image` 的非 macOS 分支实现 SoftwarBitmap → OcrEngine → 逐行 `OcrLine` + 置信度映射（Vision 的 `VNRecognizedText.confidence` 对应 Windows 的 `OcrWord` 无置信度，统一给 0.9 或按行聚合省略）；
- 切题 / 拆选项 / 标记识别逻辑（`questions_from_lines` 以下全部）平台无关，直接复用；
- 验收：同一张试卷图，Windows 本机 OCR 与 macOS Vision 的出题数差异 ≤1。

### 阶段三：Windows 截图选区

- `screen.rs` 新增 `#[cfg(windows)] mod display`：DXGI Desktop Duplication 抓主屏帧（首选）或 GDI BitBlt（兜底）；
- `overlay.rs` 适配 Windows 的多显示器坐标（逻辑坐标 vs 物理像素，DPI 感知）；
- 热键注册按平台切换 ⌘⇧O / Ctrl+Shift+O；
- 验收：Windows 上 ⌘⇧O 等价流程跑通「选区 → 标注 → 识别」。

### 阶段四：发布链路（可选）

- CI：GitHub Actions `windows-latest` + `tauri-apps/tauri-action`，出 `.msi` / `.exe`；
- 签名：Azure Trusted Signing 或 EV 证书，消除 SmartScreen 拦截；
- 自动更新：tauri-plugin-updater（Windows 端配置 manifest）。

## 4. 验收标准（阶段一）

1. `cargo check --target x86_64-pc-windows-msvc` 零错误；
2. macOS `cargo test` 212 项全过，`npm run build` 通过；
3. macOS 打包链路不受影响（`npm run app:build` 仍出 dmg）；
4. 所有非 macOS stub 的报错信息可操作（告诉用户该用什么替代）。

## 5. 已知限制与风险

- **pdfium.dll 分发**：pdfium-render 运行时加载动态库，Windows 安装包需携带 `pdfium.dll`（约 5 MB）。方案：`src-tauri/resources/` 放入并在 `tauri.windows.conf.json` 的 `bundle.resources` 声明；构建机从 https://github.com/bblanchon/pdfium-binaries 下载。阶段一先实现「找得到就加载、找不到报清晰错误」，随包分发放到 CI 阶段做。
- **HEIC**：Windows 无系统解码器，报错引导转格式；iPhone 用户建议在传输时选「自动」转换（默认就是 JPG）。
- **.doc 老格式**：Windows 上明确报错引导另存为 .docx，不做半吊子解析。
- **截图选区**：阶段一在 Windows 禁用，录入走「本地文件取题」（PDF/图片/Word 都支持）或手工录题。
- **中文路径**：rusqlite/zip/image 都是 UTF-8 处理，Windows 路径用 `std::path` 已自动兼容；注意日志与临时目录拼接统一用 `PathBuf`，不要手拼字符串。
