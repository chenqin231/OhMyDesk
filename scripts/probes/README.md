# 集成探针（node，无依赖）

用 node 内置 WebSocket 模拟 Agent/admin 多端，对真实 server 跑端到端链路验证。需先在 `127.0.0.1:8765` 起 server。

```bash
# 起 server（仓库根）
DATABASE_URL="mysql://root:ohmydesk@127.0.0.1:13307/ohmydesk" \
OHMYDESK_WEB_DIR="src/admin-web/dist" cargo run -p server

# 综合闭环：I2 远控（授权→键鼠→帧双向）+ I3 双端批量截图 + 会话结束
node scripts/probes/integration.mjs

# 拒连双分支：模式 B 密码错 / 被控主动拒绝
node scripts/probes/reject.mjs
```

- `integration.mjs`：两被控 agent（信创麒麟龙芯 / 非信创 x86）+ 一 admin 主控，验证 ConnectAck、Input/Frame 路由、`screenshot_resp` 按 `endpoint_id` 归位；跑完可 `curl /api/audit` `/api/sessions` 看审计落库。
- `reject.mjs`：验证 `reject「密码错误」` 与 `reject「用户拒绝」` 两条拒因分支。
