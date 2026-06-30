#!/usr/bin/env bash
# 从 Linux/WSL 交叉编译 OhMyDesk 客户端为 Windows exe（x86_64-pc-windows-gnu）。
# 产出 dist-windows/ohmydesk-client.exe —— 单文件独立 exe（静态链接，无 DLL、无启动脚本）。
# 用户直接双击该 exe 即可：默认连 wss://rc.guoziweb.com/ws，显示名兜底取 USERNAME@主机名。
#
# 用法：
#   bash packaging/windows/build-windows.sh                 # 默认连 wss://rc.guoziweb.com/ws
#   OHMYDESK_SERVER="ws://192.168.1.10:8765/ws" bash packaging/windows/build-windows.sh
#
# 前置（脚本会自检并给出安装指引）：
#   rustup target add x86_64-pc-windows-gnu
#   sudo apt-get install -y mingw-w64
set -euo pipefail

TARGET="x86_64-pc-windows-gnu"
LINKER="x86_64-w64-mingw32-gcc"
OBJDUMP="x86_64-w64-mingw32-objdump"
# 默认服务器地址（明文 ws，客户端未编 TLS）。可用环境变量覆盖，会写进启动 .bat。
SERVER_URL="${OHMYDESK_SERVER:-wss://rc.guoziweb.com/ws}"

# 仓库根 = 本脚本所在目录的上两级
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
DIST="$ROOT/dist-windows"

echo "==> 1/5 前置自检"
command -v cargo >/dev/null || { echo "缺 cargo，先装 Rust 工具链"; exit 1; }
if ! rustup target list --installed | grep -q "^${TARGET}$"; then
  echo "    添加 Rust 目标 ${TARGET} ..."
  rustup target add "${TARGET}"
fi
if ! command -v "${LINKER}" >/dev/null; then
  echo "缺交叉链接器 ${LINKER}。安装：sudo apt-get install -y mingw-w64" >&2
  exit 1
fi

echo "==> 2/5 交叉编译 client（release，目标 ${TARGET}）"
# -static：静态链接 mingw 运行时（libgcc/libstdc++/winpthread），尽量产出独立 exe，少依赖 DLL。
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${LINKER}" \
RUSTFLAGS="${RUSTFLAGS:-} -C link-args=-static" \
  cargo build -p client --release --target "${TARGET}"

EXE_SRC="$ROOT/target/${TARGET}/release/client.exe"
[ -f "$EXE_SRC" ] || { echo "未找到产物 $EXE_SRC" >&2; exit 1; }

echo "==> 3/4 收拢产物到 dist-windows/"
rm -rf "$DIST"
mkdir -p "$DIST"
cp "$EXE_SRC" "$DIST/ohmydesk-client.exe"

echo "==> 4/4 校验单文件独立性（不得有 mingw 运行时 DLL 依赖）"
# exe-only 交付：静态链接后必须无 libgcc/libstdc++/winpthread 依赖，否则单 exe 在目标机缺 DLL 闪退。
needed_dlls="$("$OBJDUMP" -p "$DIST/ohmydesk-client.exe" 2>/dev/null \
  | awk '/DLL Name:/ {print $3}' | grep -iE '^lib(gcc|stdc|winpthread)' || true)"
if [ -n "$needed_dlls" ]; then
  echo "✗ exe 仍依赖 mingw DLL，无法交付单文件 exe：" >&2
  echo "$needed_dlls" | sed 's/^/    /' >&2
  echo "  请确认静态链接生效（RUSTFLAGS 含 -C link-args=-static）。" >&2
  exit 1
fi
echo "    ✓ 无 mingw DLL 依赖，单 exe 独立可运行"

SIZE="$(du -h "$DIST/ohmydesk-client.exe" | cut -f1)"
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Windows 客户端构建完成（单文件 exe）"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  产物：dist-windows/ohmydesk-client.exe   ($SIZE)"
echo
echo "  用法：拷到 Windows 机器 → 双击 ohmydesk-client.exe 运行。"
echo "  默认连 $SERVER_URL，显示名兜底 USERNAME@主机名；"
echo "  需改服务器地址：在 exe 同目录用 set OHMYDESK_SERVER=... 后从命令行启动，"
echo "  或仍用环境变量覆盖（OHMYDESK_SERVER=...）。"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
