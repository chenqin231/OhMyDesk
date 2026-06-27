/**
 * OhMyDesk MCP Server 入口
 * 真实 HTTP（对接 server 只读 API），stdio transport
 *
 * 模块分工：
 *   types.ts  — 领域类型（对齐 ts-rs 生成物）
 *   client.ts — fetch 层 + toJson（GET /api/endpoints|sessions|audit）
 *   tools.ts  — 4 个只读 tool 定义与 handler
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { registerTools } from "./tools.js";

const server = new McpServer({ name: "ohmydesk", version: "0.1.0" });

registerTools(server);

const transport = new StdioServerTransport();
await server.connect(transport);
