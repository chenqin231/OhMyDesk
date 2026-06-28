import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";

import { AppShell } from "@/components/shell/app-shell";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useAuthStore } from "@/store/auth";
import { useStore } from "@/store";

// 系统设置页：修改管理员用户名 / 密码。成功后强制登出并跳登录。
export function Settings() {
  const navigate = useNavigate();
  const user = useAuthStore((s) => s.user);
  const loadMe = useAuthStore((s) => s.loadMe);
  const changeCredential = useAuthStore((s) => s.changeCredential);
  const logout = useAuthStore((s) => s.logout);

  const endpoints = useStore((s) => s.endpoints);
  const online = endpoints.filter((ep) => ep.online).length;
  const total = endpoints.length;

  const [currentPass, setCurrentPass] = useState("");
  const [newUser, setNewUser] = useState("");
  const [newPass, setNewPass] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  // 进入页面若 store.user 为空，拉一次当前用户名回显
  useEffect(() => {
    if (!user) void loadMe();
  }, [user, loadMe]);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setSuccess(null);
    if (!newUser.trim() && !newPass) {
      setError("请至少填写新用户名或新密码");
      return;
    }
    setLoading(true);
    try {
      await changeCredential(currentPass, newUser.trim(), newPass);
      setSuccess("已更新，请重新登录");
      // 凭据已变更，强制登出回登录页
      setTimeout(() => {
        logout();
        navigate("/login", { replace: true });
      }, 1200);
    } catch (err) {
      setError(err instanceof Error ? err.message : "更新失败");
    } finally {
      setLoading(false);
    }
  }

  return (
    <AppShell title="系统设置" online={online} total={total}>
      <div className="mx-auto max-w-lg">
        <Card>
          <CardHeader>
            <CardTitle>账号与密码</CardTitle>
            <CardDescription>
              当前管理员：
              <span className="font-medium text-foreground">
                {user ?? "—"}
              </span>
              。修改后需使用新凭据重新登录。
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form className="grid gap-4" onSubmit={handleSubmit}>
              <div className="grid gap-1.5">
                <label htmlFor="cur-pass" className="text-sm font-medium">
                  当前密码
                </label>
                <Input
                  id="cur-pass"
                  type="password"
                  autoComplete="current-password"
                  value={currentPass}
                  onChange={(e) => setCurrentPass(e.target.value)}
                  placeholder="请输入当前密码"
                  required
                />
              </div>
              <div className="grid gap-1.5">
                <label htmlFor="new-user" className="text-sm font-medium">
                  新用户名
                  <span className="ml-1 text-xs text-muted-foreground">
                    （可选，留空不改）
                  </span>
                </label>
                <Input
                  id="new-user"
                  autoComplete="username"
                  value={newUser}
                  onChange={(e) => setNewUser(e.target.value)}
                  placeholder={user ?? "新用户名"}
                />
              </div>
              <div className="grid gap-1.5">
                <label htmlFor="new-pass" className="text-sm font-medium">
                  新密码
                  <span className="ml-1 text-xs text-muted-foreground">
                    （可选，留空不改）
                  </span>
                </label>
                <Input
                  id="new-pass"
                  type="password"
                  autoComplete="new-password"
                  value={newPass}
                  onChange={(e) => setNewPass(e.target.value)}
                  placeholder="请输入新密码"
                />
              </div>
              {error && (
                <p className="text-sm text-destructive" role="alert">
                  {error}
                </p>
              )}
              {success && (
                <p className="text-sm text-online" role="status">
                  {success}
                </p>
              )}
              <Button type="submit" disabled={loading || !!success}>
                {loading ? "提交中…" : "保存修改"}
              </Button>
            </form>
          </CardContent>
        </Card>
      </div>
    </AppShell>
  );
}
