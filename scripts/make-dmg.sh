#!/usr/bin/env bash
# 从已构建的 .app 生成 .dmg，不经过 Finder / AppleScript。
#
# 为什么不用 tauri 自带的 DMG 打包：tauri 内嵌的是 create-dmg 脚本，它要驱动 Finder
# 摆图标位置。在没有 Finder 自动化权限的机器上（CI、远程会话、首次运行未授权），
# 会以「Failed running AppleScript / 权限违例 -10004」失败，此时 .app 已经好了，
# 只是差一层压缩镜像。这个脚本用 hdiutil 直接做，代价是没有自定义背景与图标坐标。
#
# 用法：
#   bash scripts/make-dmg.sh                 # 用默认路径的 .app
#   bash scripts/make-dmg.sh /path/app.dmg   # 指定输出
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="NeuralKB Local"
BUNDLE="$ROOT/src-tauri/target/release/bundle"
# 优先用隐藏 staging 里的（package.sh 的产物）；兼容旧的 macos/ 路径。
if [[ -d "$BUNDLE/.staging/$APP_NAME.app" ]]; then
  APP="$BUNDLE/.staging/$APP_NAME.app"
else
  APP="$BUNDLE/macos/$APP_NAME.app"
fi
OUT_DIR="$BUNDLE/dmg"
ICON="$ROOT/src-tauri/icons/icon.icns"

if [[ ! -d "$APP" ]]; then
  echo "找不到 $APP" >&2
  echo "先构建应用：npm run app:app" >&2
  exit 1
fi

VERSION="$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$APP/Contents/Info.plist" 2>/dev/null || echo "0.0.0")"
ARCH="$(uname -m)"
[[ "$ARCH" == "arm64" ]] && ARCH="aarch64"
OUT="${1:-$OUT_DIR/${APP_NAME}_${VERSION}_${ARCH}.dmg}"

mkdir -p "$(dirname "$OUT")"

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/nbdmg.XXXXXXXX")"
# 中间镜像不能放在 STAGE 里：-srcfolder 会把整个目录打进去，等于把自己打包。
RW="$(mktemp "${TMPDIR:-/tmp}/nbdmg-rw.XXXXXXXX").dmg"
trap 'rm -rf "$STAGE" "$RW"' EXIT

echo "准备内容…"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
if [[ -f "$ICON" ]]; then
  cp "$ICON" "$STAGE/.VolumeIcon.icns"
fi

echo "生成镜像…"
hdiutil create -quiet -volname "$APP_NAME" -srcfolder "$STAGE" -fs HFS+ \
  -format UDRW -ov "$RW" >/dev/null

# 卷图标属性要靠 SetFile（Xcode 命令行工具自带），没有就跳过，不影响可用性。
if [[ -f "$ICON" ]] && command -v SetFile >/dev/null 2>&1; then
  MNT="$(hdiutil attach -nobrowse -readwrite -noautoopen "$RW" | tail -n 1 | cut -f 3)"
  if [[ -n "$MNT" ]]; then
    SetFile -a C "$MNT" 2>/dev/null || true
    hdiutil detach "$MNT" >/dev/null || true
  fi
fi

echo "压缩…"
rm -f "$OUT"
hdiutil convert -quiet "$RW" -format UDZO -imagekey zlib-level=9 -o "$OUT" >/dev/null

SIZE="$(du -h "$OUT" | cut -f 1)"
echo "完成：${OUT}（${SIZE}）"
