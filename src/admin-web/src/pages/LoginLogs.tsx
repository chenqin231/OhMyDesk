import { useEffect, useState } from "react";
import { CheckCircle2, RefreshCw, ShieldX } from "lucide-react";

import { AppShell } from "@/components/shell/app-shell";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { transport } from "@/lib/transport";
import type { LoginLogEntry } from "@/lib/types/LoginLogEntry";

function fmtTime(tsSec: number): string {
  return new Date(tsSec * 1000).toLocaleString("zh-CN", { hour12: false });
}

export function LoginLogs() {
  const [rows, setRows] = useState<LoginLogEntry[]>([]);
  const [loading, setLoading] = useState(false);

  async function load() {
    setLoading(true);
    try {
      setRows(await transport.fetchLoginLogs(200, 0));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <AppShell title="登录日志">
      <div className="flex flex-col gap-4">
        <div className="flex items-center justify-between">
          <p className="text-xs leading-relaxed text-muted-foreground">
            记录每次管理员登录尝试的时间、用户名、来源 IP、客户端与结果（成功 / 失败）。
          </p>
          <Button
            variant="outline"
            size="icon"
            aria-label="刷新"
            disabled={loading}
            onClick={() => void load()}
          >
            <RefreshCw className="size-4" />
          </Button>
        </div>

        <div className="overflow-hidden rounded-lg border border-border bg-card">
          <Table>
            <TableHeader>
              <TableRow className="border-border hover:bg-transparent">
                <TableHead className="w-48">时间</TableHead>
                <TableHead className="w-32">用户名</TableHead>
                <TableHead className="w-40">来源 IP</TableHead>
                <TableHead className="w-24">结果</TableHead>
                <TableHead>客户端 / 备注</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((r) => (
                <TableRow key={String(r.id)} className="border-border">
                  <TableCell className="font-mono text-xs text-muted-foreground">
                    {fmtTime(Number(r.ts))}
                  </TableCell>
                  <TableCell className="text-sm">{r.username}</TableCell>
                  <TableCell className="font-mono text-xs">{r.ip ?? "-"}</TableCell>
                  <TableCell>
                    {r.success ? (
                      <span className="inline-flex items-center gap-1 text-online">
                        <CheckCircle2 className="size-4" /> 成功
                      </span>
                    ) : (
                      <span className="inline-flex items-center gap-1 text-warning">
                        <ShieldX className="size-4" /> 失败
                      </span>
                    )}
                  </TableCell>
                  <TableCell className="max-w-md truncate text-xs text-muted-foreground">
                    {r.reason ?? r.user_agent ?? "-"}
                  </TableCell>
                </TableRow>
              ))}
              {rows.length === 0 && (
                <TableRow className="hover:bg-transparent">
                  <TableCell colSpan={5} className="h-32 text-center text-sm text-muted-foreground">
                    暂无登录记录
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </div>

        <p className="text-xs text-muted-foreground">
          共 <span className="font-mono text-foreground">{rows.length}</span> 条登录记录
        </p>
      </div>
    </AppShell>
  );
}
