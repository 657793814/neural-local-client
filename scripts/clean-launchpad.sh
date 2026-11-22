#!/usr/bin/env bash
# 清理 Launchpad 里多出来的 NeuralKB 图标（兜底工具，正常打包脚本已自动处理）。
#
# 现象：Launchpad 里每个应用出现两次，其中一个「显示简介」会跳到
#       src-tauri/target/release/bundle/macos/*.app 这类源码目录。
# 根因：macOS 把构建产物目录、挂过的 dmg 卷里的 .app 也注册进 LaunchServices，
#       Launchpad 把它们当独立应用列出来。/Applications 下才是真正安装的。
#
# 打包脚本（scripts/package.sh）现在会把中间 .app 直接产到隐藏的 .staging/ 目录，
# 理论不会再犯。本脚本用于清理历史遗留图标。
#
# 用法：bash scripts/clean-launchpad.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LS=/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister

# 1. 永久排除索引：标记放在 src-tauri/ 根上，cargo clean 不会删，整个 target/ 生效
[[ -d "$ROOT/src-tauri" ]] && : > "$ROOT/src-tauri/.metadata_never_index"
echo "✓ 已标记构建目录不参与索引"

# 2. 把还留在 macos/ 里的旧产物移进隐藏 staging（正常打包不会走到这）
name="NeuralKB Local.app"
src="$ROOT/src-tauri/target/release/bundle/macos/$name"
stage="$ROOT/src-tauri/target/release/bundle/.staging"
if [[ -d "$src" ]]; then
  mkdir -p "$stage"
  : > "$stage/.metadata_never_index"
  rm -rf "$stage/$name"
  mv "$src" "$stage/$name"
  chflags hidden "$stage" 2>/dev/null || true
  echo "✓ 已移走旧构建产物"
fi

# 3. 注销所有非 /Applications 的残留 .app 注册。
#    注意：路径可能已被卸载/删除，所以不能先判断文件存在——那些正是要清的。
targets=(
  "$ROOT/src-tauri/target/release/bundle/macos/NeuralKB Local.app"
  "$ROOT/src-tauri/target/release/bundle/.staging/NeuralKB Local.app"
  "$HOME/.Trash/NeuralKB.app"
  "$HOME/.Trash/NeuralKB Local.app"
  "$HOME/.Trash/NeuralKB 18.05.48.app"
  "/Volumes/NeuralKB/NeuralKB.app"
  "/Volumes/NeuralKB 1/NeuralKB.app"
  "/Volumes/NeuralKB Local/NeuralKB Local.app"
  "/Volumes/NeuralKB Local 1/NeuralKB Local.app"
  "/private/tmp/nb-dmg-test/NeuralKB.app"
)
for path in "${targets[@]}"; do
  "$LS" -u -f "$path" >/dev/null 2>&1 || true
done
echo "✓ 已取消所有残留应用注册"

# 4. 卸载还没推出的 dmg 卷（挂载期间 Launchpad 会一直把里面的 app 收进去）
mounted="$(mount | grep -i '/Volumes/NeuralKB' | sed -E 's#.* on (/Volumes/[^ ]+) \(.*#\1#' || true)"
if [[ -n "$mounted" ]]; then
  while IFS= read -r vol; do
    [[ -n "$vol" ]] || continue
    hdiutil detach "$vol" >/dev/null 2>&1 || true
    echo "✓ 已卸载 $vol"
  done <<< "$mounted"
fi

# 5. 重建索引 + 重启 Dock（Launchpad 数据库由 Dock 维护）
if command -v mdutil >/dev/null; then
  mdutil -E / >/dev/null 2>&1 || true
  echo "✓ 已触发 Spotlight 索引重建（后台进行）"
fi
killall Dock 2>/dev/null || true
echo "✓ 已重启 Dock"

echo
echo "完成。Launchpad 里应该只剩 /Applications 下真正安装的应用。"
echo "若旧图标仍在：退出登录再登录一次（索引重建需要几分钟）。"
