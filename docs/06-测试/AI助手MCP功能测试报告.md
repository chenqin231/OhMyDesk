# AI 助手 MCP 功能测试报告

> **测试对象**：OhMyDesk「AI 助手」MCP 功能（`src/mcp` stdio MCP Server + server 只读 API 对接）
> **测试环境**：生产服务器 `https://rc.guoziweb.com`
> **测试方式**：官方 `@modelcontextprotocol/sdk` Client 经 stdio 真实启动 `dist/index.js`，走用户 MCP 客户端的完整链路
> **测试日期**：2026-06-28
> **结论**：✅ **通过** — MCP 服务端、stdio 协议、与 server API 的鉴权对接全部正常，4 个 tool 均返回线上真实数据，无 401 降级。

---

## 1. 被测架构

```
用户 MCP 客户端（Claude 等）
   │  stdio (JSON-RPC)
   ▼
node dist/index.js  ──── OHMYDESK_API_BASE / OHMYDESK_API_TOKEN
   │  HTTPS + Bearer JWT
   ▼
OhMyDesk server 只读 API
   /api/endpoints   /api/sessions   /api/audit
```

前端「AI 助手」页（`mcp-config-card.tsx` + `buildMcpConfig`）生成上述配置；MCP Server 暴露 4 个只读 tool，底层即对这三个 API 的 GET 封装（`src/mcp/src/tools.ts`、`client.ts`）。

## 2. 测试步骤与结果

| # | 检查项 | 方法 | 结果 |
|---|--------|------|------|
| 0 | 鉴权前置 | `POST /api/login`（admin） | ✅ 签发 JWT（124 字符）；`/api/me` 回显 `admin` |
| 1 | 无 token 访问 | `GET /api/{endpoints,sessions,audit}` | ✅ 均 `HTTP 401`（鉴权拦截生效） |
| 2 | MCP 握手 | SDK Client `initialize` | ✅ 成功 |
| 3 | 工具发现 | `tools/list` | ✅ 4 个：`list_endpoints` / `get_active_sessions` / `query_audit_log` / `get_endpoint_stats` |
| 4 | `list_endpoints` | `tools/call` 无参 + `online=true` | ✅ 1 台「演示终端」(Windows 10, 在线)；过滤参数生效 |
| 5 | `get_active_sessions` | `tools/call` | ✅ 返回活跃会话数据 |
| 6 | `query_audit_log` | `tools/call` 无参 + `result=input` | ✅ 28 条审计；按类型过滤出 6 条 input 事件 |
| 7 | `get_endpoint_stats` | `tools/call` | ✅ 在线率 100%、OS/CPU 架构/信创分布聚合正确 |

### 关键返回样例

- **list_endpoints**：`演示终端` / Windows 10 (19045) / Intel Xeon E5-2686 v4 / x86_64 / online=true / 非信创
- **get_endpoint_stats**：`total=1 online=1 在线率=100.0%`，`os_distribution={"windows":1}`，`cpu_arch_distribution={"x86_64":1}`，`xinchuang_distribution={"非信创":1}`
- **query_audit_log**：最近事件 `disconnect "会话结束"`

## 3. 判定依据

`list_endpoints` 与 `get_endpoint_stats` 返回**非空真实数据**，证明：
1. token 鉴权通过，未触发 client.ts 中「非 2xx → 返回 `[]`」的优雅降级；
2. stdio MCP 协议帧收发正常，stdout 未被诊断日志污染（诊断仅走 stderr）；
3. server i64/u64 序列化为 JSON number，MCP 侧解析无 BigInt 混合比较异常。

## 4. 旁观发现（非 MCP 缺陷，待跟进）

`get_active_sessions` 返回 **6 个 `active` 会话**，而当前仅 1 台终端在线 → server 侧存在历史会话连上后未收到 `session_end`、滞留为 `active` 的数据。MCP 仅如实反映 server 现状，非 MCP 问题；建议后续排查会话生命周期/超时清理逻辑。

## 5. 复测方法

```bash
# 1) 取 token
TOKEN=$(curl -s -X POST https://rc.guoziweb.com/api/login \
  -H 'Content-Type: application/json' \
  -d '{"user":"admin","pass":"<密码>"}' | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')

# 2) 构建并经 stdio 启动 MCP，指向线上 API + token，调用 4 个 tool
#    （在 src/mcp 目录内运行，确保 node 能解析 node_modules）
cd src/mcp && pnpm build
OHMYDESK_API_BASE=https://rc.guoziweb.com OHMYDESK_API_TOKEN=$TOKEN node dist/index.js
#    再用 MCP 客户端发 initialize / tools/list / tools/call
```
