#!/usr/bin/env bash
# OhMyDesk 协议级端到端测试运行器:构建并启动真实 server(临时 SQLite)→ 跑 protocol-e2e.mjs → 拆除。
# 真实链路验证即时消息/命令/文件/懒推流的路由 + 审计落库。需 Node ≥21(global WebSocket)。
# 用法:bash scripts/e2e/run.sh
set -euo pipefail
cd "$(dirname "$0")/../.."

DB="$(mktemp -u /tmp/ohmydesk-e2e-XXXXXX.db)"
LOG="$(mktemp /tmp/ohmydesk-e2e-srv-XXXXXX.log)"
SECRET="e2e-secret-$$"

echo "== 构建 server =="
cargo build -p server

echo "== 启动 server(临时库 $DB)=="
DATABASE_URL="sqlite:${DB}" OHMYDESK_JWT_SECRET="$SECRET" ./target/debug/server >"$LOG" 2>&1 &
SRV=$!
cleanup() { kill "$SRV" 2>/dev/null || true; rm -f "$DB" "$DB"-* "$LOG"; }
trap cleanup EXIT

# 等待端口就绪(curl 重试,不用 sleep)
curl -s --retry 60 --retry-delay 1 --retry-connrefused -o /dev/null "http://127.0.0.1:8765/api/me" || true

echo "== 运行端到端用例 =="
if node scripts/e2e/protocol-e2e.mjs; then
  echo "E2E 通过"
else
  echo "E2E 失败,server 日志尾部:"; tail -15 "$LOG"; exit 1
fi
