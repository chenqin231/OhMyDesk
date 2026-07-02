import { useMemo, useState } from "react";
import { CheckCircle2, RefreshCw, Search, ShieldX, Slash } from "lucide-react";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useStore } from "@/store";
import { aggregate, type AuditRecord } from "@/lib/adapters/audit";
import { InitiatorCell, ModeBadge, ResultBadge, TargetCell } from "@/components/audit/audit-cells";
import { AuditTimelineSheet } from "@/components/audit/audit-timeline-sheet";

type ResultFilter = AuditRecord["result"] | "all";
// G-4：时间范围筛选 today/3d/7d 落到 from/to 参数
type RangeFilter = "all" | "today" | "3d" | "7d";

const RESULT_LABELS: Record<ResultFilter, string> = {
  all: "全部结果",
  active: "进行中",
  success: "成功",
  rejected: "拒绝",
  auth_failed: "鉴权失败",
};

const RANGE_LABELS: Record<RangeFilter, string> = {
  all: "全部时间",
  today: "今天",
  "3d": "近 3 天",
  "7d": "近 7 天",
};

// G-4：时间范围 → from/to 参数（秒级 epoch）
function rangeToFromTo(range: RangeFilter): { from?: number; to?: number } {
  const now = Math.floor(Date.now() / 1000);
  if (range === "today") {
    const startOfDay = new Date();
    startOfDay.setHours(0, 0, 0, 0);
    return { from: Math.floor(startOfDay.getTime() / 1000), to: now };
  }
  if (range === "3d") return { from: now - 3 * 86400, to: now };
  if (range === "7d") return { from: now - 7 * 86400, to: now };
  return {};
}

// 概览统计小卡
function StatCard({
  label,
  value,
  icon,
  tone,
}: {
  label: string;
  value: number;
  icon: React.ReactNode;
  tone: "default" | "online" | "muted" | "warning";
}) {
  const toneClass = {
    default: "text-primary",
    online: "text-online",
    muted: "text-muted-foreground",
    warning: "text-warning",
  }[tone];
  return (
    <div className="flex items-center gap-3 rounded-lg border border-border bg-card px-4 py-3">
      <span className={cn("flex size-9 items-center justify-center rounded-md bg-secondary", toneClass)}>
        {icon}
      </span>
      <div className="flex flex-col">
        <span className="font-mono text-xl font-semibold text-foreground">{value}</span>
        <span className="text-xs text-muted-foreground">{label}</span>
      </div>
    </div>
  );
}

export function AuditLog() {
  const auditLogs = useStore((s) => s.auditLogs);
  const sessions = useStore((s) => s.sessions);
  const fetchAudit = useStore((s) => s.fetchAudit);

  const [keyword, setKeyword] = useState("");
  const [range, setRange] = useState<RangeFilter>("all");
  const [resultFilter, setResultFilter] = useState<ResultFilter>("all");
  const [selected, setSelected] = useState<AuditRecord | null>(null);
  const [open, setOpen] = useState(false);

  // G-4：时间筛选下发给 fetchAudit
  function handleRangeChange(v: RangeFilter) {
    setRange(v);
    const { from, to } = rangeToFromTo(v);
    void fetchAudit(from, to);
  }

  // 前端二次筛选（关键字 + result）
  const records = useMemo(() => aggregate(auditLogs, sessions), [auditLogs, sessions]);

  const rows = useMemo(() => {
    const kw = keyword.trim().toLowerCase();
    return records.filter((r) => {
      if (resultFilter !== "all" && r.result !== resultFilter) return false;
      if (!kw) return true;
      return (
        r.actor.toLowerCase().includes(kw) ||
        r.target.toLowerCase().includes(kw) ||
        r.sessionId.toLowerCase().includes(kw)
      );
    });
  }, [records, keyword, resultFilter]);

  const stats = useMemo(() => ({
    total: records.length,
    success: records.filter((r) => r.result === "success").length,
    rejected: records.filter((r) => r.result === "rejected").length,
    authFailed: records.filter((r) => r.result === "auth_failed").length,
  }), [records]);

  function openTimeline(r: AuditRecord) {
    setSelected(r);
    setOpen(true);
  }

  return (
    <div className="flex flex-col gap-4">
      <p className="text-xs leading-relaxed text-muted-foreground">
        所有远程控制会话均可追溯：谁在何时控制了哪台终端、做了哪些操作。审计采用纯文本记录，不录像。
      </p>

      {/* 统计概览 */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <StatCard label="会话总数" value={stats.total} icon={<Search className="size-4" />} tone="default" />
        <StatCard label="成功" value={stats.success} icon={<CheckCircle2 className="size-4" />} tone="online" />
        <StatCard label="拒绝" value={stats.rejected} icon={<Slash className="size-4" />} tone="muted" />
        <StatCard label="鉴权失败" value={stats.authFailed} icon={<ShieldX className="size-4" />} tone="warning" />
      </div>

      {/* 筛选栏 */}
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center">
        <div className="relative w-full lg:max-w-xs">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="搜索操作人 / 终端 / 会话号"
            className="pl-9"
          />
        </div>

        <div className="flex flex-wrap items-center gap-2 lg:ml-auto">
          {/* G-4：时间范围下拉 */}
          <Select value={range} onValueChange={(v) => handleRangeChange(v as RangeFilter)}>
            <SelectTrigger className="w-32">
              <SelectValue>{RANGE_LABELS[range]}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">全部时间</SelectItem>
              <SelectItem value="today">今天</SelectItem>
              <SelectItem value="3d">近 3 天</SelectItem>
              <SelectItem value="7d">近 7 天</SelectItem>
            </SelectContent>
          </Select>

          <Select value={resultFilter} onValueChange={(v) => setResultFilter(v as ResultFilter)}>
            <SelectTrigger className="w-32">
              <SelectValue>{RESULT_LABELS[resultFilter]}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">全部结果</SelectItem>
              <SelectItem value="active">进行中</SelectItem>
              <SelectItem value="success">成功</SelectItem>
              <SelectItem value="rejected">拒绝</SelectItem>
              <SelectItem value="auth_failed">鉴权失败</SelectItem>
            </SelectContent>
          </Select>

          <Button variant="outline" size="icon" aria-label="刷新" onClick={() => void fetchAudit()}>
            <RefreshCw className="size-4" />
          </Button>
        </div>
      </div>

      {/* 审计记录表格 */}
      <div className="overflow-hidden rounded-lg border border-border bg-card">
        <Table>
          <TableHeader>
            <TableRow className="border-border hover:bg-transparent">
              <TableHead className="w-44">时间</TableHead>
              <TableHead>操作人</TableHead>
              <TableHead>目标终端</TableHead>
              <TableHead>模式</TableHead>
              <TableHead>结果</TableHead>
              <TableHead className="w-20">时长</TableHead>
              <TableHead>操作摘要</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((r) => (
              <TableRow
                key={r.sessionId}
                onClick={() => openTimeline(r)}
                className="cursor-pointer border-border"
              >
                <TableCell className="font-mono text-xs text-muted-foreground">{r.startText}</TableCell>
                <TableCell>
                  <InitiatorCell name={r.actor} />
                </TableCell>
                <TableCell>
                  <TargetCell id={r.target} user={r.target} />
                </TableCell>
                <TableCell>
                  <ModeBadge mode={r.mode} />
                </TableCell>
                <TableCell>
                  <ResultBadge result={r.result} />
                </TableCell>
                <TableCell className="font-mono text-sm text-foreground">{r.durationText}</TableCell>
                <TableCell className="max-w-xs">
                  <span
                    className={cn(
                      "text-sm",
                      r.result === "auth_failed" ? "text-warning" : "text-muted-foreground",
                    )}
                  >
                    {r.summary}
                  </span>
                </TableCell>
              </TableRow>
            ))}
            {rows.length === 0 && (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={7} className="h-32 text-center text-sm text-muted-foreground">
                  未找到匹配的审计记录
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </div>

      <p className="text-xs text-muted-foreground">
        共 <span className="font-mono text-foreground">{rows.length}</span> 条审计记录
        {(resultFilter !== "all" || range !== "all" || keyword) && <span>（已筛选）</span>}
      </p>

      <AuditTimelineSheet record={selected} open={open} onOpenChange={setOpen} />
    </div>
  );
}
