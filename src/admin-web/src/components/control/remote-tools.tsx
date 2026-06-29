import { useEffect, useState } from "react";
import {
  Terminal,
  Upload,
  Download,
  Loader2,
  FolderTree,
  Folder,
  File as FileIcon,
  ArrowUp,
  RefreshCw,
  X,
  ChevronUp,
  ChevronDown,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { useStore } from "@/store";

// 远控会话内的「命令行 + 文件传输」底部停靠面板。挂在远控画面下方，全宽，可折叠。
// 两个标签页：命令行（一次性命令执行）/ 文件传输（左本地暂存区 ↔ 右远端目录浏览）。
// 整个面板仅在 remotePhase==="connected" 时随 RemoteSession 渲染——天然满足「连接成功方可使用」。
const inputCls =
  "min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1.5 font-mono text-xs " +
  "text-foreground placeholder:text-muted-foreground outline-none focus:border-primary";

// 绝对路径分隔符：含反斜杠且不含正斜杠 → Windows。
function sep(path: string): "\\" | "/" {
  return path.includes("\\") && !path.includes("/") ? "\\" : "/";
}
function joinPath(dir: string, name: string): string {
  const s = sep(dir);
  return `${dir.replace(/[\\/]+$/, "")}${s}${name}`;
}
function parentPath(dir: string): string {
  const s = sep(dir);
  const trimmed = dir.replace(/[\\/]+$/, "");
  const idx = trimmed.lastIndexOf(s);
  if (idx <= 0) return s === "\\" ? trimmed.slice(0, idx + 1) : "/";
  return trimmed.slice(0, idx) || (s === "/" ? "/" : trimmed.slice(0, idx + 1));
}
function fmtSize(n: bigint): string {
  const b = Number(n);
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${(b / 1024 / 1024).toFixed(1)} MB`;
}

export function RemoteTools() {
  const [tab, setTab] = useState<"cmd" | "file">("cmd");
  const [collapsed, setCollapsed] = useState(false);

  return (
    <aside className="flex shrink-0 flex-col border-t border-border bg-card">
      {/* 标签头 + 折叠开关 */}
      <div className="flex h-10 shrink-0 items-center justify-between border-b border-border px-2">
        <div className="flex items-center gap-1">
          <TabButton active={tab === "cmd"} onClick={() => setTab("cmd")} icon={<Terminal className="size-3.5" />}>
            命令行
          </TabButton>
          <TabButton active={tab === "file"} onClick={() => setTab("file")} icon={<FolderTree className="size-3.5" />}>
            文件传输
          </TabButton>
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => setCollapsed((c) => !c)}
          title={collapsed ? "展开" : "收起"}
          aria-label={collapsed ? "展开工具面板" : "收起工具面板"}
        >
          {collapsed ? <ChevronUp className="size-4" /> : <ChevronDown className="size-4" />}
        </Button>
      </div>

      {!collapsed && (
        <div className="h-72 min-h-0">
          {tab === "cmd" ? <CommandPanel /> : <FilePanel />}
        </div>
      )}
    </aside>
  );
}

function TabButton({
  active,
  onClick,
  icon,
  children,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={
        "flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors " +
        (active
          ? "bg-primary/10 text-primary"
          : "text-muted-foreground hover:bg-secondary hover:text-foreground")
      }
    >
      {icon}
      {children}
    </button>
  );
}

// ── 命令行标签页 ────────────────────────────────────────────────────────────
function CommandPanel() {
  const execResults = useStore((s) => s.execResults);
  const execCommand = useStore((s) => s.execCommand);
  const [cmd, setCmd] = useState("");

  return (
    <div className="flex h-full flex-col p-3">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          execCommand(cmd);
          setCmd("");
        }}
        className="flex shrink-0 gap-1.5"
      >
        <input
          value={cmd}
          onChange={(e) => setCmd(e.target.value)}
          placeholder="whoami / ipconfig / ls -al"
          className={inputCls}
          spellCheck={false}
          autoComplete="off"
        />
        <Button type="submit" size="sm" disabled={!cmd.trim()}>
          执行
        </Button>
      </form>

      <div className="mt-2 flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto">
        {execResults.length === 0 && (
          <p className="text-xs text-muted-foreground">在被控端系统 shell 中执行一次性命令，回传输出。</p>
        )}
        {execResults.map((e) => (
          <div key={e.exec_id} className="rounded-md border border-border bg-background p-2 text-xs">
            <div className="break-all font-mono text-foreground">$ {e.command}</div>
            {e.pending ? (
              <div className="mt-1 flex items-center gap-1 text-muted-foreground">
                <Loader2 className="size-3 animate-spin" aria-hidden /> 执行中…
              </div>
            ) : (
              <div className="mt-1 flex flex-col gap-1">
                {e.stdout && (
                  <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all font-mono text-foreground">
                    {e.stdout}
                  </pre>
                )}
                {e.stderr && (
                  <pre className="max-h-40 overflow-auto whitespace-pre-wrap break-all font-mono text-destructive">
                    {e.stderr}
                  </pre>
                )}
                <div className="text-muted-foreground">
                  退出码 {e.exit_code ?? "—"} · {e.duration_ms}ms
                  {e.truncated && " · 输出已截断"}
                </div>
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

// ── 文件传输标签页：左本地暂存区 ↔ 右远端目录浏览 ─────────────────────────────
function FilePanel() {
  const fileNotice = useStore((s) => s.fileNotice);

  return (
    <div className="flex h-full flex-col">
      <div className="grid min-h-0 flex-1 grid-cols-2 divide-x divide-border">
        <LocalPane />
        <RemotePane />
      </div>
      {fileNotice && (
        <div className="shrink-0 border-t border-border px-3 py-1.5 text-xs text-muted-foreground">
          {fileNotice}
        </div>
      )}
    </div>
  );
}

// 左栏：本地暂存区（浏览器无法枚举本地磁盘，改用选择/拖入文件形成待传列表）。
function LocalPane() {
  const pushFile = useStore((s) => s.pushFile);
  const remotePath = useStore((s) => s.remotePath);
  const [staged, setStaged] = useState<File[]>([]);
  const [dragOver, setDragOver] = useState(false);

  function addFiles(list: FileList | null) {
    if (!list) return;
    setStaged((prev) => [...prev, ...Array.from(list)]);
  }
  function remove(i: number) {
    setStaged((prev) => prev.filter((_, idx) => idx !== i));
  }
  function send(file: File, i: number) {
    void pushFile(file, remotePath || undefined);
    remove(i);
  }

  return (
    <div className="flex min-h-0 flex-col p-3">
      <h3 className="mb-2 flex shrink-0 items-center gap-1.5 text-xs font-semibold text-foreground">
        <Upload className="size-3.5 shrink-0" aria-hidden /> 本地文件（暂存区）
      </h3>

      <label
        onDragOver={(e) => {
          e.preventDefault();
          setDragOver(true);
        }}
        onDragLeave={() => setDragOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragOver(false);
          addFiles(e.dataTransfer.files);
        }}
        className={
          "flex shrink-0 cursor-pointer items-center justify-center gap-1.5 rounded-md border border-dashed px-3 py-3 text-xs hover:border-primary " +
          (dragOver ? "border-primary bg-primary/5 text-primary" : "border-border bg-background text-foreground")
        }
      >
        <Upload className="size-3.5 shrink-0" aria-hidden /> 拖入文件，或点击选择
        <input
          type="file"
          multiple
          className="hidden"
          onChange={(e) => {
            addFiles(e.target.files);
            e.target.value = "";
          }}
        />
      </label>

      <div className="mt-2 flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto">
        {staged.length === 0 ? (
          <p className="text-xs text-muted-foreground">
            选择待传文件后，点「下发」发送到右侧远端当前目录{remotePath ? "" : "（未浏览时落被控端接收目录）"}。单文件 ≤ 50MB。
          </p>
        ) : (
          staged.map((f, i) => (
            <div key={`${f.name}-${i}`} className="flex items-center gap-2 rounded-md border border-border bg-background px-2 py-1.5 text-xs">
              <FileIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
              <span className="min-w-0 flex-1 truncate text-foreground" title={f.name}>
                {f.name}
              </span>
              <span className="shrink-0 text-muted-foreground">{fmtSize(BigInt(f.size))}</span>
              <Button size="sm" className="h-6 px-2" onClick={() => send(f, i)}>
                <Download className="size-3" aria-hidden /> 下发
              </Button>
              <button
                type="button"
                onClick={() => remove(i)}
                className="shrink-0 text-muted-foreground hover:text-destructive"
                title="移除"
              >
                <X className="size-3.5" />
              </button>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

// 右栏：远端目录浏览（被控端真实文件系统，经 file_list_request/resp 协议）。
function RemotePane() {
  const remotePath = useStore((s) => s.remotePath);
  const remoteEntries = useStore((s) => s.remoteEntries);
  const loading = useStore((s) => s.remoteListLoading);
  const error = useStore((s) => s.remoteListError);
  const listRemote = useStore((s) => s.listRemote);
  const pullFile = useStore((s) => s.pullFile);

  // 首次进入文件传输标签页时加载被控端默认目录（home）。
  // 以 store 状态（而非组件内 ref）判定是否已加载——切标签/折叠导致组件重挂时不重复拉取、不刷审计；
  // 出错后不自动重试（error 已置位），由用户点「刷新」恢复。
  useEffect(() => {
    if (!remotePath && !loading && !error && remoteEntries.length === 0) {
      listRemote("");
    }
  }, [remotePath, loading, error, remoteEntries.length, listRemote]);

  return (
    <div className="flex min-h-0 flex-col p-3">
      <div className="mb-2 flex shrink-0 items-center gap-1.5">
        <h3 className="flex items-center gap-1.5 text-xs font-semibold text-foreground">
          <Folder className="size-3.5 shrink-0" aria-hidden /> 远端文件
        </h3>
        <div className="ml-auto flex items-center gap-1">
          <button
            type="button"
            onClick={() => remotePath && listRemote(parentPath(remotePath))}
            disabled={!remotePath}
            className="rounded p-1 text-muted-foreground hover:bg-secondary hover:text-foreground disabled:opacity-40"
            title="上级目录"
            aria-label="上级目录"
          >
            <ArrowUp className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={() => listRemote(remotePath)}
            className="rounded p-1 text-muted-foreground hover:bg-secondary hover:text-foreground"
            title="刷新"
            aria-label="刷新当前目录"
          >
            <RefreshCw className="size-3.5" />
          </button>
        </div>
      </div>

      <div className="mb-1 shrink-0 truncate font-mono text-[11px] text-muted-foreground" title={remotePath}>
        {remotePath || "（加载中…）"}
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto rounded-md border border-border bg-background p-1">
        {loading ? (
          <div className="flex items-center gap-1 p-2 text-xs text-muted-foreground">
            <Loader2 className="size-3 animate-spin" aria-hidden /> 加载中…
          </div>
        ) : error ? (
          <div className="p-2 text-xs text-destructive">{error}</div>
        ) : remoteEntries.length === 0 ? (
          <div className="p-2 text-xs text-muted-foreground">（空目录）</div>
        ) : (
          remoteEntries.map((entry) => (
            <div
              key={entry.name}
              className="group flex items-center gap-2 rounded px-2 py-1 text-xs hover:bg-secondary"
            >
              {entry.is_dir ? (
                <button
                  type="button"
                  onClick={() => listRemote(joinPath(remotePath, entry.name))}
                  className="flex min-w-0 flex-1 items-center gap-2 text-left text-foreground"
                  title="进入目录"
                >
                  <Folder className="size-3.5 shrink-0 text-primary" aria-hidden />
                  <span className="truncate">{entry.name}</span>
                </button>
              ) : (
                <>
                  <FileIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
                  <span className="min-w-0 flex-1 truncate text-foreground" title={entry.name}>
                    {entry.name}
                  </span>
                  <span className="shrink-0 text-muted-foreground">{fmtSize(entry.size)}</span>
                  <Button
                    size="sm"
                    variant="outline"
                    className="h-6 px-2 opacity-0 group-hover:opacity-100"
                    onClick={() => pullFile(joinPath(remotePath, entry.name))}
                  >
                    <Download className="size-3" aria-hidden /> 取回
                  </Button>
                </>
              )}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
