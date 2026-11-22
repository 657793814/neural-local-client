#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# 给 NeuralKB Local.app 盖一个「稳定的」签名。
#
#   ./scripts/sign.sh <NeuralKB Local.app>   签名并打印 designated requirement
#   ./scripts/sign.sh --identity             只确保签名身份存在，然后打印它
#   ./scripts/sign.sh --perm-reset           清掉旧的屏幕录制授权记录并打开设置页
#
# 为什么非签不可（与 desktop/sign.sh 同一套机制，见 desktop/README.md）：
#
#   cargo / tauri 默认给二进制盖的是 adhoc 签名（linker-signed）。adhoc 没有
#   证书，TCC 只能拿 cdhash 当身份——每编译一次 cdhash 就变一次。而「屏幕录制」
#   这条权限是认身份的：改一行代码重新打包 → cdhash 变了 → 系统眼里是另一个
#   程序 → 授权失效，但「系统设置」里那一行还勾着（记的是旧身份）。这就是
#   「反复要求授权」的根因。
#
#   换成固定的自签证书之后，designated requirement 变成
#   「identifier + 证书指纹」，重编译不再影响它，授权一次就一直有效。
#
# 证书与 desktop 壳共用那张 "NeuralKB Dev"（同一个 SIGN_DIR）。两个应用的
# identifier 不同（com.neuralkb.local / com.neuralkb.shell），是各自独立的 TCC
# 身份，互不干扰；共用证书只是不再多生成一张、少一次指纹漂移的机会。
# 证书存在登录钥匙串 + $HOME/.neuralkb/signing，删掉就等于撤销这个身份。
# ---------------------------------------------------------------------------
set -uo pipefail

SELF_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# 证书必须存在一个**永远不会被清理**的地方：放家目录，仓库随便清、换分支、
# 重新 clone 都不影响身份。放仓库里一旦被清、钥匙串又丢，就会新建一张新指纹
# → TCC 眼里是另一个程序 → 用户又得去系统设置里勾一次。
SIGN_DIR="$HOME/.neuralkb/signing"
KEYCHAIN="$SIGN_DIR/nb-sign.keychain"
IDENTITY="NeuralKB Dev"
BUNDLE_ID="com.neuralkb.local"
PASS="neuralkb-dev"
LOGIN_KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

info() { printf '%s\n' "$*"; }
warn() { printf '[warn] %s\n' "$*" >&2; }
fail() { printf '[fail] %s\n' "$*" >&2; }

has_identity() {
  security find-identity -v -p codesigning 2>/dev/null | grep -q "$IDENTITY"
}

# 造一张自签的代码签名证书。只在找不到身份、也没有备份时跑一次。
create_identity() {
  mkdir -p "$SIGN_DIR" || return 1
  if [ ! -f "$SIGN_DIR/cert.pem" ] || [ ! -f "$SIGN_DIR/key.pem" ]; then
    local stamp
    stamp=$(date +%s)
    [ -f "$SIGN_DIR/cert.pem" ] && mv "$SIGN_DIR/cert.pem" "$SIGN_DIR/cert.abandoned.$stamp.pem"
    [ -f "$SIGN_DIR/identity.p12" ] && mv "$SIGN_DIR/identity.p12" "$SIGN_DIR/identity.abandoned.$stamp.p12"
    info "生成自签证书（${IDENTITY}，10 年有效）…"
    openssl req -x509 -newkey rsa:2048 \
      -keyout "$SIGN_DIR/key.pem" -out "$SIGN_DIR/cert.pem" \
      -days 3650 -nodes -subj "/CN=$IDENTITY/O=NeuralKB" \
      -addext "keyUsage=critical,digitalSignature" \
      -addext "extendedKeyUsage=critical,codeSigning" \
      -addext "basicConstraints=critical,CA:false" >/dev/null 2>&1 || {
      fail "openssl 生成证书失败"
      return 1
    }
  fi

  # openssl 3 默认的 PKCS#12 加密 macOS 的 security 认不出来，必须用 -legacy。
  local p12="$SIGN_DIR/neuralkb-dev.p12"
  openssl pkcs12 -export -legacy \
    -inkey "$SIGN_DIR/key.pem" -in "$SIGN_DIR/cert.pem" \
    -out "$p12" -passout "pass:$PASS" -name "$IDENTITY" >/dev/null 2>&1 || {
    warn "openssl 导出 p12 失败（老版本 openssl 可能不支持 -legacy）"
    return 1
  }

  security create-keychain -p "$PASS" "$KEYCHAIN" >/dev/null 2>&1 || true
  security unlock-keychain -p "$PASS" "$KEYCHAIN" >/dev/null 2>&1 || true
  security import "$p12" -k "$KEYCHAIN" -P "$PASS" -T /usr/bin/codesign -A >/dev/null 2>&1 || {
    fail "导入临时钥匙串失败"
    return 1
  }
  security add-trusted-cert -p codeSign -r trustRoot -k "$KEYCHAIN" "$SIGN_DIR/cert.pem" >/dev/null 2>&1 || true
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$PASS" "$KEYCHAIN" >/dev/null 2>&1 || true

  # codesign 只认登录钥匙串里能查到的身份，所以再往登录钥匙串导一份。
  security import "$p12" -k "$LOGIN_KEYCHAIN" -P "$PASS" -T /usr/bin/codesign -A >/dev/null 2>&1 || {
    fail "导入登录钥匙串失败"
    return 1
  }

  has_identity
}

# 把钥匙串里的身份导出成文件备份。私钥一丢就再也签不出同一个身份。
backup_identity() {
  mkdir -p "$SIGN_DIR" 2>/dev/null || return 0
  if [ ! -f "$SIGN_DIR/cert.pem" ]; then
    security find-certificate -c "$IDENTITY" -p -a 2>/dev/null \
      | awk '/BEGIN CERTIFICATE/,/END CERTIFICATE/' > "$SIGN_DIR/cert.pem" 2>/dev/null || true
  fi
  if [ ! -f "$SIGN_DIR/identity.p12" ]; then
    security export -t identities -f pkcs12 -k "$LOGIN_KEYCHAIN" \
      -P "$PASS" -o "$SIGN_DIR/identity.p12" >/dev/null 2>&1 || true
  fi
}

# 从备份把身份装回钥匙串。**绝不重新生成**：新指纹＝新身份＝重新授权。
restore_identity() {
  local p12="$SIGN_DIR/identity.p12"
  [ -f "$p12" ] || return 1
  info "从备份恢复签名身份（指纹不变，已有授权继续有效）"
  security import "$p12" -k "$LOGIN_KEYCHAIN" -P "$PASS" -T /usr/bin/codesign -A >/dev/null 2>&1 || return 1
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$PASS" "$LOGIN_KEYCHAIN" >/dev/null 2>&1 || true
  has_identity
}

ensure_identity() {
  if has_identity; then
    backup_identity
    return 0
  fi
  if restore_identity; then
    backup_identity
    return 0
  fi
  warn "找不到签名身份、也没有备份，只能新建一张。"
  warn "新证书＝新指纹，macOS 会当成另一个程序，屏幕录制要重新授权一次。"
  create_identity || return 1
  if ! has_identity; then
    fail "签名身份 $IDENTITY 建好了但系统不认（trust 没写进去）"
    return 1
  fi
  backup_identity
}

sign_app() {
  local target=${1:-}
  if [ -z "$target" ] || [ ! -d "$target" ]; then
    fail "用法：$0 <NeuralKB Local.app>"
    return 1
  fi
  ensure_identity || return 1
  rm -rf "$target/Contents/_CodeSignature"
  if ! codesign --force --deep --sign "$IDENTITY" \
    --identifier "$BUNDLE_ID" "$target" 2>&1 | tail -3; then
    fail "codesign 失败"
    return 1
  fi
  info "已签名：${target}"
  codesign -d -r- "$target" 2>&1 | sed -n 's/^designated => /  要求：/p'
}

if [ "${1:-}" = "--identity" ]; then
  ensure_identity && security find-identity -v -p codesigning | grep "$IDENTITY"
  exit $?
fi

# 切换成固定签名后，旧 adhoc 身份留下的授权记录还勾在设置面板里、容易看岔。
# 清一次，再打开应用截一次图，系统弹窗里允许即可，之后重编译不再问。
if [ "${1:-}" = "--perm-reset" ]; then
  info "清除旧的屏幕录制授权记录（${BUNDLE_ID}）"
  tccutil reset ScreenCapture "$BUNDLE_ID" 2>/dev/null || warn "tccutil 清理失败，可到系统设置里手动关掉再打开"
  open "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture" 2>/dev/null || true
  info "已打开设置页。请勾选 NeuralKB Local，然后完全退出应用（⌘Q）再打开。"
  exit 0
fi

sign_app "${1:-}"
