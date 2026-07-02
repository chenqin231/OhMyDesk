import {
  LayoutList,
  Monitor,
  MonitorPlay,
  ScrollText,
  Bot,
  History,
  Users,
  Settings,
  type LucideIcon,
} from "lucide-react";

import type { Permission } from "@/lib/permissions";

// 菜单项：permission 为可选——带权限的按登录者权限过滤，不带的（如 AI 助手）所有登录用户可见。
export type NavItem = {
  key: string;
  title: string;
  href: string;
  icon: LucideIcon;
  permission?: Permission;
};

export const navItems: readonly NavItem[] = [
  { key: "assets", title: "终端资产", href: "/assets", icon: LayoutList, permission: "view_assets" },
  { key: "grid", title: "批量监控", href: "/grid", icon: Monitor, permission: "view_grid" },
  { key: "remote", title: "远程控制", href: "/remote", icon: MonitorPlay, permission: "use_remote" },
  { key: "audit", title: "会话审计", href: "/audit", icon: ScrollText, permission: "view_audit" },
  { key: "assistant", title: "AI 助手", href: "/assistant", icon: Bot },
] as const;

// 系统级入口，单独分组渲染在管控功能下方
export const systemNavItems: readonly NavItem[] = [
  { key: "login-logs", title: "登录日志", href: "/login-logs", icon: History, permission: "view_login_logs" },
  { key: "users", title: "用户管理", href: "/users", icon: Users, permission: "manage_users" },
  { key: "settings", title: "系统设置", href: "/settings", icon: Settings, permission: "manage_settings" },
] as const;
