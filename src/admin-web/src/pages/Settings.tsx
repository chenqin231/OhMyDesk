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

// 个人设置页：任意已登录用户自助修改自己的密码（旧密码 + 新密码 + 确认）。
// 仅需登录，不依赖任何功能菜单权限。成功后强制登出并跳登录，用新密码重新登录。
export function Settings() {
  const navigate = useNavigate();
  const user = useAuthStore((s) => s.user);
  const loadMe = useAuthStore((s) => s.loadMe);
  const changeOwnPassword = useAuthStore((s) => s.changeOwnPassword);
  const logout = useAuthStore((s) => s.logout);

  const endpoints = useStore((s) => s.endpoints);
  const online = endpoints.filter((ep) => ep.online).length;
  const total = endpoints.length;

  const [oldPass, setOldPass] = useState("");
  const [newPass, setNewPass] = useState("");
  const [confirmPass, setConfirmPass] = useState("");
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
    if (!oldPass || !newPass) {
      setError("请填写旧密码与新密码");
      return;
    }
    if (newPass !== confirmPass) {
      setError("两次输入的新密码不一致");
      return;
    }
    setLoading(true);
    try {
      await changeOwnPassword(oldPass, newPass);
      setSuccess("密码已修改，请用新密码重新登录");
      // 密码已变更，强制登出回登录页
      setTimeout(() => {
        logout();
        navigate("/login", { replace: true });
      }, 1200);
    } catch (err) {
      setError(err instanceof Error ? err.message : "修改密码失败");
    } finally {
      setLoading(false);
    }
  }

  return (
    <AppShell title="个人设置" online={online} total={total}>
      <div className="mx-auto max-w-lg">
        <Card>
          <CardHeader>
            <CardTitle>修改密码</CardTitle>
            <CardDescription>
              当前用户：
              <span className="font-medium text-foreground">
                {user ?? "—"}
              </span>
              。修改后需使用新密码重新登录。
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form className="grid gap-4" onSubmit={handleSubmit}>
              <div className="grid gap-1.5">
                <label htmlFor="old-pass" className="text-sm font-medium">
                  旧密码
                </label>
                <Input
                  id="old-pass"
                  type="password"
                  autoComplete="current-password"
                  value={oldPass}
                  onChange={(e) => setOldPass(e.target.value)}
                  placeholder="请输入当前密码"
                  required
                />
              </div>
              <div className="grid gap-1.5">
                <label htmlFor="new-pass" className="text-sm font-medium">
                  新密码
                </label>
                <Input
                  id="new-pass"
                  type="password"
                  autoComplete="new-password"
                  value={newPass}
                  onChange={(e) => setNewPass(e.target.value)}
                  placeholder="请输入新密码"
                  required
                />
              </div>
              <div className="grid gap-1.5">
                <label htmlFor="confirm-pass" className="text-sm font-medium">
                  确认新密码
                </label>
                <Input
                  id="confirm-pass"
                  type="password"
                  autoComplete="new-password"
                  value={confirmPass}
                  onChange={(e) => setConfirmPass(e.target.value)}
                  placeholder="请再次输入新密码"
                  required
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
                {loading ? "提交中…" : "修改密码"}
              </Button>
            </form>
          </CardContent>
        </Card>
      </div>
    </AppShell>
  );
}
