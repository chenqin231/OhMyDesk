import { LayoutList, Monitor, MonitorPlay, ScrollText, Bot } from "lucide-react";

export const navItems = [
  { key: "assets", title: "终端资产", href: "/assets", icon: LayoutList },
  { key: "grid", title: "批量监控", href: "/grid", icon: Monitor },
  { key: "remote", title: "远程控制", href: "/remote", icon: MonitorPlay },
  { key: "audit", title: "会话审计", href: "/audit", icon: ScrollText },
  { key: "assistant", title: "AI 助手", href: "/assistant", icon: Bot },
] as const;
