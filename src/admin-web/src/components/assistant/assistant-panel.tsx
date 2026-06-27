import { useEffect, useRef, useState } from "react";
import { Bot, Send, ShieldCheck, Sparkles } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  buildAnswer,
  initialConversation,
  sampleQuestions,
  userMessage,
  type ChatMessage,
} from "@/lib/assistant";
import { ChatBubble, ThinkingBubble } from "@/components/assistant/chat-message";
import { useStore } from "@/store";

// G-5：接 store 数据，提供真实问答；降级脚本作为最终兜底
export function AssistantPanel() {
  const endpoints = useStore((s) => s.endpoints);
  const auditLogs = useStore((s) => s.auditLogs);
  const fetchAudit = useStore((s) => s.fetchAudit);
  const userAskedRef = useRef(false);

  const [messages, setMessages] = useState<ChatMessage[]>(() =>
    initialConversation(endpoints, auditLogs),
  );
  const [input, setInput] = useState("");
  const [thinking, setThinking] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  // 新消息时滚动到底部
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
  }, [messages, thinking]);

  useEffect(() => {
    void fetchAudit();
  }, [fetchAudit]);

  useEffect(() => {
    if (userAskedRef.current) return;
    setMessages(initialConversation(endpoints, auditLogs));
  }, [endpoints, auditLogs]);

  function send(text: string) {
    const q = text.trim();
    if (!q || thinking) return;
    userAskedRef.current = true;
    setInput("");
    setMessages((prev) => [...prev, userMessage(q)]);
    setThinking(true);
    // 模拟 MCP 工具调用延迟，基于 store 数据构建应答
    window.setTimeout(() => {
      setMessages((prev) => [...prev, buildAnswer(q, endpoints, auditLogs)]);
      setThinking(false);
    }, 1100);
  }

  return (
    <div className="flex h-full flex-col overflow-hidden rounded-xl border border-border bg-background">
      {/* 标题区 */}
      <header className="flex items-center gap-3 border-b border-border bg-card px-4 py-3">
        <span
          className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-primary/30 bg-primary/10 text-primary"
          aria-hidden
        >
          <Bot className="size-5" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="text-sm font-semibold text-foreground">AI 安全助手</h2>
          <p className="truncate text-xs text-muted-foreground">基于 MCP 实时查询内网管控数据</p>
        </div>
        <span className="inline-flex items-center gap-1.5 rounded-full border border-online/30 bg-online/10 px-2.5 py-1 text-xs text-online">
          <ShieldCheck className="size-3.5" aria-hidden />
          数据不出内网
        </span>
      </header>

      {/* 消息区 */}
      <div ref={scrollRef} className="flex-1 overflow-auto px-4 py-5">
        <div className="mx-auto flex max-w-3xl flex-col gap-5">
          {messages.map((m) => (
            <ChatBubble key={m.id} message={m} />
          ))}
          {thinking ? <ThinkingBubble /> : null}
        </div>
      </div>

      {/* 示例问题 + 输入区 */}
      <footer className="border-t border-border bg-card px-4 py-3">
        <div className="mx-auto flex max-w-3xl flex-col gap-3">
          <div className="flex flex-wrap items-center gap-2">
            <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
              <Sparkles className="size-3.5" aria-hidden />
              试试：
            </span>
            {sampleQuestions.map((q) => (
              <button
                key={q}
                type="button"
                onClick={() => send(q)}
                disabled={thinking}
                className="rounded-full border border-border bg-secondary px-3 py-1 text-xs text-foreground transition-colors hover:border-primary/40 hover:bg-primary/10 hover:text-primary disabled:opacity-50"
              >
                {q}
              </button>
            ))}
          </div>
          <form
            className="flex items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              send(input);
            }}
          >
            <Input
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder="用自然语言查询全网终端态势…"
              className="h-11 flex-1 text-base"
              aria-label="向 AI 安全助手提问"
            />
            <Button type="submit" size="lg" disabled={thinking || !input.trim()} className="h-11">
              <Send data-icon="inline-start" />
              发送
            </Button>
          </form>
          <p className="text-center text-[11px] text-muted-foreground">
            AI 助手仅通过 MCP 只读接口查询数据，不会对终端执行任何变更操作。
          </p>
        </div>
      </footer>
    </div>
  );
}
