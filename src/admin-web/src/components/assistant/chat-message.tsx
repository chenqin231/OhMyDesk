import { Bot, User, Wrench } from "lucide-react";

import { cn } from "@/lib/utils";
import type { AnswerBlock, ChatMessage } from "@/lib/assistant";

// 工具调用 Chip：体现 AI 通过 MCP 调用了平台只读工具
function ToolChip({ name, args }: { name: string; args: string }) {
  return (
    <div className="mb-2 inline-flex max-w-full items-center gap-2 rounded-md border border-primary/30 bg-primary/10 px-2.5 py-1.5">
      <Wrench className="size-3.5 shrink-0 text-primary" aria-hidden />
      <span className="font-mono text-xs text-primary">
        {name}
        <span className="text-primary/60">({args})</span>
      </span>
    </div>
  );
}

// 结构化内容块渲染
function Block({ block }: { block: AnswerBlock }) {
  if (block.type === "text") {
    return <p className="text-sm leading-relaxed text-foreground">{block.text}</p>;
  }
  if (block.type === "stat") {
    return (
      <div className="inline-flex flex-col rounded-lg border border-border bg-secondary/50 px-4 py-2">
        <span className="text-2xl font-semibold tabular-nums text-foreground">{block.value}</span>
        <span className="text-xs text-muted-foreground">{block.label}</span>
      </div>
    );
  }
  // table
  return (
    <div className="overflow-hidden rounded-lg border border-border">
      <table className="w-full border-collapse text-sm">
        <thead>
          <tr className="bg-secondary/60">
            {block.columns.map((c) => (
              <th key={c} className="px-3 py-2 text-left text-xs font-medium text-muted-foreground">
                {c}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {block.rows.map((row, ri) => (
            <tr key={ri} className="border-t border-border">
              {row.map((cell, ci) => (
                <td
                  key={ci}
                  className={cn(
                    "px-3 py-2 text-foreground",
                    block.mono?.includes(ci) && "font-mono text-[13px] text-muted-foreground",
                  )}
                >
                  {cell}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function ChatBubble({ message }: { message: ChatMessage }) {
  const isUser = message.role === "user";

  if (isUser) {
    return (
      <div className="flex items-start justify-end gap-3">
        <div className="max-w-[80%] rounded-2xl rounded-tr-sm bg-primary px-4 py-2.5 text-sm leading-relaxed text-primary-foreground">
          {message.text}
        </div>
        <span
          className="flex size-8 shrink-0 items-center justify-center rounded-full border border-border bg-secondary text-muted-foreground"
          aria-hidden
        >
          <User className="size-4" />
        </span>
      </div>
    );
  }

  return (
    <div className="flex items-start gap-3">
      <span
        className="flex size-8 shrink-0 items-center justify-center rounded-full border border-primary/30 bg-primary/10 text-primary"
        aria-hidden
      >
        <Bot className="size-4" />
      </span>
      <div className="min-w-0 max-w-[88%] rounded-2xl rounded-tl-sm border border-border bg-card px-4 py-3">
        {message.tool ? <ToolChip name={message.tool.name} args={message.tool.args} /> : null}
        <div className="flex flex-col gap-3">
          {message.blocks?.map((b, i) => <Block key={i} block={b} />)}
        </div>
      </div>
    </div>
  );
}

// AI 思考中（工具调用进行中）
export function ThinkingBubble() {
  return (
    <div className="flex items-start gap-3">
      <span
        className="flex size-8 shrink-0 items-center justify-center rounded-full border border-primary/30 bg-primary/10 text-primary"
        aria-hidden
      >
        <Bot className="size-4" />
      </span>
      <div className="flex items-center gap-2 rounded-2xl rounded-tl-sm border border-border bg-card px-4 py-3">
        <Wrench className="size-3.5 animate-pulse text-primary" aria-hidden />
        <span className="text-sm text-muted-foreground">正在调用 MCP 工具查询内网数据…</span>
        <span className="flex gap-1">
          <span className="size-1.5 animate-bounce rounded-full bg-muted-foreground [animation-delay:-0.3s]" />
          <span className="size-1.5 animate-bounce rounded-full bg-muted-foreground [animation-delay:-0.15s]" />
          <span className="size-1.5 animate-bounce rounded-full bg-muted-foreground" />
        </span>
      </div>
    </div>
  );
}
