// AI 安全助手 —— 消息模型与基于 store 数据的 mock 应答逻辑
// G-5：从 store.endpoints/auditLogs 派生，提供真实数据问答；降级脚本作为兜底

import type { EndpointView } from "@/lib/types/EndpointView";
import type { AuditLog } from "@/lib/types/AuditLog";
import { endpointToRow, OS_LABEL } from "@/lib/adapters/endpoint";

// 结构化内容块
export type AnswerBlock =
  | { type: "text"; text: string }
  | { type: "stat"; value: string; label: string }
  | { type: "table"; columns: string[]; rows: string[][]; mono?: number[] };

// 工具调用信息（体现 MCP 工具调用）
export type ToolCall = {
  name: string;
  args: string;
};

export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  text?: string;
  tool?: ToolCall;
  blocks?: AnswerBlock[];
};

// 预置示例问题
export const sampleQuestions = [
  "现在有几台麒麟终端在线？",
  "今天有哪些远程连接记录？",
  "谁在控制财务部的电脑？",
] as const;

let seq = 0;
const nextId = () => `msg-${Date.now()}-${seq++}`;

export function userMessage(text: string): ChatMessage {
  return { id: nextId(), role: "user", text };
}

// G-5：接收 store 数据，基于真实数据计算回答；无法匹配则降级到预录脚本
export function buildAnswer(
  question: string,
  endpoints: EndpointView[],
  auditLogs: AuditLog[],
): ChatMessage {
  const q = question.trim();
  const nowSec = Math.floor(Date.now() / 1000);
  const rows = endpoints.map((ep) => endpointToRow(ep, nowSec));

  // 1) 麒麟在线终端
  if (q.includes("麒麟")) {
    const list = rows.filter((r) => r.osKey === "kylin" && r.status === "online");
    return {
      id: nextId(),
      role: "assistant",
      tool: { name: "list_endpoints", args: "os=麒麟, online=true" },
      blocks: [
        { type: "text", text: `当前共有 ${list.length} 台银河麒麟终端在线，明细如下：` },
        {
          type: "table",
          columns: ["终端ID", "IP 地址", "使用人", "CPU 架构"],
          rows: list.map((t) => [t.id, t.ip, t.user, t.arch]),
          mono: [1],
        },
        { type: "text", text: "以上麒麟终端均为国产信创设备，数据通过 MCP 只读接口实时获取。" },
      ],
    };
  }

  // 2) 今天的远程连接记录
  if (q.includes("今天") && (q.includes("连接") || q.includes("记录") || q.includes("远程"))) {
    const todayStart = new Date();
    todayStart.setHours(0, 0, 0, 0);
    const todayStartSec = Math.floor(todayStart.getTime() / 1000);
    const todayLogs = auditLogs.filter((l) => Number(l.ts) >= todayStartSec);
    const sessionIds = [...new Set(todayLogs.map((l) => l.session_id))];
    return {
      id: nextId(),
      role: "assistant",
      tool: { name: "query_sessions", args: `date=${new Date().toLocaleDateString("zh-CN")}` },
      blocks: [
        {
          type: "text",
          text: `今天共有 ${sessionIds.length} 个远程控制会话，${todayLogs.length} 条审计事件：`,
        },
        {
          type: "table",
          columns: ["会话ID", "事件类型", "时间", "描述"],
          rows: todayLogs.slice(0, 10).map((l) => [
            l.session_id,
            l.type,
            new Date(Number(l.ts) * 1000).toLocaleTimeString("zh-CN"),
            l.text,
          ]),
          mono: [2],
        },
        { type: "text", text: "完整时间线可在「会话审计」页查看，所有连接均可追溯。" },
      ],
    };
  }

  // 3) 财务部控制状态
  if (q.includes("财务") || q.includes("谁在控制")) {
    const finance = rows.filter((r) => r.department === "财务部");
    return {
      id: nextId(),
      role: "assistant",
      tool: { name: "get_active_control", args: "department=财务部" },
      blocks: [
        finance.length > 0
          ? { type: "text", text: `财务部共有 ${finance.length} 台终端，状态如下：` }
          : { type: "text", text: "当前财务部没有在线终端。" },
        ...(finance.length > 0
          ? ([{
              type: "table",
              columns: ["终端ID", "使用人", "状态", "系统"],
              rows: finance.map((t) => [t.id, t.user, t.status === "online" ? "在线" : "离线", OS_LABEL[t.osKey]]),
            }] as AnswerBlock[])
          : []),
        { type: "text", text: "如需立即终止某个会话，可在「会话审计」中定位并强制断开。" },
      ],
    };
  }

  // 4) 在线终端统计
  if (q.includes("在线") || q.includes("多少台")) {
    const online = rows.filter((r) => r.status === "online");
    const osByKind = Object.entries(
      online.reduce<Record<string, number>>((acc, r) => {
        acc[OS_LABEL[r.osKey]] = (acc[OS_LABEL[r.osKey]] ?? 0) + 1;
        return acc;
      }, {}),
    );
    return {
      id: nextId(),
      role: "assistant",
      tool: { name: "list_endpoints", args: "online=true" },
      blocks: [
        { type: "stat", value: String(online.length), label: `在线 / 总计 ${rows.length}` },
        {
          type: "table",
          columns: ["操作系统", "在线数"],
          rows: osByKind.map(([os, count]) => [os, String(count)]),
        },
      ],
    };
  }

  // 降级：兜底预录应答
  return {
    id: nextId(),
    role: "assistant",
    tool: { name: "list_endpoints", args: "scope=all" },
    blocks: [
      {
        type: "text",
        text: `我可以基于内网管控数据回答终端态势相关问题。当前共 ${rows.length} 台终端，${rows.filter((r) => r.status === "online").length} 台在线。试试下方的示例问题，或换个说法提问。`,
      },
    ],
  };
}

// 预置的示例对话（首屏展示）
export function initialConversation(
  endpoints: EndpointView[],
  auditLogs: AuditLog[],
): ChatMessage[] {
  const q = sampleQuestions[0];
  return [userMessage(q), buildAnswer(q, endpoints, auditLogs)];
}
