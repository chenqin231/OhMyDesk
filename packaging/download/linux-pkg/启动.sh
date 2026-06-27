#!/bin/sh
# OhMyDesk 被控端启动器（信创内网）。
# 默认连接 wss://rc.guoziweb.com/ws，自动以 9 位码注册为被控端。
# 需在 X11 桌面会话下运行（xcap/enigo 在 Wayland 不可靠）。
DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR" || exit 1
chmod +x ./ohmydesk-client 2>/dev/null
exec ./ohmydesk-client "$(hostname 2>/dev/null || echo 信创终端)"
