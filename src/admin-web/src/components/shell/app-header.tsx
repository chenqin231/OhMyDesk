import { Bot } from "lucide-react";
import { Link } from "react-router-dom";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { SidebarTrigger } from "@/components/ui/sidebar";

type AppHeaderProps = {
  title: string;
  online?: number;
  total?: number;
};

export function AppHeader({ title, online = 0, total = 0 }: AppHeaderProps) {
  return (
    <header className="sticky top-0 z-10 flex h-14 shrink-0 items-center gap-3 border-b border-border bg-background/80 px-4 backdrop-blur">
      <SidebarTrigger className="text-muted-foreground" />
      <Separator orientation="vertical" className="h-5" />
      <h1 className="text-sm font-semibold tracking-wide">{title}</h1>

      <div className="ml-auto flex items-center gap-2">
        {/* 全局在线统计胶囊 */}
        <Badge
          variant="outline"
          className="gap-1.5 rounded-full border-border bg-card py-1 font-normal text-muted-foreground"
        >
          <span className="relative flex size-2">
            <span className="absolute inline-flex size-full animate-ping rounded-full bg-online opacity-60" />
            <span className="relative inline-flex size-2 rounded-full bg-online" />
          </span>
          <span>
            在线 <span className="font-mono font-medium text-online">{online}</span>
            <span className="mx-1 text-border">/</span>
            总数 <span className="font-mono font-medium text-foreground">{total}</span>
          </span>
        </Badge>

        {/* AI 助手快捷入口 */}
        <Button
          variant="ghost"
          size="icon"
          className="text-muted-foreground hover:text-primary"
          aria-label="打开 AI 助手"
          render={<Link to="/assistant" />}
        >
          <Bot />
        </Button>
      </div>
    </header>
  );
}
