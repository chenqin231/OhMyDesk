import { BrowserRouter, Routes, Route, Navigate, useLocation } from "react-router-dom";
import { useEffect, type ReactNode } from "react";
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
  const token = useAuthStore((s) => s.token);
  const user = useAuthStore((s) => s.user);
  const permissions = useAuthStore((s) => s.permissions);
  const loadMe = useAuthStore((s) => s.loadMe);
  const location = useLocation();

  // 刷新后 token 仍在但 user 为空：补拉一次当前用户/角色/权限
  useEffect(() => {
    if (token && !user) void loadMe();
  }, [token, user, loadMe]);

  if (!token) {
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }
  // 需要权限但权限尚未加载完（刷新后 loadMe 未回）：先等待，避免误判把用户踢走
  if (permission && permissions === null) {
    return null;
  }
  if (permission && !hasPermission(permissions, permission)) {
    return <Navigate to={defaultPathForPermissions(permissions)} replace />;
  }
  return <>{children}</>;
}

// 已登录访问 / 或未匹配路由时，跳到登录者权限对应的默认页。
function DefaultRedirect() {
  const token = useAuthStore((s) => s.token);
  const user = useAuthStore((s) => s.user);
  const permissions = useAuthStore((s) => s.permissions);
  const loadMe = useAuthStore((s) => s.loadMe);

  useEffect(() => {
    if (token && !user) void loadMe();
  }, [token, user, loadMe]);

  if (!token) return <Navigate to="/login" replace />;
  if (permissions === null) return null; // 等 loadMe 拉回权限再定向
  return <Navigate to={defaultPathForPermissions(permissions)} replace />;
}

// 登录页守卫：已登录访问 /login 跳到权限默认页。
function LoginRoute() {
  const token = useAuthStore((s) => s.token);
  const user = useAuthStore((s) => s.user);
  const permissions = useAuthStore((s) => s.permissions);
  const loadMe = useAuthStore((s) => s.loadMe);

  useEffect(() => {
    if (token && !user) void loadMe();
  }, [token, user, loadMe]);

  if (!token) return <Login />;
  if (permissions === null) return null; // 等权限拉回再定向
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
            <RequireAuth permission="manage_settings">
              <Settings />
            </RequireAuth>
          }
        />
        <Route path="*" element={<DefaultRedirect />} />
      </Routes>
    </BrowserRouter>
  );
}
