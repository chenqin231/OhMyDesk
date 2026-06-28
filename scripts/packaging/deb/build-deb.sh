#!/usr/bin/env bash
# 构建 OhMyDesk 客户端 .deb 包（dpkg-deb，自动从 ldd 推导运行时依赖）。
# 用法：先 `cargo build -p client --release`，再在仓库根执行 `bash packaging/deb/build-deb.sh`。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$REPO_ROOT"

PKG=ohmydesk-client
VERSION="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"(.*)".*/\1/')"
[ -z "$VERSION" ] && VERSION=0.1.0
# ARCH/BIN/DEPS 允许环境变量覆盖：CI 交叉/多架构构建时由 release.yml 注入
# （宿主架构 ≠ 目标架构时，dpkg --print-architecture 会标错，故须显式传 ARCH）。
ARCH="${ARCH:-$(dpkg --print-architecture)}"
BIN="${BIN:-target/release/client}"

if [ ! -f "$BIN" ]; then
  echo "✗ 未找到 $BIN，请先：cargo build -p client --release" >&2
  exit 1
fi

STAGE="$(mktemp -d)/${PKG}_${VERSION}_${ARCH}"
trap 'rm -rf "$(dirname "$STAGE")"' EXIT

# ── 目录树 ──────────────────────────────────────────────────────────────────
install -d "$STAGE/DEBIAN" "$STAGE/usr/bin" "$STAGE/etc/ohmydesk" \
           "$STAGE/usr/share/applications" "$STAGE/usr/share/doc/$PKG"

# 主二进制（strip 瘦身）
install -m755 "$BIN" "$STAGE/usr/bin/ohmydesk-client"
strip "$STAGE/usr/bin/ohmydesk-client" 2>/dev/null || true

# 启动封装：加载配置 + 传使用人名
cat > "$STAGE/usr/bin/ohmydesk-client-launch" <<'EOF'
#!/bin/sh
# 加载内网服务端配置后启动 Agent。参数 1 = 使用人名（缺省 用户@主机）。
set -a
[ -f /etc/ohmydesk/client.env ] && . /etc/ohmydesk/client.env
set +a
exec /usr/bin/ohmydesk-client "${1:-${USER:-user}@$(hostname)}"
EOF
chmod 755 "$STAGE/usr/bin/ohmydesk-client-launch"

# 配置文件（conffile：升级不覆盖管理员改动）
cat > "$STAGE/etc/ohmydesk/client.env" <<'EOF'
# OhMyDesk 客户端配置 —— 默认连接演示中转；内网部署可改为 ws://<服务端IP>:8765/ws。
OHMYDESK_SERVER=wss://rc.guoziweb.com/ws
# 演示占位帧：真实截屏不可用的环境（如 WSL）设 1，用合成帧验证「授权→画面→断开」链路；
# 真机信创 X11 留空走真实屏幕。
OHMYDESK_FAKE_CAPTURE=
EOF

# 桌面菜单项
cat > "$STAGE/usr/share/applications/ohmydesk-client.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=OhMyDesk 终端
Comment=信创内网终端远程安全管控 Agent
Exec=/usr/bin/ohmydesk-client-launch
Icon=utilities-terminal
Terminal=false
Categories=Network;RemoteAccess;System;
EOF

# 文档
cp -f README.md "$STAGE/usr/share/doc/$PKG/README.md" 2>/dev/null || true

# ── 依赖推导（ldd → dpkg -S → 包名去重）──────────────────────────────────────
# 交叉构建时宿主 ldd 读不了异构二进制（或包名属于宿主发行版），故允许 CI 直接注入 DEPS。
if [ -z "${DEPS:-}" ]; then
  DEPS="$(ldd "$BIN" 2>/dev/null | awk '/=>/{print $3}' | sort -u \
    | xargs -r dpkg -S 2>/dev/null | sed -E 's/:.*//' | sort -u \
    | grep -vE '^$' | paste -sd ', ' || true)"
fi
[ -z "$DEPS" ] && DEPS="libc6, libxcb1, libfontconfig1, libxkbcommon0, libx11-6, libxi6, libxtst6, libxrandr2"

# ── 控制文件 ────────────────────────────────────────────────────────────────
cat > "$STAGE/DEBIAN/control" <<EOF
Package: $PKG
Version: $VERSION
Section: net
Priority: optional
Architecture: $ARCH
Depends: $DEPS
Maintainer: OhMyDesk <chin@guoziweb.com>
Description: 信创内网终端远程安全管控 Agent
 终端侧 Agent：反连中转服务端注册、上报硬件资产与心跳，
 支持被控/主控远程控制与批量截图。需 X11 会话（信创机推荐物理 X11 桌面）。
EOF

echo "/etc/ohmydesk/client.env" > "$STAGE/DEBIAN/conffiles"

cat > "$STAGE/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
command -v update-desktop-database >/dev/null 2>&1 && \
  update-desktop-database -q /usr/share/applications 2>/dev/null || true
echo "OhMyDesk 客户端已安装。"
echo "① 默认连接 wss://rc.guoziweb.com/ws；内网部署可编辑 /etc/ohmydesk/client.env"
echo "② 启动：命令行 ohmydesk-client-launch  或 应用菜单「OhMyDesk 终端」"
exit 0
EOF
chmod 755 "$STAGE/DEBIAN/postinst"

# ── 打包 ────────────────────────────────────────────────────────────────────
OUT="$REPO_ROOT/dist/linux/${PKG}_${VERSION}_${ARCH}.deb"
install -d "$REPO_ROOT/dist/linux"
dpkg-deb --build --root-owner-group "$STAGE" "$OUT"

echo ""
echo "✓ 已生成：$OUT"
dpkg-deb --info "$OUT" | sed -n '1,20p'
echo ""
echo "安装：sudo dpkg -i $OUT  （缺依赖则 sudo apt-get -f install）"
