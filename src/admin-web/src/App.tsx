import { BrowserRouter, Routes, Route, Navigate, useLocation } from "react-router-dom";
import { useEffect, type ReactNode } from "react";
import { Assets } from "@/pages/Assets";
import { Grid } from "@/pages/Grid";
import { Remote } from "@/pages/Remote";
import { Audit } from "@/pages/Audit";
import { Assistant } from "@/pages/Assistant";
import { Settings } from "@/pages/Settings";
import { Login } from "@/pages/Login";
import { useStore } from "@/store";
import { useAuthStore } from "@/store/auth";

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

// 路由守卫：无 token → 重定向 /login；已登录则补拉当前用户名。
// 注意：Transport 初始化不放在这里——避免每次路由切换重挂导致 WS 反复重连，
// 改由 Routes 外层的 <TransportInit> 在已登录时只挂一次。
function RequireAuth({ children }: { children: ReactNode }) {
  const token = useAuthStore((s) => s.token);
  const user = useAuthStore((s) => s.user);
  const loadMe = useAuthStore((s) => s.loadMe);
  const location = useLocation();

  // 刷新后 token 仍在但 user 为空：补拉一次当前用户名
  useEffect(() => {
    if (token && !user) void loadMe();
  }, [token, user, loadMe]);

  if (!token) {
    return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  }
  return <>{children}</>;
}

// 登录页守卫：已登录访问 /login 直接跳 /assets。
function LoginRoute() {
  const token = useAuthStore((s) => s.token);
  if (token) return <Navigate to="/assets" replace />;
  return <Login />;
}

export function App() {
  return (
    <BrowserRouter>
      <TransportInit />
      <Routes>
        <Route path="/login" element={<LoginRoute />} />
        <Route
          path="/"
          element={
            <RequireAuth>
              <Navigate to="/assets" replace />
            </RequireAuth>
          }
        />
        <Route
          path="/assets"
          element={
            <RequireAuth>
              <Assets />
            </RequireAuth>
          }
        />
        <Route
          path="/grid"
          element={
            <RequireAuth>
              <Grid />
            </RequireAuth>
          }
        />
        <Route
          path="/remote"
          element={
            <RequireAuth>
              <Remote />
            </RequireAuth>
          }
        />
        <Route
          path="/audit"
          element={
            <RequireAuth>
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
          path="/settings"
          element={
            <RequireAuth>
              <Settings />
            </RequireAuth>
          }
        />
        <Route path="*" element={<Navigate to="/assets" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
