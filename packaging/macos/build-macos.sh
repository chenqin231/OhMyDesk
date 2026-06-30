#!/usr/bin/env bash
# 构建 OhMyDesk 客户端 macOS .app bundle + .dmg。
# 用法（须在 macOS 上执行，依赖 codesign / hdiutil / sips / iconutil）：
#   先 `cargo build -p client --release --target <triple>`，
#   再 `BIN=target/<triple>/release/client ARCH=arm64 bash packaging/macos/build-macos.sh`。
#
# 说明：本脚本只做 ad-hoc 签名（codesign -s -），不做 Developer ID 签名与公证(notarize)。
# 故 arm64 能正常执行，但用户首次打开仍会遇到 Gatekeeper「无法验证开发者」提示，
# 需右键「打开」或 `xattr -dr com.apple.quarantine`（dmg 内附说明）。
# 要消除该提示须申请 Apple Developer Program 走 notarytool 公证——另行实现。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

APP_NAME="OhMyDesk"
BUNDLE_ID="com.ohmydesk.client"
EXE="ohmydesk-client"
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"
[ -z "$VERSION" ] && VERSION=0.1.0
# ARCH/BIN 允许环境变量覆盖：CI 多架构构建时由 release.yml 注入目标三元组路径。
ARCH="${ARCH:-$(uname -m)}"
BIN="${BIN:-target/release/client}"

if [ ! -f "$BIN" ]; then
  echo "✗ 未找到 $BIN，请先：cargo build -p client --release --target <triple>" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
APP="$WORK/${APP_NAME}.app"

# ── .app 目录树 ──────────────────────────────────────────────────────────────
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# 主二进制（Slint GUI 程序的默认值已足够：缺省自动探测登录用户 + 默认中转，
# 故双击 .app 直接可用，无需启动包装脚本）。
install -m755 "$BIN" "$APP/Contents/MacOS/$EXE"
strip "$APP/Contents/MacOS/$EXE" 2>/dev/null || true

# ── 图标（best-effort）：存在 packaging/macos/AppIcon.png(建议 1024×1024) 时生成 .icns ──
# 用标准 iconset 命名（base@1x + @2x），否则 iconutil 拒绝。无源图则跳过，用系统通用图标。
ICON_SRC="$REPO_ROOT/packaging/macos/AppIcon.png"
ICON_KEY=""
if [ -f "$ICON_SRC" ] && command -v iconutil >/dev/null 2>&1; then
  ICONSET="$WORK/AppIcon.iconset"
  mkdir -p "$ICONSET"
  gen() { sips -z "$1" "$1" "$ICON_SRC" --out "$ICONSET/$2" >/dev/null 2>&1 || true; }
  for base in 16 32 128 256 512; do
    gen "$base"           "icon_${base}x${base}.png"
    gen "$((base * 2))"   "icon_${base}x${base}@2x.png"
  done
  if iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns" 2>/dev/null; then
    ICON_KEY='  <key>CFBundleIconFile</key><string>AppIcon</string>'
  fi
fi

# ── Info.plist ───────────────────────────────────────────────────────────────
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>${APP_NAME}</string>
  <key>CFBundleDisplayName</key><string>OhMyDesk 终端</string>
  <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleExecutable</key><string>${EXE}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
${ICON_KEY}
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
</dict>
</plist>
EOF

# ── ad-hoc 签名（arm64 必需，否则报「已损坏」无法执行；不消除 Gatekeeper 提示）──
codesign --force --sign - --timestamp=none "$APP" >/dev/null 2>&1 \
  || codesign --force --sign - "$APP"
codesign --verify --verbose=2 "$APP" 2>&1 | sed -n '1,3p' || true

# ── dmg 暂存：.app + /Applications 软链（拖拽安装）+ 首次打开说明 ──────────────
STAGE="$WORK/dmg"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cat > "$STAGE/首次打开说明.txt" <<'EOF'
OhMyDesk 终端 —— 首次打开说明

本应用未做 Apple 公证(notarize)，macOS 首次打开会提示
「无法验证开发者 / 是否包含恶意软件」。这是系统对未公证应用的默认拦截，
不代表应用有问题。按以下任一方式放行即可：

方式一（推荐）：
  1. 把 OhMyDesk.app 拖到「应用程序」文件夹；
  2. 在「应用程序」里右键点 OhMyDesk → 选「打开」→ 弹窗里再点「打开」。
  （只需首次这样做一次，之后双击即可。）

方式二（命令行）：
  在「终端」执行（按实际路径替换）：
    xattr -dr com.apple.quarantine /Applications/OhMyDesk.app

方式三：
  系统设置 → 隐私与安全性 → 下滑找到被拦截提示 → 点「仍要打开」。

注：首次运行需在「系统设置 → 隐私与安全性」授予
「屏幕录制」与「辅助功能」权限，远程画面与键鼠控制才能生效。
EOF

# ── 生成 dmg ─────────────────────────────────────────────────────────────────
OUT="$REPO_ROOT/ohmydesk-client-macos-${ARCH}.dmg"
rm -f "$OUT"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" \
  -fs HFS+ -format UDZO -ov "$OUT" >/dev/null

echo ""
echo "✓ 已生成：$OUT"
ls -lh "$OUT"
echo "  bundle id=${BUNDLE_ID}  version=${VERSION}  arch=${ARCH}"
echo "  签名：ad-hoc（未公证；用户首次打开需右键『打开』放行）"
