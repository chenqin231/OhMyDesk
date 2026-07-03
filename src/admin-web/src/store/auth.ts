// 鉴权 store：管理 token / 当前用户 / 身份(tier) / 权限，封装登录、登出、改密、拉取当前用户。
// token 持久化到 localStorage，刷新后保留；其余 store 与 real.ts 通过 getToken() 读取同一份。
// tier/permissions 不持久化，刷新后由 loadMe 从 /api/me 重新拉取（server 为唯一权威）。
import { create } from "zustand";

import type { Permission, Tier } from "@/lib/permissions";

const TOKEN_KEY = "ohmydesk_token";

// 从 localStorage 读取 token（real.ts 拼 WS / 加 Bearer 时复用，避免环依赖）
export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

function setStoredToken(token: string | null) {
  if (token) localStorage.setItem(TOKEN_KEY, token);
  else localStorage.removeItem(TOKEN_KEY);
}

// 统一拼接同源 API 地址：默认相对路径（生产由 server 托管）；
// 开发期可用 VITE_API_BASE 指向 http://127.0.0.1:8765 跨端口联调。
function apiUrl(path: string): string {
  const base = (import.meta.env.VITE_API_BASE as string | undefined) ?? "";
  return `${base}${path}`;
}

// 带 Bearer token 的 JSON 请求头（用户管理各写操作复用，避免重复拼接）
function authJsonHeaders(token: string | null): Record<string, string> {
  return {
    "Content-Type": "application/json",
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
  };
}

// 管理端用户记录：字段与 server users.rs UserRecord 序列化一一对应
// （password_hash 后端 skip_serializing，不下发）。tier 为身份二值，permissions 为按账户菜单集。
export type AdminUser = {
  id: string;
  username: string;
  tier: Tier;
  permissions: Permission[];
  enabled: boolean;
  created_at: number;
  updated_at: number;
};

type AuthState = {
  token: string | null;
  // 当前登录账号 user_id（= server users.id）。支撑 superadmin 视图按 owner_id 归属展示终端。
  userId: string | null;
  user: string | null;
  tier: Tier | null;
  permissions: Permission[] | null;
  // loadMe 是否已完成过一次（无论成败）。路由守卫据此区分「加载中」与「已加载」，
  // 避免 /api/me 遇 5xx/网络抖动时 permissions 永停 null 而整屏空白。
  meLoaded: boolean;
  // loadMe 是否在途：用于首帧/重试的加载态展示与并发去重。
  meLoading: boolean;
  // loadMe 失败信息（非 401 的 5xx / 网络异常）。非空时守卫渲染可重试错误态而非空屏。
  authError: string | null;
  // 登录：成功存 token 并返回；失败抛出可读错误信息
  login: (user: string, pass: string) => Promise<void>;
  // 登出：清 token + user + tier + permissions + 身份加载标志（跳转由调用方负责）
  logout: () => void;
  // 自助改密（人人可用，仅需登录）：POST /api/me/password { old, new }
  changeOwnPassword: (oldPassword: string, newPassword: string) => Promise<void>;
  // 用 token 拉当前用户；401 清空，5xx/网络异常记 authError，无论成败置 meLoaded=true
  loadMe: () => Promise<void>;
  // ── 用户管理（需 manage_users 权限 = superadmin 独占）──
  // 拉取全部管理端账号
  listUsers: () => Promise<AdminUser[]>;
  // 新建账号（tier 固定 user；权限由菜单集指定，superadmin 由 bootstrap 唯一）
  createUser: (input: {
    username: string;
    password: string;
    permissions: Permission[];
  }) => Promise<void>;
  // 配菜单集 / 改用户名 / 停启用（superadmin 目标由后端守卫拦截）
  updateUser: (
    id: string,
    input: { permissions?: Permission[]; username?: string; enabled?: boolean },
  ) => Promise<void>;
  // 重置指定账号密码
  resetUserPassword: (id: string, password: string) => Promise<void>;
};

export const useAuthStore = create<AuthState>((set, get) => ({
  token: getToken(),
  userId: null,
  user: null,
  tier: null,
  permissions: null,
  meLoaded: false,
  meLoading: false,
  authError: null,

  async login(user, pass) {
    const res = await fetch(apiUrl("/api/login"), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ user, pass }),
    });
    if (res.status === 401) {
      throw new Error("账号或密码错误");
    }
    if (!res.ok) {
      const msg = await readError(res);
      throw new Error(msg ?? "登录失败，请稍后重试");
    }
    const data = (await res.json()) as {
      token: string;
      id: string;
      user: string;
      tier: Tier;
      permissions: Permission[];
    };
    setStoredToken(data.token);
    // 登录直接带回身份：标记已加载、清错误，避免守卫再走一次 loadMe 加载态
    set({
      token: data.token,
      userId: data.id,
      user: data.user,
      tier: data.tier,
      permissions: data.permissions,
      meLoaded: true,
      meLoading: false,
      authError: null,
    });
  },

  logout() {
    setStoredToken(null);
    set({
      token: null,
      userId: null,
      user: null,
      tier: null,
      permissions: null,
      meLoaded: false,
      meLoading: false,
      authError: null,
    });
  },

  async changeOwnPassword(oldPassword, newPassword) {
    const token = get().token;
    const res = await fetch(apiUrl("/api/me/password"), {
      method: "POST",
      headers: authJsonHeaders(token),
      body: JSON.stringify({ old: oldPassword, new: newPassword }),
    });
    if (res.status === 401) {
      throw new Error("登录已失效，请重新登录");
    }
    if (!res.ok) {
      const msg = await readError(res);
      throw new Error(msg ?? "修改密码失败");
    }
  },

  async loadMe() {
    const { token, meLoading } = get();
    if (!token) {
      set({ meLoaded: true, meLoading: false });
      return;
    }
    if (meLoading) return; // 已有在途请求，避免并发重复拉取
    set({ meLoading: true, authError: null });
    try {
      const res = await fetch(apiUrl("/api/me"), {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (res.status === 401) {
        // token 失效：清空（logout 会一并重置 meLoaded 等），守卫据此回 /login
        get().logout();
        return;
      }
      if (!res.ok) {
        // 5xx 等：保留 token，记错误，守卫据此渲染可重试态而非空屏
        set({ authError: `加载账户信息失败（${res.status}），请重试` });
        return;
      }
      const data = (await res.json()) as {
        id: string;
        user: string;
        tier: Tier;
        permissions: Permission[];
      };
      set({
        userId: data.id,
        user: data.user,
        tier: data.tier,
        permissions: data.permissions,
        authError: null,
      });
    } catch (err) {
      // 网络异常/断网：同样不空屏，交由守卫展示重试
      set({
        authError:
          err instanceof Error
            ? `网络异常：${err.message}`
            : "网络异常，无法加载账户信息",
      });
    } finally {
      set({ meLoading: false, meLoaded: true });
    }
  },

  async listUsers() {
    const token = get().token;
    const res = await fetch(apiUrl("/api/users"), {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
    });
    if (!res.ok) {
      const msg = await readError(res);
      throw new Error(msg ?? "加载用户列表失败");
    }
    return (await res.json()) as AdminUser[];
  },

  async createUser(input) {
    const token = get().token;
    const res = await fetch(apiUrl("/api/users"), {
      method: "POST",
      headers: authJsonHeaders(token),
      body: JSON.stringify(input),
    });
    if (!res.ok) {
      const msg = await readError(res);
      throw new Error(msg ?? "创建用户失败");
    }
  },

  async updateUser(id, input) {
    const token = get().token;
    const res = await fetch(apiUrl(`/api/users/${id}`), {
      method: "PATCH",
      headers: authJsonHeaders(token),
      body: JSON.stringify(input),
    });
    if (!res.ok) {
      const msg = await readError(res);
      throw new Error(msg ?? "更新用户失败");
    }
  },

  async resetUserPassword(id, password) {
    const token = get().token;
    const res = await fetch(apiUrl(`/api/users/${id}/reset-password`), {
      method: "POST",
      headers: authJsonHeaders(token),
      body: JSON.stringify({ password }),
    });
    if (!res.ok) {
      const msg = await readError(res);
      throw new Error(msg ?? "重置密码失败");
    }
  },
}));

// 解析后端 { error } 文本，失败返回 null
async function readError(res: Response): Promise<string | null> {
  try {
    const data = (await res.json()) as { error?: string };
    return data.error ?? null;
  } catch {
    return null;
  }
}
