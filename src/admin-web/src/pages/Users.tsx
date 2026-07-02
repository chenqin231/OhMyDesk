import { useCallback, useEffect, useState } from "react";
import { KeyRound, UserPlus } from "lucide-react";

import { AppShell } from "@/components/shell/app-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { roleLabel, type Role } from "@/lib/permissions";
import { useAuthStore, type AdminUser } from "@/store/auth";
import { useStore } from "@/store";

// 可分配角色：不含 superadmin —— 超级管理员由 server bootstrap 内置且唯一。
const ASSIGNABLE_ROLES: Role[] = ["admin", "operator", "auditor"];

// 用户管理页：新增账号 + 列表改角色 / 停启用 / 重置密码（需 manage_users 权限，
// 由路由守卫拦截）。superadmin 行在 UI 上锁定，与 server 守卫呼应。
export function Users() {
  const listUsers = useAuthStore((s) => s.listUsers);
  const createUser = useAuthStore((s) => s.createUser);
  const updateUser = useAuthStore((s) => s.updateUser);
  const resetUserPassword = useAuthStore((s) => s.resetUserPassword);

  const endpoints = useStore((s) => s.endpoints);
  const online = endpoints.filter((ep) => ep.online).length;
  const total = endpoints.length;

  const [users, setUsers] = useState<AdminUser[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  // 新增表单
  const [newUsername, setNewUsername] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [newRole, setNewRole] = useState<Role>("operator");
  const [creating, setCreating] = useState(false);

  // 行级操作进行中的用户 id：禁用该行按钮，避免重复提交
  const [busyId, setBusyId] = useState<string | null>(null);

  // 重置密码弹窗
  const [resetTarget, setResetTarget] = useState<AdminUser | null>(null);
  const [resetPassword, setResetPassword] = useState("");
  const [resetError, setResetError] = useState<string | null>(null);
  const [resetting, setResetting] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setUsers(await listUsers());
    } catch (err) {
      setError(err instanceof Error ? err.message : "加载用户列表失败");
    } finally {
      setLoading(false);
    }
  }, [listUsers]);

  useEffect(() => {
    void reload();
  }, [reload]);

  async function handleCreate(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setSuccess(null);
    if (!newUsername.trim() || !newPassword) {
      setError("请填写用户名与初始密码");
      return;
    }
    setCreating(true);
    try {
      await createUser({
        username: newUsername.trim(),
        password: newPassword,
        role: newRole,
        enabled: true,
      });
      setNewUsername("");
      setNewPassword("");
      setNewRole("operator");
      setSuccess("已创建账号");
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : "创建用户失败");
    } finally {
      setCreating(false);
    }
  }

  async function handleRoleChange(u: AdminUser, role: Role) {
    if (role === u.role) return;
    setError(null);
    setSuccess(null);
    setBusyId(u.id);
    try {
      await updateUser(u.id, { role });
      setSuccess(`已将 ${u.username} 的角色改为${roleLabel(role)}`);
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : "更新用户失败");
    } finally {
      setBusyId(null);
    }
  }

  async function handleToggleEnabled(u: AdminUser) {
    setError(null);
    setSuccess(null);
    setBusyId(u.id);
    try {
      await updateUser(u.id, { enabled: !u.enabled });
      setSuccess(`已${u.enabled ? "停用" : "启用"} ${u.username}`);
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : "更新用户失败");
    } finally {
      setBusyId(null);
    }
  }

  function openReset(u: AdminUser) {
    setResetTarget(u);
    setResetPassword("");
    setResetError(null);
  }

  async function handleResetSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!resetTarget) return;
    setResetError(null);
    if (!resetPassword) {
      setResetError("请输入新密码");
      return;
    }
    setResetting(true);
    try {
      await resetUserPassword(resetTarget.id, resetPassword);
      setSuccess(`已重置 ${resetTarget.username} 的密码`);
      setResetTarget(null);
      setResetPassword("");
    } catch (err) {
      setResetError(err instanceof Error ? err.message : "重置密码失败");
    } finally {
      setResetting(false);
    }
  }

  return (
    <AppShell title="用户管理" online={online} total={total}>
      <div className="mx-auto flex max-w-4xl flex-col gap-6">
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

        {/* 新增账号 */}
        <Card>
          <CardHeader>
            <CardTitle>新增账号</CardTitle>
            <CardDescription>
              超级管理员由系统内置且唯一，此处仅可创建管理员 / 操作员 / 审计员。
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form
              className="grid gap-4 sm:grid-cols-[1fr_1fr_auto_auto] sm:items-end"
              onSubmit={handleCreate}
            >
              <div className="grid gap-1.5">
                <label htmlFor="new-username" className="text-sm font-medium">
                  用户名
                </label>
                <Input
                  id="new-username"
                  autoComplete="off"
                  value={newUsername}
                  onChange={(e) => setNewUsername(e.target.value)}
                  placeholder="登录用户名"
                />
              </div>
              <div className="grid gap-1.5">
                <label htmlFor="new-password" className="text-sm font-medium">
                  初始密码
                </label>
                <Input
                  id="new-password"
                  type="password"
                  autoComplete="new-password"
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  placeholder="初始密码"
                />
              </div>
              <div className="grid gap-1.5">
                <span className="text-sm font-medium">角色</span>
                <Select
                  value={newRole}
                  onValueChange={(v) => setNewRole(v as Role)}
                >
                  <SelectTrigger className="w-32">
                    <SelectValue>{roleLabel(newRole)}</SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    {ASSIGNABLE_ROLES.map((r) => (
                      <SelectItem key={r} value={r}>
                        {roleLabel(r)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <Button type="submit" disabled={creating}>
                <UserPlus className="size-4" />
                {creating ? "创建中…" : "新增"}
              </Button>
            </form>
          </CardContent>
        </Card>

        {/* 账号列表 */}
        <Card>
          <CardHeader>
            <CardTitle>账号列表</CardTitle>
            <CardDescription>
              调整角色即时生效；停用后该账号无法登录。
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="overflow-hidden rounded-lg border border-border">
              <Table>
                <TableHeader>
                  <TableRow className="border-border hover:bg-transparent">
                    <TableHead>用户名</TableHead>
                    <TableHead className="w-40">角色</TableHead>
                    <TableHead className="w-24">状态</TableHead>
                    <TableHead className="w-64 text-right">操作</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {users.map((u) => {
                    const isSuperadmin = u.role === "superadmin";
                    const busy = busyId === u.id;
                    return (
                      <TableRow key={u.id} className="border-border">
                        <TableCell className="font-medium">
                          {u.username}
                        </TableCell>
                        <TableCell>
                          {isSuperadmin ? (
                            <span className="text-sm text-muted-foreground">
                              超级管理员（锁定）
                            </span>
                          ) : (
                            <Select
                              value={u.role}
                              onValueChange={(v) =>
                                void handleRoleChange(u, v as Role)
                              }
                              disabled={busy}
                            >
                              <SelectTrigger className="w-32">
                                <SelectValue>{roleLabel(u.role)}</SelectValue>
                              </SelectTrigger>
                              <SelectContent>
                                {ASSIGNABLE_ROLES.map((r) => (
                                  <SelectItem key={r} value={r}>
                                    {roleLabel(r)}
                                  </SelectItem>
                                ))}
                              </SelectContent>
                            </Select>
                          )}
                        </TableCell>
                        <TableCell>
                          {u.enabled ? (
                            <Badge variant="outline" className="text-online">
                              启用
                            </Badge>
                          ) : (
                            <Badge
                              variant="outline"
                              className="text-muted-foreground"
                            >
                              停用
                            </Badge>
                          )}
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="flex justify-end gap-2">
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={isSuperadmin || busy}
                              onClick={() => openReset(u)}
                            >
                              <KeyRound className="size-3.5" />
                              重置密码
                            </Button>
                            <Button
                              variant={u.enabled ? "destructive" : "outline"}
                              size="sm"
                              disabled={isSuperadmin || busy}
                              onClick={() => void handleToggleEnabled(u)}
                            >
                              {u.enabled ? "停用" : "启用"}
                            </Button>
                          </div>
                        </TableCell>
                      </TableRow>
                    );
                  })}
                  {!loading && users.length === 0 && (
                    <TableRow className="hover:bg-transparent">
                      <TableCell
                        colSpan={4}
                        className="h-24 text-center text-sm text-muted-foreground"
                      >
                        暂无账号
                      </TableCell>
                    </TableRow>
                  )}
                  {loading && users.length === 0 && (
                    <TableRow className="hover:bg-transparent">
                      <TableCell
                        colSpan={4}
                        className="h-24 text-center text-sm text-muted-foreground"
                      >
                        加载中…
                      </TableCell>
                    </TableRow>
                  )}
                </TableBody>
              </Table>
            </div>
          </CardContent>
        </Card>
      </div>

      {/* 重置密码弹窗 */}
      <Dialog
        open={resetTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setResetTarget(null);
            setResetError(null);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>重置密码</DialogTitle>
            <DialogDescription>
              为账号{" "}
              <span className="font-medium text-foreground">
                {resetTarget?.username}
              </span>{" "}
              设置新密码。
            </DialogDescription>
          </DialogHeader>
          <form className="grid gap-4" onSubmit={handleResetSubmit}>
            <div className="grid gap-1.5">
              <label htmlFor="reset-password" className="text-sm font-medium">
                新密码
              </label>
              <Input
                id="reset-password"
                type="password"
                autoComplete="new-password"
                value={resetPassword}
                onChange={(e) => setResetPassword(e.target.value)}
                placeholder="请输入新密码"
              />
            </div>
            {resetError && (
              <p className="text-sm text-destructive" role="alert">
                {resetError}
              </p>
            )}
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setResetTarget(null)}
              >
                取消
              </Button>
              <Button type="submit" disabled={resetting}>
                {resetting ? "提交中…" : "确认重置"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </AppShell>
  );
}
