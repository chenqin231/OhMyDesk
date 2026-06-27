import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Monitor, MoreHorizontal, RefreshCw, Search, Terminal as TerminalIcon, Trash2 } from "lucide-react";

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
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useStore } from "@/store";
import { endpointToRow, type TerminalRow } from "@/lib/adapters/endpoint";
import type { OsKind } from "@/lib/types/OsKind";
import { ArchBadge, MemoryBar, OsCell, StatusBadge } from "@/components/assets/terminal-cells";
import { TerminalDetailSheet } from "@/components/assets/terminal-detail-sheet";

type StatusFilter = "online" | "offline" | "all";
type OsFilter = OsKind | "all";

const STATUS_LABELS: Record<StatusFilter, string> = {
  all: "全部状态",
  online: "在线",
  offline: "离线",
};

const OS_LABELS: Record<OsFilter, string> = {
  all: "全部系统",
  kylin: "银河麒麟",
  uos: "统信 UOS",
  windows: "Windows",
  linux: "Linux",
  other: "其他",
};

export function TerminalAssets() {
  const endpoints = useStore((s) => s.endpoints);
  const startRemote = useStore((s) => s.startRemote);
  const deleteEndpoints = useStore((s) => s.deleteEndpoints);
  const navigate = useNavigate();
  const [keyword, setKeyword] = useState("");
  const [status, setStatus] = useState<StatusFilter>("all");
  const [os, setOs] = useState<OsFilter>("all");
  const [selected, setSelected] = useState<TerminalRow | null>(null);
  const [open, setOpen] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  const nowSec = Math.floor(Date.now() / 1000);

  const rows = useMemo(() => {
    const kw = keyword.trim().toLowerCase();
    return endpoints
      .map((ep) => endpointToRow(ep, nowSec))
      .filter((t) => {
        if (status !== "all" && t.status !== status) return false;
        if (os !== "all" && t.osKey !== os) return false;
        if (!kw) return true;
        return (
          t.user.toLowerCase().includes(kw) ||
          t.ip.toLowerCase().includes(kw) ||
          t.id.toLowerCase().includes(kw) ||
          t.department.toLowerCase().includes(kw)
        );
      });
  }, [endpoints, keyword, status, os, nowSec]);

  function openDetail(t: TerminalRow) {
    setSelected(t);
    setOpen(true);
  }

  // 发起模式 A 远控：目标取终端 id，跳 /remote 进入「连接中→控制中」会话视图。
  function handleRemote(id: string, name?: string) {
    startRemote("a", id, null, name);
    navigate("/remote");
  }

  // ── 多选 + 删除（清理离线/冗余终端记录）──────────────────────────────
  const allSelected = rows.length > 0 && rows.every((r) => selectedIds.has(r.id));

  function toggleAll() {
    setSelectedIds((prev) => {
      const n = new Set(prev);
      if (allSelected) rows.forEach((r) => n.delete(r.id));
      else rows.forEach((r) => n.add(r.id));
      return n;
    });
  }

  function toggleOne(id: string) {
    setSelectedIds((prev) => {
      const n = new Set(prev);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  }

  async function handleDelete(ids: string[]) {
    if (!ids.length) return;
    if (!window.confirm(`确定删除选中的 ${ids.length} 台终端记录？仅清理资产列表，不影响会话/审计历史。`)) return;
    await deleteEndpoints(ids);
    setSelectedIds((prev) => {
      const n = new Set(prev);
      ids.forEach((id) => n.delete(id));
      return n;
    });
  }

  return (
    <div className="flex flex-col gap-4">
      {/* 操作栏 */}
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <div className="relative w-full sm:max-w-xs">
          <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={keyword}
            onChange={(e) => setKeyword(e.target.value)}
            placeholder="搜索使用人 / IP / 终端编号"
            className="pl-9"
          />
        </div>

        <div className="flex items-center gap-2 sm:ml-auto">
          <Select value={status} onValueChange={(v) => setStatus(v as StatusFilter)}>
            <SelectTrigger className="w-32">
              <SelectValue>{STATUS_LABELS[status]}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">全部状态</SelectItem>
              <SelectItem value="online">在线</SelectItem>
              <SelectItem value="offline">离线</SelectItem>
            </SelectContent>
          </Select>

          <Select value={os} onValueChange={(v) => setOs(v as OsFilter)}>
            <SelectTrigger className="w-36">
              <SelectValue>{OS_LABELS[os]}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">全部系统</SelectItem>
              <SelectItem value="kylin">银河麒麟</SelectItem>
              <SelectItem value="uos">统信 UOS</SelectItem>
              <SelectItem value="windows">Windows</SelectItem>
            </SelectContent>
          </Select>

          <Button variant="outline" size="icon" aria-label="刷新">
            <RefreshCw className="size-4" />
          </Button>
        </div>
      </div>

      {/* 批量操作栏：选中 ≥1 台时出现 */}
      {selectedIds.size > 0 && (
        <div className="flex items-center gap-3 rounded-lg border border-border bg-muted/40 px-4 py-2">
          <span className="text-sm text-foreground">
            已选 <span className="font-mono font-semibold">{selectedIds.size}</span> 台终端
          </span>
          <Button
            size="sm"
            variant="destructive"
            className="h-8 gap-1.5"
            onClick={() => handleDelete([...selectedIds])}
          >
            <Trash2 className="size-3.5" />
            批量删除
          </Button>
          <Button size="sm" variant="ghost" className="h-8" onClick={() => setSelectedIds(new Set())}>
            取消选择
          </Button>
        </div>
      )}

      {/* 数据表格 */}
      <div className="overflow-hidden rounded-lg border border-border bg-card">
        <Table>
          <TableHeader>
            <TableRow className="border-border hover:bg-transparent">
              <TableHead className="w-10">
                <input
                  type="checkbox"
                  className="size-4 cursor-pointer accent-primary align-middle"
                  aria-label="全选"
                  checked={allSelected}
                  onChange={toggleAll}
                />
              </TableHead>
              <TableHead className="w-24">状态</TableHead>
              <TableHead>使用人</TableHead>
              <TableHead>IP 地址</TableHead>
              <TableHead>MAC 地址</TableHead>
              <TableHead>操作系统</TableHead>
              <TableHead>CPU 架构</TableHead>
              <TableHead>CPU 型号</TableHead>
              <TableHead>内存占用</TableHead>
              <TableHead>最后在线</TableHead>
              <TableHead className="text-right">操作</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((t) => (
              <TableRow
                key={t.id}
                onClick={() => openDetail(t)}
                className="cursor-pointer border-border"
              >
                <TableCell onClick={(e) => e.stopPropagation()}>
                  <input
                    type="checkbox"
                    className="size-4 cursor-pointer accent-primary align-middle"
                    aria-label={`选择 ${t.user}`}
                    checked={selectedIds.has(t.id)}
                    onChange={() => toggleOne(t.id)}
                  />
                </TableCell>
                <TableCell>
                  <StatusBadge status={t.status} />
                </TableCell>
                <TableCell>
                  <div className="flex flex-col">
                    <span className="text-sm text-foreground">{t.user}</span>
                    <span className="font-mono text-[11px] text-muted-foreground">{t.id}</span>
                  </div>
                </TableCell>
                <TableCell className="font-mono text-sm text-foreground">{t.ip}</TableCell>
                <TableCell className="font-mono text-sm text-muted-foreground">{t.mac}</TableCell>
                <TableCell>
                  <OsCell osKey={t.osKey} osName={t.osName} />
                </TableCell>
                <TableCell>
                  <ArchBadge arch={t.arch} />
                </TableCell>
                <TableCell className="text-sm text-muted-foreground">{t.cpuModel}</TableCell>
                <TableCell>
                  <MemoryBar used={t.memUsedGb} total={t.memTotalGb} />
                </TableCell>
                <TableCell className="text-sm text-muted-foreground">{t.lastSeenText}</TableCell>
                <TableCell onClick={(e) => e.stopPropagation()} className="text-right">
                  <div className="flex items-center justify-end gap-1">
                    <Button
                      size="sm"
                      disabled={t.status === "offline"}
                      className="h-8 gap-1.5"
                      onClick={() => handleRemote(t.id, t.user)}
                    >
                      <TerminalIcon className="size-3.5" />
                      远程控制
                    </Button>
                    <DropdownMenu>
                      <DropdownMenuTrigger
                        render={
                          <Button variant="ghost" size="icon" className="size-8" aria-label="更多操作">
                            <MoreHorizontal className="size-4" />
                          </Button>
                        }
                      />
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem onClick={() => openDetail(t)}>
                          <Monitor className="size-4" />
                          查看硬件画像
                        </DropdownMenuItem>
                        <DropdownMenuItem>重置连接密码</DropdownMenuItem>
                        <DropdownMenuItem>采集系统日志</DropdownMenuItem>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          variant="destructive"
                          onClick={() => handleDelete([t.id])}
                        >
                          <Trash2 className="size-4" />
                          删除该终端
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                </TableCell>
              </TableRow>
            ))}
            {rows.length === 0 && (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={11} className="h-32 text-center text-sm text-muted-foreground">
                  {endpoints.length === 0 ? "正在加载终端列表…" : "未找到匹配的终端"}
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </div>

      {/* 统计脚注 */}
      <p className="text-xs text-muted-foreground">
        共 <span className="font-mono text-foreground">{rows.length}</span> 台终端
        {(status !== "all" || os !== "all" || keyword) && <span>（已筛选）</span>}
      </p>

      <TerminalDetailSheet
        terminal={selected}
        open={open}
        onOpenChange={setOpen}
        onRemoteControl={(id) => handleRemote(id, selected?.user)}
      />
    </div>
  );
}
