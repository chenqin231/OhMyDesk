import { BrowserRouter, Routes, Route, Navigate, useLocation } from "react-router-dom";
import { useEffect, type ReactNode } from "react";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Assets } from "@/pages/Assets";
import { Grid } from "@/pages/Grid";
import { Remote } from "@/pages/Remote";
import { Audit } from "@/pages/Audit";
import { Assistant } from "@/pages/Assistant";
import { LoginLogs } from "@/pages/LoginLogs";
import { Users } from "@/pages/Users";
import { Settings } from "@/pages/Settings";
import { Login } from "@/pages/Login";
import { useStore } from "@/store";
import { useAuthStore } from "@/store/auth";
import {
  defaultPathForPermissions,
  hasPermission,
  type Permission,
} from "@/lib/permissions";

// App 级别初始化 Transport（VITE_USE_MOCK=1 使用 mock，否则连真实 server）。
// 仅在已登录时挂载：未登录时 WS 无 token 会被 server 拒。
// token 变化（登录/登出）时 key 改变触发重挂，确保新 token 生效。
function TransportInit() {
  const token = useAuthStore((s) => s.token);
  if (!token) return null;
  return <TransportInitInner key={token} />;
}

function TransportInitInner() {
  const initTransport = useStore((s) => s.initTransport);
  const disconnectTransport = useStore((s) => s.disconnectTransport);
  useEffect(() => {
    initTransport();
    // 登出（token 置空）或 token 切换导致卸载时断开旧 WS，避免连接泄漏
    return () => disconnectTransport();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return null;
}

// 身份加载中占位：token 在、/api/me 未回时展示，替代原来的 `return null` 空屏。
function IdentityLoading() {
  return (
    <div className="flex min-h-screen items-center justify-center text-sm text-muted-foreground">
      正在加载账户信息…
    </div>
  );
}

// 身份加载失败态：/api/me 遇 5xx / 网络异常时展示，给重试 + 退出登录兜底，绝不空屏。
function IdentityError({
  message,
  onRetry,
  onLogout,
}: {
  message: string;
  onRetry: () => void;
  onLogout: () => void;
}) {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-4 p-6 text-center">
      <p className="max-w-sm text-sm text-destructive" role="alert">
        {message}
      </p>
      <div className="flex gap-2">
        <Button variant="outline" size="sm" onClick={onRetry}>
          <RefreshCw className="size-4" /> 重试
        </Button>
        <Button variant="ghost" size="sm" onClick={onLogout}>
          退出登录
        </Button>
      </div>
    </div>
  );
}

// 统一身份门（三守卫共用）：token 在则确保补拉 /api/me，并把「加载中 / 加载失败」
// 判断收敛到 gate。返回 token/permissions 与 gate——gate 非空时守卫直接返回它，
// 从而消除三处重复的 selector + useEffect + 等待逻辑（Task 6 遗留 B）。
// 注意：!token 分支不在此处理——三守卫落地不同（跳登录带 from / 渲染登录页），各自处理。
function useEnsureIdentity(): {
  token: string | null;
  permissions: Permission[] | null;
  gate: ReactNode;
} {
  const token = useAuthStore((s) => s.token);
  const permissions = useAuthStore((s) => s.permissions);
  const meLoaded = useAuthStore((s) => s.meLoaded);
  const meLoading = useAuthStore((s) => s.meLoading);
  const authError = useAuthStore((s) => s.authError);
  const loadMe = useAuthStore((s) => s.loadMe);
  const logout = useAuthStore((s) => s.logout);

  // 刷新后 token 仍在但未加载过身份：补拉一次。失败后 meLoaded=true，不会反复重试。
  useEffect(() => {
    if (token && !meLoaded) void loadMe();
  }, [token, meLoaded, loadMe]);

  let gate: ReactNode = null;
  if (token) {
    if (meLoading) gate = <IdentityLoading />;
    else if (authError)
      gate = (
        <IdentityError
          message={authError}
          onRetry={() => void loadMe()}
          onLogout={logout}
        />
      );
    else if (!meLoaded) gate = <IdentityLoading />; // 首帧 effect 未触发前
  }
  return { token, permissions, gate };
}

// 路由守卫：无 token → 重定向 /login；已登录则补拉当前用户/角色/权限。
// 可选 permission：无该权限时按 defaultPathForPermissions 跳回登录者可达的默认页。
// 注意：Transport 初始化不放在这里——避免每次路由切换重挂导致 WS 反复重连，
// 改由 Routes 外层的 <TransportInit> 在已登录时只挂一次。
function RequireAuth({
  children,
  permission,
}: {
  children: ReactNode;
  permission?: Permission;
}) {
  const location = useLocation();
  const { token, permissions, gate } = useEnsureIdentity();

  if (!token) {
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }
  if (gate) return gate; // 加载中 / 加载失败：等待或可重试，避免误判踢人或空屏
  if (permission && !hasPermission(permissions, permission)) {
    return <Navigate to={defaultPathForPermissions(permissions)} replace />;
  }
  return <>{children}</>;
}

// 已登录访问 / 或未匹配路由时，跳到登录者权限对应的默认页。
function DefaultRedirect() {
  const { token, permissions, gate } = useEnsureIdentity();

  if (!token) return <Navigate to="/login" replace />;
  if (gate) return gate;
  return <Navigate to={defaultPathForPermissions(permissions)} replace />;
}

// 登录页守卫：已登录访问 /login 跳到权限默认页。
function LoginRoute() {
  const { token, permissions, gate } = useEnsureIdentity();

  if (!token) return <Login />;
  if (gate) return gate;
  return <Navigate to={defaultPathForPermissions(permissions)} replace />;
}

export function App() {
  return (
    <BrowserRouter>
      <TransportInit />
      <Routes>
        <Route path="/login" element={<LoginRoute />} />
        <Route path="/" element={<DefaultRedirect />} />
        <Route
          path="/assets"
          element={
            <RequireAuth permission="view_assets">
              <Assets />
            </RequireAuth>
          }
        />
        <Route
          path="/grid"
          element={
            <RequireAuth permission="view_grid">
              <Grid />
            </RequireAuth>
          }
        />
        <Route
          path="/remote"
          element={
            <RequireAuth permission="use_remote">
              <Remote />
            </RequireAuth>
          }
        />
        <Route
          path="/audit"
          element={
            <RequireAuth permission="view_audit">
              <Audit />
            </RequireAuth>
          }
        />
        <Route
          path="/assistant"
          element={
            <RequireAuth>
              <Assistant />
            </RequireAuth>
          }
        />
        <Route
          path="/login-logs"
          element={
            <RequireAuth permission="view_login_logs">
              <LoginLogs />
            </RequireAuth>
          }
        />
        <Route
          path="/users"
          element={
            <RequireAuth permission="manage_users">
              <Users />
            </RequireAuth>
          }
        />
        <Route
          path="/settings"
          element={
            <RequireAuth>
              <Settings />
            </RequireAuth>
          }
        />
        <Route path="*" element={<DefaultRedirect />} />
      </Routes>
    </BrowserRouter>
  );
}
