#!/usr/bin/env bash
# 一条命令出可分发的 .dmg。
#
#   bash scripts/package.sh            # 构建 + 打 dmg + 校验
#   bash scripts/package.sh --no-build # 复用已有 .app，只重新压镜像
#   bash scripts/package.sh -o ~/Desktop/NeuralKB.dmg
#
# 为什么分两步（先 --bundles app，再自己压 dmg）：
# Tauri 自带的 DMG 阶段用的是内嵌 create-dmg，它要驱动 Finder 摆图标位置。
# 在没给过 Finder 自动化权限的机器上（CI、远程会话、首次运行），这一步会失败，
# 而 .app 其实已经好了。所以这里只用 tauri 产出 .app，镜像交给 scripts/make-dmg.sh
# 用 hdiutil 直接做，整条链路不碰 Finder、不依赖终端颜色，任何 macOS 会话都能跑通。
set -euo pipefail

# 用 pwd -P 解析物理路径：旧目录 (neural-knowledge-base/local-client) 还留着
# 指向这里的符号链接，若用逻辑路径，tauri-build 会把绝对路径写进 rerun-if-changed，
# 两条路径交替打包会让 cargo 缓存反复失效、每次全量重编。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
APP_NAME="NeuralKB Local"
BUNDLE="$ROOT/src-tauri/target/release/bundle"
# 中间 .app 放隐藏 staging 而不是 bundle/macos/：macOS 会把 macos/ 里的 .app
# 注册进 LaunchServices，Launchpad 于是出现两个同名图标（一个指向 /Applications，
# 一个点了跳到源码目录）。隐藏目录 + .metadata_never_index 不会被索引。
STAGING="$BUNDLE/.staging"
APP="$STAGING/$APP_NAME.app"
OUT=""
DO_BUILD=1
DO_VERIFY=1
DO_OPEN=0

usage() {
  cat <<'EOF'
用法：bash scripts/package.sh [选项]

  -o, --out <路径>   指定 dmg 输出位置（默认 bundle/dmg/<名称>_<版本>_<架构>.dmg）
      --no-build     跳过构建，直接用已有的 .app 重新压镜像
      --no-verify    跳过 hdiutil verify
      --open         完成后在 Finder 里显示产物
  -h, --help         显示这段说明
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -o|--out)
      [[ $# -ge 2 ]] || { echo "✗ --out 后面要跟路径" >&2; exit 2; }
      OUT="$2"
      shift 2
      ;;
    --no-build) DO_BUILD=0; shift ;;
    --no-verify) DO_VERIFY=0; shift ;;
    --open) DO_OPEN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "✗ 未知参数：$1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ -t 1 ]]; then
  BOLD=$'\033[1m'; RED=$'\033[31m'; DIM=$'\033[2m'; OFF=$'\033[0m'
else
  BOLD=""; RED=""; DIM=""; OFF=""
fi

step() { printf '\n%s==> %s%s\n' "$BOLD" "$*" "$OFF"; }
fail() { printf '%s✗ %s%s\n' "$RED" "$*" "$OFF" >&2; exit 1; }

# ---------- 环境检查 ----------

[[ "$(uname -s)" == "Darwin" ]] || fail "dmg 只能在 macOS 上打（当前 $(uname -s)）"
command -v hdiutil >/dev/null || fail "找不到 hdiutil，这不是正常的 macOS 环境"

if [[ $DO_BUILD -eq 1 ]]; then
  for tool in node npm npx cargo; do
    command -v "$tool" >/dev/null || fail "找不到 ${tool}，先装好 Node 20.19+/22.12+ 与 Rust 工具链"
  done
fi

cd "$ROOT"

# ---------- 构建 ----------

if [[ $DO_BUILD -eq 1 ]]; then
  step "构建 release 包（前端 + Rust）"
  echo "${DIM}Tauri 的 DMG 阶段被跳过，镜像下面用 hdiutil 自己做${OFF}"
  npx tauri build --bundles app

  # 立刻把 Tauri 产出的 .app 移进隐藏目录，避免被 Launchpad 索引成一个假应用。
  BUILT="$BUNDLE/macos/$APP_NAME.app"
  if [[ -d "$BUILT" ]]; then
    mkdir -p "$STAGING"
    : > "$STAGING/.metadata_never_index"
    rm -rf "$APP"
    mv "$BUILT" "$APP"
    chflags hidden "$STAGING" 2>/dev/null || true
    # 旧的 macos/ 路径可能已被 LaunchServices 记住，注销掉。
    LS=/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister
    "$LS" -u -f "$BUILT" >/dev/null 2>&1 || true
  fi
else
  step "跳过构建，复用已有 .app"
fi

[[ -d "$APP" ]] || fail "没找到 $APP —— 去掉 --no-build 先完整构建一次"

# ---------- 盖稳定签名 ----------
# tauri 默认出的是 adhoc 签名，cdhash 每次编译都变，屏幕录制权限会跟着失效
# （「设置里明明勾着却还要授权」的根因）。换成固定自签证书后 TCC 认证书指纹，
# 重编译不再影响授权。失败不阻断出包，但明确告知后果。
if [[ $DO_BUILD -eq 1 ]]; then
  step "盖稳定签名"
  if bash "$ROOT/scripts/sign.sh" "$APP"; then
    :
  else
    echo "⚠ 签名失败；应用仍能跑，但每次重新打包都要重新授权屏幕录制" >&2
  fi
fi

# ---------- 压制镜像 ----------

VERSION="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist" 2>/dev/null || echo 0.0.0)"
ARCH="$(uname -m)"
[[ "$ARCH" == "arm64" ]] && ARCH="aarch64"

if [[ -z "$OUT" ]]; then
  OUT="$BUNDLE/dmg/${APP_NAME}_${VERSION}_${ARCH}.dmg"
else
  # 允许 -o ~/Desktop/xx.dmg 这种写法：相对路径按调用时的目录算。
  case "$OUT" in
    /*) ;;
    *) OUT="$(pwd)/$OUT" ;;
  esac
fi

step "打包 DMG（${APP_NAME} ${VERSION} / ${ARCH}）"
bash "$ROOT/scripts/make-dmg.sh" "$OUT"

# ---------- 校验与摘要 ----------

if [[ $DO_VERIFY -eq 1 ]]; then
  step "校验镜像"
  hdiutil verify "$OUT" | tail -n 1
fi

SIZE="$(du -h "$OUT" | cut -f 1)"
SHA="$(shasum -a 256 "$OUT" | cut -d ' ' -f 1)"

step "完成"
printf '  应用  %s\n' "$APP"
printf '  镜像  %s（%s）\n' "$OUT" "$SIZE"
printf '  校验  %s\n' "$SHA"
echo "${DIM}自签证书：别的机器首次打开要右键 →「打开」；本机不再因重编译丢屏幕录制权限${OFF}"

if [[ $DO_OPEN -eq 1 ]]; then
  open -R "$OUT"
fi
