# NeuralKB 本地题库

面向独立个体的**离线优先**题库客户端：所有题目数据只存在本机，只有调用你自配的 AI 模型时才联网。

- **平台**：macOS 11.0+（Windows / Linux 移植进行中）
- **数据**：SQLite 存在 `~/Documents/NeuralKB-Local`，不上传、不收集使用日志
- **许可**：[PolyForm Noncommercial 1.0.0](LICENSE) —— 个人 / 教育 / 非营利免费，**禁止商用**

## 它能做什么

- **录题**：截图识别（⌘⇧O）、本机离线 OCR、内置浏览器网页取题、图片 / PDF / Word / txt 导入、手工录入
- **题库**：搜索、标签、知识点、难度、来源筛选；题目包 JSON / JSONL / ZIP 导入导出；草稿复核后才正式入库
- **AI**：出题、批量归类、识图、矢量制图、学情诊断（需自配 OpenAI 兼容模型，断网不影响其余功能）
- **试卷**：套方案或蓝图组卷、换题撤销、答题卡模板、A4/A3/B5/8K 浏览器排版打印
- **教学**：班级与学生、作业布置与批改、拍照批改纸质卷、考试成绩、学情矩阵 / 错题本 / 家长周报
- **课件**：AI 生成讲义式课件，或导入 DOCX / PPTX / PDF / MD 后整理导出

完整功能清单与实现细节见 [docs/product.md](docs/product.md)。

## 安装

1. 到 Releases 下载最新的 `NeuralKB Local_0.1.0_aarch64.dmg`（或从源码自行打包，见下）；
2. 双击挂载，把 `NeuralKB Local.app` 拖进「应用程序」；
3. 首次打开若提示「无法验证开发者」，右键 →「打开」即可（自签名的正常现象）；
4. 按 5 步向导建账号、按需配置模型，之后就能用。**模型可以跳过**，题库、录题、试卷等功能不依赖 AI。

截图识别需要在「系统设置 → 隐私与安全性 → 屏幕录制」里勾选 NeuralKB Local。

## 从源码打包（macOS）

需要 Node.js 20.19+ / 22.12+、Rust stable、Xcode Command Line Tools（`xcode-select --install`）。

```bash
npm install
npm run app:build                # 一条命令出 .dmg（含构建、自签名、校验）
bash scripts/package.sh --no-build   # 只重压镜像，调 dmg 布局时用
```

产物在 `src-tauri/target/release/bundle/dmg/`。开发模式用 `npm run app:dev`（Vite + Tauri 窗口，`npm run dev` 只起前端）。后端测试：`cd src-tauri && cargo test`。

Windows / Linux 的移植进度见 [docs/windows-port-plan.md](docs/windows-port-plan.md)。

## 数据放在哪

默认 `~/Documents/NeuralKB-Local`：

```
neuralkb.db      SQLite（题目、知识点、模型池、作答记录等）
assets/<题 id>/  图片附件
tmp/             截图、PDF 渲染等中间图（可清）
backup/          整库备份 ZIP（只留最近 3 份）
```

可在设置页改数据目录、导出整库备份或数据库副本。改目录会自动把数据复制过去。

## 常见问题

**Q：截图识别一直要授权？**
到「系统设置 → 隐私与安全性 → 屏幕录制」勾选本应用。安装包用稳定自签证书，重新打包不会丢授权。

**Q：`npm run app:dev` 提示端口 5183 被占用？**
端口是固定配置。`lsof -ti:5183 | xargs kill` 后重试。

**Q：Launchpad 出现两个一样的图标？**
旧构建产物被系统注册了。跑 `bash scripts/clean-launchpad.sh` 清理。

**Q：调用模型反复弹钥匙串授权？**
Key 首次读取后会落到本地仓库，正常不再弹。若反复弹，删掉 `~/Library/Application Support/com.neuralkb.local/secrets/` 里的对应条目重录。

**Q：Rust 首次编译很慢？**
首次要拉全部依赖，几分钟到十几分钟属正常，之后增量很快。失败先确认 Xcode CLT 与 Rust stable 已装好。

**Q：能用 Docker 部署吗？**
不能。这是桌面应用，不是 Web 服务，数据和界面都在用户本机。

## 许可与隐私

本项目采用 [PolyForm Noncommercial 1.0.0](LICENSE)：个人、研究、教育、非营利组织可自由使用；**商业用途需另行获得授权**。

API Key 与同步令牌存在本机，不收集使用日志，断网时除 AI 外全部功能可用。详见 [docs/product.md](docs/product.md) 的「安全与隐私」一节。
