#!/usr/bin/env bash
# 从 Linux/WSL 交叉编译 OhMyDesk 客户端为 Windows exe（x86_64-pc-windows-gnu）。
# 产出 dist-windows/：ohmydesk-client.exe + 所需 mingw 运行时 DLL + 一键启动 .bat。
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

echo "==> 3/5 收拢产物到 dist-windows/"
rm -rf "$DIST"
mkdir -p "$DIST"
cp "$EXE_SRC" "$DIST/ohmydesk-client.exe"

echo "==> 4/5 兜底打包 mingw 运行时 DLL（若静态链接已消除依赖则跳过）"
needed_dlls="$("$OBJDUMP" -p "$DIST/ohmydesk-client.exe" 2>/dev/null \
  | awk '/DLL Name:/ {print $3}' | grep -iE '^lib(gcc|stdc|winpthread)' || true)"
if [ -z "$needed_dlls" ]; then
  echo "    无 mingw DLL 依赖（已独立 exe）"
else
  for dll in $needed_dlls; do
    found="$(find /usr/lib/gcc/x86_64-w64-mingw32 /usr/x86_64-w64-mingw32 -name "$dll" 2>/dev/null | head -1 || true)"
    if [ -n "$found" ]; then
      cp "$found" "$DIST/"; echo "    + $dll"
    else
      echo "    ! 未找到 $dll，运行时可能报缺 DLL" >&2
    fi
  done
fi

echo "==> 5/5 生成一键启动脚本（连接服务器.bat）"
# 写 CRLF 换行的 .bat（Windows 友好）。服务器地址内嵌，用户可记事本改。
{
  printf '@echo off\r\n'
  printf 'chcp 65001 >nul\r\n'
  printf 'rem === OhMyDesk Windows 被控端启动 ===\r\n'
  printf 'rem 改服务器地址改下面这行（明文 ws，需服务端开放对应端口）\r\n'
  printf 'set "OHMYDESK_SERVER=%s"\r\n' "$SERVER_URL"
  printf 'rem 显示名（管理端「使用人」列）：默认 Windows 用户名-主机名，可自定义\r\n'
  printf 'set "OHMYDESK_NAME=%%USERNAME%%-%%COMPUTERNAME%%"\r\n'
  printf 'echo 连接 %%OHMYDESK_SERVER%% 身份 %%OHMYDESK_NAME%% ...\r\n'
  printf '"%%~dp0ohmydesk-client.exe" "%%OHMYDESK_NAME%%"\r\n'
  printf 'echo.\r\n'
  printf 'echo 客户端已退出（窗口可关）。若闪退，上方为错误信息。\r\n'
  printf 'pause\r\n'
} > "$DIST/连接服务器.bat"

SIZE="$(du -h "$DIST/ohmydesk-client.exe" | cut -f1)"
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Windows 客户端构建完成"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  产物目录：dist-windows/"
echo "    ohmydesk-client.exe   ($SIZE)"
echo "    连接服务器.bat        （双击启动，已内嵌 $SERVER_URL）"
ls "$DIST"/*.dll >/dev/null 2>&1 && echo "    *.dll                 （mingw 运行时，需与 exe 同目录）"
echo
echo "  用法：把整个 dist-windows/ 拷到 Windows 机器 → 双击「连接服务器.bat」"
echo "  即注册为被控端，可在管理后台远控该 Windows。"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
