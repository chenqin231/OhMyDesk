OhMyDesk 被控端客户端（Linux x86_64）
=====================================

作用：把本机注册为 OhMyDesk 被控端，管理员可在管理后台
（https://rc.guoziweb.com）远程控制本机。默认连接
wss://rc.guoziweb.com/ws，无需任何配置。

运行方式
--------
1. 解压后进入目录：
     cd ohmydesk-client
2. 启动：
     ./启动.sh
   或直接运行二进制：
     chmod +x ohmydesk-client && ./ohmydesk-client

环境要求
--------
- X11 桌面会话（麒麟 / 统信 默认为 X11；若为 Wayland 请切换到 X11 登录）。
- 适用 Ubuntu 22.04+ / 麒麟 V10 / 统信 UOS V20 等 x86_64 桌面发行版。

启动后窗口会显示「本机 ID（9 位）」与「临时密码」，
把它们报给管理员，即可被远程控制。

修改服务器地址（可选）
--------------------
  OHMYDESK_SERVER=wss://你的服务器/ws ./ohmydesk-client
