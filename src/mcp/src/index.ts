/**
 * OhMyDesk MCP Server 入口
 * Wave 1: mock HTTP，stdio transport
 *
 * 模块分工：
 *   types.ts — 领域类型（对齐 ts-rs 生成物）
 *   mock.ts  — Wave 1 mock 数据 + toJson；Wave 2 换真实 fetch client
 *   tools.ts — 4 个只读 tool 定义与 handler
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { registerTools } from "./tools.js";

const server = new McpServer({ name: "ohmydesk", version: "0.1.0" });

registerTools(server);

const transport = new StdioServerTransport();
await server.connect(transport);
