// 前端权限模型：与 server 端 users.rs 的 Role / Permission 保持一一对应。
// 后端 /api/login、/api/me 返回 { role, permissions }，role 为小写字符串，
// permissions 为 snake_case 权限字符串数组，本文件负责类型化与判定。

export type Role = "superadmin" | "admin" | "operator" | "auditor";

// 与 server users.rs Permission 枚举对齐（含 manage_assets：superadmin/admin 会返回，
// 但当前前端无资产管理入口使用它，仅为让类型与后端返回完全一致）。
export type Permission =
  | "view_assets"
  | "manage_assets"
  | "view_grid"
  | "use_remote"
  | "view_audit"
  | "view_login_logs"
  | "manage_users"
  | "manage_settings";

// 角色中文名（侧边栏展示当前登录者角色）
const ROLE_LABELS: Record<Role, string> = {
  superadmin: "超级管理员",
  admin: "管理员",
  operator: "操作员",
  auditor: "审计员",
};

export function roleLabel(role: Role | null | undefined): string {
  return role ? ROLE_LABELS[role] : "管理员";
}

// 判定当前权限集合是否包含某权限；permissions 为空（未加载/未登录）一律为 false
export function hasPermission(
  permissions: readonly Permission[] | null | undefined,
  permission: Permission,
): boolean {
  return Boolean(permissions?.includes(permission));
}

// 登录/跳转时按权限选择落地页：优先资产 → 审计 → 登录日志，都无则回登录页。
// 覆盖各角色：operator→/assets，auditor→/audit，superadmin/admin→/assets。
export function defaultPathForPermissions(
  permissions: readonly Permission[] | null | undefined,
): string {
  if (hasPermission(permissions, "view_assets")) return "/assets";
  if (hasPermission(permissions, "view_audit")) return "/audit";
  if (hasPermission(permissions, "view_login_logs")) return "/login-logs";
  return "/login";
}
