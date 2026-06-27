/**
 * 4 个 P0 只读 tool 的定义与 handler
 * 导出 registerTools(server) 供 index.ts 调用
 *
 * SDK 1.29.0 注册签名：
 *   server.registerTool(name, { title, description, inputSchema: ZodRawShape }, callback)
 *   inputSchema 传 shape 对象（Record<string, ZodSchema>），不是 z.object()
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { MOCK_AUDIT_LOGS, MOCK_ENDPOINTS, MOCK_SESSIONS, toJson } from "./mock.js";

export function registerTools(server: McpServer): void {
  // ── Tool 1: list_endpoints ──────────────────────────────────────────────
  // 对应 GET /api/endpoints → EndpointView[]
  server.registerTool(
    "list_endpoints",
    {
      title: "列出终端",
      description: "列出所有受管终端（可按在线状态和操作系统过滤）。返回 EndpointView 数组。",
      inputSchema: {
        online: z.boolean().optional().describe("true=仅在线，false=仅离线，省略=全部"),
        os: z.string().optional().describe("OS 类型关键词过滤，如 kylin / uos / windows"),
      },
    },
    async (args) => {
      // Wave 2: 替换为 fetch(`${BASE_URL}/api/endpoints`)
      let result = MOCK_ENDPOINTS;

      if (args.online !== undefined) {
        result = result.filter((e) => e.online === args.online);
      }
      if (args.os) {
        const osLower = args.os.toLowerCase();
        result = result.filter(
          (e) =>
            e.info.os.kind.includes(osLower) ||
            e.info.os.name.toLowerCase().includes(osLower),
        );
      }

      return { content: [{ type: "text", text: toJson(result) }] };
    },
  );

  // ── Tool 2: get_active_sessions ─────────────────────────────────────────
  // 对应 GET /api/sessions → 进行中会话
  server.registerTool(
    "get_active_sessions",
    {
      title: "获取活跃会话",
      description: "列出当前所有进行中（status=active）的远控会话。返回 Session 数组。",
      inputSchema: {},
    },
    async (_args) => {
      // Wave 2: 替换为 fetch(`${BASE_URL}/api/sessions`)
      const result = MOCK_SESSIONS.filter((s) => s.status === "active");
      return { content: [{ type: "text", text: toJson(result) }] };
    },
  );

  // ── Tool 3: query_audit_log ─────────────────────────────────────────────
  // 对应 GET /api/audit?endpoint=&from=&to=&result= → AuditLog[]
  server.registerTool(
    "query_audit_log",
    {
      title: "查询审计日志",
      description: "查询操作审计日志，支持按终端ID、时间范围、审计类型过滤。返回 AuditLog 数组。",
      inputSchema: {
        endpoint: z.string().optional().describe("终端 ID（精确匹配 session 关联的终端）"),
        from: z.string().optional().describe("起始时间 ISO 8601，如 2026-06-27T00:00:00Z"),
        to: z.string().optional().describe("结束时间 ISO 8601，如 2026-06-27T23:59:59Z"),
        result: z
          .enum(["connect", "auth_fail", "reject", "screenshot", "input", "disconnect"])
          .optional()
          .describe("审计类型过滤"),
      },
    },
    async (args) => {
      // Wave 2: 替换为 fetch(`${BASE_URL}/api/audit?${new URLSearchParams(...)}`)
      let logs = MOCK_AUDIT_LOGS;

      if (args.from) {
        const fromTs = BigInt(new Date(args.from).getTime());
        logs = logs.filter((l) => l.ts >= fromTs);
      }
      if (args.to) {
        const toTs = BigInt(new Date(args.to).getTime());
        logs = logs.filter((l) => l.ts <= toTs);
      }
      if (args.result) {
        logs = logs.filter((l) => l.type === args.result);
      }
      // endpoint 过滤需通过 session_id 关联，Wave 1 简化为文本匹配
      if (args.endpoint) {
        logs = logs.filter((l) => l.session_id.includes(args.endpoint as string));
      }

      return { content: [{ type: "text", text: toJson(logs) }] };
    },
  );

  // ── Tool 4: get_endpoint_stats ──────────────────────────────────────────
  // 聚合统计：在线率 + 信创 OS 分布 + CPU 架构分布
  server.registerTool(
    "get_endpoint_stats",
    {
      title: "终端统计",
      description: "汇总终端在线率、信创 OS 分布（麒麟/统信/龙芯等）和架构分布。",
      inputSchema: {},
    },
    async (_args) => {
      // Wave 2: 基于 fetch(`${BASE_URL}/api/endpoints`) 数据聚合
      const endpoints = MOCK_ENDPOINTS;
      const total = endpoints.length;
      const online = endpoints.filter((e) => e.online).length;

      const osDist: Record<string, number> = {};
      const archDist: Record<string, number> = {};
      const xinchuangDist: Record<string, number> = {};

      for (const e of endpoints) {
        const osKind = e.info.os.kind;
        osDist[osKind] = (osDist[osKind] ?? 0) + 1;

        const arch = e.info.cpu.arch;
        archDist[arch] = (archDist[arch] ?? 0) + 1;

        const tag = e.xinchuang;
        xinchuangDist[tag] = (xinchuangDist[tag] ?? 0) + 1;
      }

      const stats = {
        total,
        online,
        offline: total - online,
        online_rate: total > 0 ? `${((online / total) * 100).toFixed(1)}%` : "0%",
        os_distribution: osDist,
        cpu_arch_distribution: archDist,
        xinchuang_distribution: xinchuangDist,
      };

      return { content: [{ type: "text", text: toJson(stats) }] };
    },
  );
}
