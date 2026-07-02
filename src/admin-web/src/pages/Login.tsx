import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { ShieldCheck } from "lucide-react";

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
import { defaultPathForPermissions } from "@/lib/permissions";

// 登录页：未登录时唯一可达页。登录成功按权限跳默认页，401 提示账号或密码错误。
export function Login() {
  const navigate = useNavigate();
  const login = useAuthStore((s) => s.login);

  const [user, setUser] = useState("");
  const [pass, setPass] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setLoading(true);
    try {
      await login(user.trim(), pass);
      // 登录成功后 store 已写入 permissions，按权限跳到对应默认页
      const perms = useAuthStore.getState().permissions;
      navigate(defaultPathForPermissions(perms), { replace: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : "登录失败");
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex min-h-svh items-center justify-center bg-background p-4">
      <Card className="w-full max-w-sm">
        <CardHeader className="items-center text-center">
          <div className="flex aspect-square size-10 items-center justify-center rounded-md bg-primary text-primary-foreground">
            <ShieldCheck className="size-6" />
          </div>
          <CardTitle className="mt-2">信创终端管控平台</CardTitle>
          <CardDescription>请使用管理员账号登录控制台</CardDescription>
        </CardHeader>
        <CardContent>
          <form className="grid gap-4" onSubmit={handleSubmit}>
            <div className="grid gap-1.5">
              <label htmlFor="login-user" className="text-sm font-medium">
                用户名
              </label>
              <Input
                id="login-user"
                autoComplete="username"
                value={user}
                onChange={(e) => setUser(e.target.value)}
                placeholder="请输入用户名"
                required
                autoFocus
              />
            </div>
            <div className="grid gap-1.5">
              <label htmlFor="login-pass" className="text-sm font-medium">
                密码
              </label>
              <Input
                id="login-pass"
                type="password"
                autoComplete="current-password"
                value={pass}
                onChange={(e) => setPass(e.target.value)}
                placeholder="请输入密码"
                required
              />
            </div>
            {error && (
              <p className="text-sm text-destructive" role="alert">
                {error}
              </p>
            )}
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? "登录中…" : "登录"}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
