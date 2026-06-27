import { useMemo, useState } from "react";
import { Check, Copy, Link2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { buildMcpConfig, getMcpApiBase } from "@/lib/mcp-config";
import { getToken } from "@/store/auth";

export function McpConfigCard() {
  const [copied, setCopied] = useState(false);
  const apiBase = getMcpApiBase();
  const token = getToken();
  const configText = useMemo(() => buildMcpConfig(apiBase, token), [apiBase, token]);

  async function copyConfig() {
    await copyText(configText);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1600);
  }

  return (
    <Card size="sm" className="shrink-0">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Link2 className="size-4 text-primary" aria-hidden />
          MCP 接入配置
        </CardTitle>
        <CardDescription>
          外部 AI 客户端通过 MCP 读取终端、会话和审计数据。
        </CardDescription>
        <CardAction>
          <Button variant="outline" size="sm" onClick={() => void copyConfig()}>
            {copied ? <Check data-icon="inline-start" /> : <Copy data-icon="inline-start" />}
            {copied ? "已复制" : "复制配置"}
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="grid gap-3 md:grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)]">
        <div className="rounded-lg border border-border bg-secondary/50 p-3">
          <div className="text-xs text-muted-foreground">MCP 数据源</div>
          <div className="mt-1 break-all font-mono text-xs text-foreground">{apiBase}</div>
          <div className="mt-2 text-xs text-muted-foreground">
            Token：{token ? "已包含当前登录凭据" : "未登录或 token 缺失"}
          </div>
        </div>
        <pre className="max-h-36 overflow-auto rounded-lg border border-border bg-muted/40 p-3 text-xs leading-relaxed text-muted-foreground">
          {configText}
        </pre>
      </CardContent>
    </Card>
  );
}

async function copyText(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // 非 HTTPS 内网页面可能被浏览器拒绝，降级到 textarea 复制。
    }
  }

  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  document.body.appendChild(textarea);
  textarea.focus();
  textarea.select();
  document.execCommand("copy");
  textarea.remove();
}
