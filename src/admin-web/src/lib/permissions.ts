// 前端权限模型：与 server 端 users.rs 的 Permission / tier 保持一一对应。
// 后端 /api/login、/api/me 返回 { user, tier, permissions }：tier 为账户身份二值，
// permissions 为 snake_case 权限字符串数组（每账户自定义的菜单集）。superadmin 隐式全集。
// 本文件负责类型化与判定；不再有 role→permission 硬映射（权限一律以存储集为准）。

// 账户身份二值：superadmin 隐式全权且独占账户管理；user 权限由存储集驱动。
export type Tier = "superadmin" | "user";

// 与 server users.rs Permission 枚举对齐：6 项可配给普通账户 + manage_users（superadmin 独占）。
// manage_assets 门控资产页删除终端 UI（批量删除 + 单行「删除该终端」，见 terminal-assets.tsx），
// 依赖 view_assets：仅有 view_assets 时隐藏删除入口。
export type Permission =
  | "view_assets"
  | "manage_assets"
  | "view_grid"
  | "use_remote"
  | "view_audit"
  | "view_login_logs"
  | "manage_users";

// tier 中文名（侧边栏展示当前登录者身份）
const TIER_LABELS: Record<Tier, string> = {
  superadmin: "超级管理员",
  user: "普通用户",
};

export function tierLabel(tier: Tier | null | undefined): string {
  return tier ? TIER_LABELS[tier] : "普通用户";
}

// 可配给普通账户的菜单元数据：Users 页勾选器据此渲染。
// parent 表示子项依赖：manage_assets 依赖 view_assets（父未勾时子不可勾）。
// 不含 manage_users（superadmin 独占）与自助改密（人人默认）。
export type AssignableMenu = {
  key: Permission;
  label: string;
  parent?: Permission;
};

export const ASSIGNABLE_MENUS: readonly AssignableMenu[] = [
  { key: "view_assets", label: "终端资产" },
  { key: "manage_assets", label: "可删除终端", parent: "view_assets" },
  { key: "view_grid", label: "批量监控" },
  { key: "use_remote", label: "远程控制" },
  { key: "view_audit", label: "会话审计" },
  { key: "view_login_logs", label: "登录日志" },
] as const;

// 判定当前权限集合是否包含某权限；permissions 为空（未加载/未登录）一律为 false
export function hasPermission(
  permissions: readonly Permission[] | null | undefined,
  permission: Permission,
): boolean {
  return Boolean(permissions?.includes(permission));
}

// 登录/跳转时按权限选择落地页：优先资产 → 审计 → 登录日志，都无则回登录页。
export function defaultPathForPermissions(
  permissions: readonly Permission[] | null | undefined,
): string {
  if (hasPermission(permissions, "view_assets")) return "/assets";
  if (hasPermission(permissions, "view_audit")) return "/audit";
  if (hasPermission(permissions, "view_login_logs")) return "/login-logs";
  return "/login";
}
