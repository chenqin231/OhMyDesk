# 端到端测试(协议级)

`run.sh` 构建并启动**真实 server 二进制**(临时 SQLite),用 `protocol-e2e.mjs` 通过**真实 WebSocket** 双客户端跑通完整会话流程,验证四项能力的**会话路由 + 审计落库**:

- 即时消息(A→B 与 B→A 双向)
- 远程命令(ExecRequest)
- 远程文件(FileListRequest)
- 懒推流(SetCapture)
- 审计:chat / command / file_transfer 三类写入 SQLite,经 `/api/audit` 可查

## 运行

```bash
bash scripts/e2e/run.sh
```

预期:`端到端测试全部通过:13 项断言` + `E2E 通过`。

## 前置

- Node ≥ 21(用内置 global `WebSocket`)
- 端口 8765 空闲(脚本起停自带 server,用临时库,不污染 `ohmydesk.db`)

## 不覆盖

Slint GUI 渲染层(远程桌面画面、键鼠注入、四标签交互、被控聊天面板)需 X11 + 两端联动,自动化测不到,由人工验收。
