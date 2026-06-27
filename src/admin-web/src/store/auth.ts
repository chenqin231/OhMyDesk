// 鉴权 store：管理 token / 当前用户，封装登录、登出、改密、拉取当前用户。
// token 持久化到 localStorage，刷新后保留；其余 store 与 real.ts 通过 getToken() 读取同一份。
import { create } from "zustand";

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

type AuthState = {
  token: string | null;
  user: string | null;
  // 登录：成功存 token 并返回；失败抛出可读错误信息
  login: (user: string, pass: string) => Promise<void>;
  // 登出：清 token + user（跳转由调用方负责）
  logout: () => void;
  // 改密/改用户名：成功后返回，调用方负责提示并登出
  changeCredential: (
    currentPass: string,
    newUser: string,
    newPass: string,
  ) => Promise<void>;
  // 用 token 拉当前用户；token 失效则清空
  loadMe: () => Promise<void>;
};

export const useAuthStore = create<AuthState>((set, get) => ({
  token: getToken(),
  user: null,

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
    const data = (await res.json()) as { token: string; user: string };
    setStoredToken(data.token);
    set({ token: data.token, user: data.user });
  },

  logout() {
    setStoredToken(null);
    set({ token: null, user: null });
  },

  async changeCredential(currentPass, newUser, newPass) {
    const token = get().token;
    const res = await fetch(apiUrl("/api/settings/credential"), {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      body: JSON.stringify({
        current_pass: currentPass,
        new_user: newUser,
        new_pass: newPass,
      }),
    });
    if (res.status === 401) {
      throw new Error("登录已失效，请重新登录");
    }
    if (!res.ok) {
      const msg = await readError(res);
      throw new Error(msg ?? "更新失败");
    }
  },

  async loadMe() {
    const token = get().token;
    if (!token) return;
    const res = await fetch(apiUrl("/api/me"), {
      headers: { Authorization: `Bearer ${token}` },
    });
    if (res.status === 401) {
      // token 失效：清空，路由守卫会把页面带回 /login
      get().logout();
      return;
    }
    if (!res.ok) return;
    const data = (await res.json()) as { user: string };
    set({ user: data.user });
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
