import { useState } from "react";
import { Terminal, Upload, Download, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useStore } from "@/store";

// 远控会话内的「命令执行 + 文件传输」侧栏。挂在远控画面右侧。
// 所有操作复用当前已授权会话（remoteSessionId），被控端无需逐条确认，server 全程审计。
const inputCls =
  "min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1.5 font-mono text-xs " +
  "text-foreground placeholder:text-muted-foreground outline-none focus:border-primary";

export function RemoteTools() {
  const execResults = useStore((s) => s.execResults);
  const fileNotice = useStore((s) => s.fileNotice);
  const execCommand = useStore((s) => s.execCommand);
  const pushFile = useStore((s) => s.pushFile);
  const pullFile = useStore((s) => s.pullFile);

  const [cmd, setCmd] = useState("");
  const [pullPath, setPullPath] = useState("");

  return (
    <aside className="flex w-80 shrink-0 flex-col gap-4 overflow-y-auto border-l border-border bg-card p-3">
      {/* 命令面板 */}
      <section className="flex min-h-0 flex-col">
        <h3 className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-foreground">
          <Terminal className="size-3.5 shrink-0" aria-hidden /> 远程命令执行
        </h3>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            execCommand(cmd);
            setCmd("");
          }}
          className="flex gap-1.5"
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

        <div className="mt-2 flex flex-col gap-2">
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
      </section>

      {/* 文件面板 */}
      <section>
        <h3 className="mb-2 flex items-center gap-1.5 text-xs font-semibold text-foreground">
          <Upload className="size-3.5 shrink-0" aria-hidden /> 文件传输
        </h3>

        <label className="flex cursor-pointer items-center justify-center gap-1.5 rounded-md border border-dashed border-border bg-background px-3 py-2 text-xs text-foreground hover:border-primary">
          <Upload className="size-3.5 shrink-0" aria-hidden /> 选择文件下发到被控端
          <input
            type="file"
            className="hidden"
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) void pushFile(f);
              e.target.value = "";
            }}
          />
        </label>

        <form
          onSubmit={(e) => {
            e.preventDefault();
            pullFile(pullPath);
            setPullPath("");
          }}
          className="mt-2 flex gap-1.5"
        >
          <input
            value={pullPath}
            onChange={(e) => setPullPath(e.target.value)}
            placeholder="被控端文件绝对路径"
            className={inputCls}
            spellCheck={false}
            autoComplete="off"
          />
          <Button type="submit" size="sm" variant="outline" disabled={!pullPath.trim()}>
            <Download className="size-3.5" aria-hidden />
            取回
          </Button>
        </form>

        {fileNotice && <p className="mt-2 break-all text-xs text-muted-foreground">{fileNotice}</p>}
        <p className="mt-1 text-[11px] text-muted-foreground">下发文件落在被控端 OhMyDesk 接收目录；单文件 ≤ 50MB。</p>
      </section>
    </aside>
  );
}
