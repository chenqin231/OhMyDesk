import { LayoutList, Monitor, MonitorPlay, ScrollText, Bot, Settings } from "lucide-react";

export const navItems = [
  { key: "assets", title: "终端资产", href: "/assets", icon: LayoutList },
  { key: "grid", title: "批量监控", href: "/grid", icon: Monitor },
  { key: "remote", title: "远程控制", href: "/remote", icon: MonitorPlay },
  { key: "audit", title: "会话审计", href: "/audit", icon: ScrollText },
  { key: "assistant", title: "AI 助手", href: "/assistant", icon: Bot },
] as const;

// 系统级入口，单独分组渲染在管控功能下方
export const systemNavItems = [
  { key: "settings", title: "系统设置", href: "/settings", icon: Settings },
] as const;
