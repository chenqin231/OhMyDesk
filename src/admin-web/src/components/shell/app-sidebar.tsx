import { useLocation, Link } from "react-router-dom";
import { ShieldCheck, ChevronsUpDown } from "lucide-react";

import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { navItems, systemNavItems } from "@/components/shell/nav-config";
import { hasPermission, roleLabel } from "@/lib/permissions";
import { useAuthStore } from "@/store/auth";

export function AppSidebar() {
  const { pathname } = useLocation();
  const user = useAuthStore((s) => s.user);
  const role = useAuthStore((s) => s.role);
  const permissions = useAuthStore((s) => s.permissions);

  // 无权限项（如 AI 助手）恒显示；有权限项按登录者权限过滤
  const visibleNavItems = navItems.filter(
    (item) => !item.permission || hasPermission(permissions, item.permission),
  );
  const visibleSystemItems = systemNavItems.filter(
    (item) => !item.permission || hasPermission(permissions, item.permission),
  );

  return (
    <Sidebar collapsible="icon">
      {/* 平台标识 */}
      <SidebarHeader className="border-b border-sidebar-border">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              size="lg"
              className="data-[slot=sidebar-menu-button]:!p-2"
            >
              <div className="flex aspect-square size-8 items-center justify-center rounded-md bg-primary text-primary-foreground">
                <ShieldCheck className="size-5" />
              </div>
              <div className="grid flex-1 text-left leading-tight">
                <span className="truncate text-sm font-semibold">
                  信创终端管控平台
                </span>
                <span className="truncate text-xs text-muted-foreground">
                  内网安全管控控制台
                </span>
              </div>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      {/* 主导航 */}
      <SidebarContent>
        {visibleNavItems.length > 0 && (
          <SidebarGroup>
            <SidebarGroupLabel>管控功能</SidebarGroupLabel>
            <SidebarMenu>
              {visibleNavItems.map((item) => {
                const isActive = pathname === item.href || pathname.startsWith(item.href + "/");
                return (
                  <SidebarMenuItem key={item.key}>
                    <SidebarMenuButton
                      isActive={isActive}
                      tooltip={item.title}
                      render={<Link to={item.href} />}
                    >
                      <item.icon />
                      <span>{item.title}</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                );
              })}
            </SidebarMenu>
          </SidebarGroup>
        )}

        {/* 系统级入口：组内全部无权限时整组隐藏 */}
        {visibleSystemItems.length > 0 && (
          <SidebarGroup>
            <SidebarGroupLabel>系统</SidebarGroupLabel>
            <SidebarMenu>
              {visibleSystemItems.map((item) => {
                const isActive = pathname === item.href || pathname.startsWith(item.href + "/");
                return (
                  <SidebarMenuItem key={item.key}>
                    <SidebarMenuButton
                      isActive={isActive}
                      tooltip={item.title}
                      render={<Link to={item.href} />}
                    >
                      <item.icon />
                      <span>{item.title}</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                );
              })}
            </SidebarMenu>
          </SidebarGroup>
        )}
      </SidebarContent>

      {/* 当前管理员 */}
      <SidebarFooter className="border-t border-sidebar-border">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg">
              <Avatar className="size-8 rounded-md">
                <AvatarFallback className="rounded-md bg-secondary text-xs">
                  {user ? user.slice(0, 2).toUpperCase() : "管理"}
                </AvatarFallback>
              </Avatar>
              <div className="grid flex-1 text-left leading-tight">
                <span className="truncate text-sm font-medium">
                  {user ?? "管理员"}
                </span>
                <span className="truncate text-xs text-muted-foreground">
                  {roleLabel(role)}
                </span>
              </div>
              <ChevronsUpDown className="ml-auto size-4 text-muted-foreground" />
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  );
}
