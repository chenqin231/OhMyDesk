// D-6~D-8 适配：AuditLog[] + Session[] → AuditRecord[]（最重适配）
import type { AuditLog } from "@/lib/types/AuditLog";
import type { AuditType } from "@/lib/types/AuditType";
import type { Session } from "@/lib/types/Session";

// 时间线条目（不含 O-1 裁决砍掉的 transfer 类型）
export type TimelineItem = {
  ts: number;              // 转为 number 用于排序和展示
  kind: AuditType;
  text: string;
};

// UI 层期望的聚合会话视图
export type AuditRecord = {
  sessionId: string;
  actor: string;
  target: string;
  mode: "A" | "B";         // D-6 Mode 小写 → 大写展示
  result: "active" | "success" | "rejected" | "auth_failed";  // D-8 补进行中态
  startText: string;
  durationText: string;
  summary: string;         // "截图 2 次，输入操作 47 次"
  timeline: TimelineItem[];
};

// D-6 Mode 展示标签
export const MODE_LABEL: Record<"A" | "B", string> = {
  A: "A 管理端→终端",
  B: "B 终端→终端",
};

// result 展示标签（含 active）
export const RESULT_LABEL: Record<AuditRecord["result"], string> = {
  active: "进行中",
  success: "成功",
  rejected: "拒绝",
  auth_failed: "鉴权失败",
};

function fmtTime(epochSec: bigint): string {
  return new Date(Number(epochSec) * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function fmtDur(sec: number): string {
  if (sec <= 0) return "—";
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

export function summarize(items: TimelineItem[]): string {
  const count = (kind: AuditType) => items.filter((i) => i.kind === kind).length;
  const shots = count("screenshot");
  const inputs = items.filter((i) => i.kind === "input");
  const commands = count("command");
  const files = count("file_transfer");
  const chats = count("chat");
  const parts: string[] = [];
  if (shots > 0) parts.push(`截图 ${shots} 次`);
  if (inputs.length > 0) parts.push(inputs.map((i) => i.text).join("、"));
  if (commands > 0) parts.push(`命令 ${commands} 条`);
  if (files > 0) parts.push(`文件传输 ${files} 次`);
  if (chats > 0) parts.push(`消息 ${chats} 条`);
  return parts.join("，") || "无操作记录";
}

// D-8：从 status + 日志推导 result
function deriveResult(
  s: Session,
  items: TimelineItem[],
): AuditRecord["result"] {
  if (s.status === "active") return "active";
  if (s.status === "ended") return "success";
  // status === "rejected"：看是否有 auth_fail 日志
  const hasAuthFail = items.some((i) => i.kind === "auth_fail");
  return hasAuthFail ? "auth_failed" : "rejected";
}

function groupBy<T>(arr: T[], key: (t: T) => string): Record<string, T[]> {
  const out: Record<string, T[]> = {};
  for (const item of arr) {
    const k = key(item);
    (out[k] ??= []).push(item);
  }
  return out;
}

// 事件流 → 会话聚合（D-7 按 session_id 聚合）
export function aggregate(logs: AuditLog[], sessions: Session[]): AuditRecord[] {
  const bySession = groupBy(logs, (l) => l.session_id);

  return sessions.map((s) => {
    const items: TimelineItem[] = (bySession[s.id] ?? [])
      .sort((a, b) => Number(a.ts) - Number(b.ts))
      .map((l) => ({ ts: Number(l.ts), kind: l.type, text: l.text }));

    const endSec = s.end_at ? Number(s.end_at) : null;
    const startSec = Number(s.start_at);
    const durSec = endSec !== null ? Math.abs(endSec - startSec) : null;

    return {
      sessionId: s.id,
      // 操作人 = 真实 WEB 登录账号；旧数据（operator_username 为 null）显示「旧版本记录」
      actor: s.operator_username ?? "旧版本记录",
      target: s.to_id,
      mode: s.mode.toUpperCase() as "A" | "B",  // D-6
      result: deriveResult(s, items),             // D-8
      startText: fmtTime(s.start_at),
      durationText: durSec !== null ? fmtDur(durSec) : "进行中",
      summary: summarize(items),
      timeline: items,
    };
  });
}
