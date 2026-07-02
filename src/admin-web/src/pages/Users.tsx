import { useCallback, useEffect, useState } from "react";
import { KeyRound, ListChecks, Pencil, UserPlus } from "lucide-react";

import { AppShell } from "@/components/shell/app-shell";
import { MenuPermissionEditor } from "@/components/users/menu-permission-editor";
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
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  ASSIGNABLE_MENUS,
  tierLabel,
  type Permission,
} from "@/lib/permissions";
import { useAuthStore, type AdminUser } from "@/store/auth";
import { useStore } from "@/store";

// 已授菜单键 → 中文名 chips（按 ASSIGNABLE_MENUS 顺序稳定展示）
function menuLabels(perms: readonly Permission[]): string[] {
  return ASSIGNABLE_MENUS.filter((m) => perms.includes(m.key)).map(
    (m) => m.label,
  );
}

// 用户管理页：新增账号（含初始菜单勾选器）+ 列表改名 / 配菜单 / 停启用 / 重置密码。
// 需 manage_users 权限 = superadmin 独占，由路由守卫拦截。superadmin 行在 UI 上全锁定
// （改名 / 配菜单 / 停用 / 重置密码 均禁用），与 server 守卫呼应。
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
  const [newPermissions, setNewPermissions] = useState<Permission[]>([]);
  const [creating, setCreating] = useState(false);

  // 行级操作进行中的用户 id：禁用该行按钮，避免重复提交
  const [busyId, setBusyId] = useState<string | null>(null);

  // 行内改名：正在编辑的用户 id + 草稿 + 局部错误
  const [editingNameId, setEditingNameId] = useState<string | null>(null);
  const [nameDraft, setNameDraft] = useState("");
  const [nameError, setNameError] = useState<string | null>(null);

  // 配菜单弹窗
  const [permTarget, setPermTarget] = useState<AdminUser | null>(null);
  const [permDraft, setPermDraft] = useState<Permission[]>([]);
  const [permError, setPermError] = useState<string | null>(null);
  const [permSaving, setPermSaving] = useState(false);

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
        permissions: newPermissions,
      });
      setNewUsername("");
      setNewPassword("");
      setNewPermissions([]);
      setSuccess("已创建账号");
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : "创建用户失败");
    } finally {
      setCreating(false);
    }
  }

  function startRename(u: AdminUser) {
    setEditingNameId(u.id);
    setNameDraft(u.username);
    setNameError(null);
  }

  function cancelRename() {
    setEditingNameId(null);
    setNameError(null);
  }

  async function handleRenameSubmit(u: AdminUser) {
    const name = nameDraft.trim();
    if (!name) {
      setNameError("用户名不能为空");
      return;
    }
    if (name === u.username) {
      cancelRename();
      return;
    }
    setError(null);
    setSuccess(null);
    setNameError(null);
    setBusyId(u.id);
    try {
      await updateUser(u.id, { username: name });
      setSuccess(`已将 ${u.username} 重命名为 ${name}`);
      setEditingNameId(null);
      await reload();
    } catch (err) {
      setNameError(err instanceof Error ? err.message : "改名失败");
    } finally {
      setBusyId(null);
    }
  }

  function openPermEdit(u: AdminUser) {
    setPermTarget(u);
    setPermDraft(u.permissions);
    setPermError(null);
  }

  async function handlePermSubmit() {
    if (!permTarget) return;
    setError(null);
    setSuccess(null);
    setPermError(null);
    setPermSaving(true);
    try {
      await updateUser(permTarget.id, { permissions: permDraft });
      setSuccess(`已更新 ${permTarget.username} 的菜单权限`);
      setPermTarget(null);
      await reload();
    } catch (err) {
      setPermError(err instanceof Error ? err.message : "更新权限失败");
    } finally {
      setPermSaving(false);
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
      <div className="mx-auto flex max-w-5xl flex-col gap-6">
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
              超级管理员由系统内置且唯一，此处创建普通账户并勾选初始菜单权限，之后可在列表随时调整。
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form className="grid gap-4" onSubmit={handleCreate}>
              <div className="grid gap-4 sm:grid-cols-2">
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
              </div>
              <div className="grid gap-2">
                <span className="text-sm font-medium">初始菜单权限</span>
                <MenuPermissionEditor
                  value={newPermissions}
                  onChange={setNewPermissions}
                  disabled={creating}
                />
              </div>
              <div className="flex justify-end">
                <Button type="submit" disabled={creating}>
                  <UserPlus className="size-4" />
                  {creating ? "创建中…" : "新增"}
                </Button>
              </div>
            </form>
          </CardContent>
        </Card>

        {/* 账号列表 */}
        <Card>
          <CardHeader>
            <CardTitle>账号列表</CardTitle>
            <CardDescription>
              停用后该账号无法登录；点击「编辑权限」调整可见菜单，用户名可行内重命名。
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="overflow-hidden rounded-lg border border-border">
              <Table>
                <TableHeader>
                  <TableRow className="border-border hover:bg-transparent">
                    <TableHead className="w-56">用户名</TableHead>
                    <TableHead className="w-28">身份</TableHead>
                    <TableHead>菜单权限</TableHead>
                    <TableHead className="w-20">状态</TableHead>
                    <TableHead className="w-52 text-right">操作</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {users.map((u) => {
                    const isSuperadmin = u.tier === "superadmin";
                    const busy = busyId === u.id;
                    const editing = editingNameId === u.id;
                    const labels = menuLabels(u.permissions);
                    return (
                      <TableRow key={u.id} className="border-border">
                        <TableCell className="font-medium">
                          {editing ? (
                            <div className="flex flex-col gap-1">
                              <div className="flex items-center gap-2">
                                <Input
                                  value={nameDraft}
                                  autoFocus
                                  className="h-7 w-36"
                                  onChange={(e) => setNameDraft(e.target.value)}
                                  onKeyDown={(e) => {
                                    if (e.key === "Enter") {
                                      e.preventDefault();
                                      void handleRenameSubmit(u);
                                    } else if (e.key === "Escape") {
                                      cancelRename();
                                    }
                                  }}
                                />
                                <Button
                                  size="sm"
                                  disabled={busy}
                                  onClick={() => void handleRenameSubmit(u)}
                                >
                                  保存
                                </Button>
                                <Button
                                  size="sm"
                                  variant="outline"
                                  disabled={busy}
                                  onClick={cancelRename}
                                >
                                  取消
                                </Button>
                              </div>
                              {nameError && (
                                <p
                                  className="text-xs text-destructive"
                                  role="alert"
                                >
                                  {nameError}
                                </p>
                              )}
                            </div>
                          ) : (
                            <div className="flex items-center gap-2">
                              <span>{u.username}</span>
                              {!isSuperadmin && (
                                <Button
                                  variant="ghost"
                                  size="icon-sm"
                                  aria-label={`重命名 ${u.username}`}
                                  disabled={busy}
                                  onClick={() => startRename(u)}
                                >
                                  <Pencil className="size-3.5" />
                                </Button>
                              )}
                            </div>
                          )}
                        </TableCell>
                        <TableCell>
                          <span className="text-sm text-muted-foreground">
                            {tierLabel(u.tier)}
                            {isSuperadmin ? "（锁定）" : ""}
                          </span>
                        </TableCell>
                        <TableCell>
                          <div className="flex flex-wrap items-center gap-1.5">
                            {isSuperadmin ? (
                              <Badge variant="secondary">全部（锁定）</Badge>
                            ) : labels.length > 0 ? (
                              labels.map((l) => (
                                <Badge key={l} variant="outline">
                                  {l}
                                </Badge>
                              ))
                            ) : (
                              <span className="text-xs text-muted-foreground">
                                未授权任何菜单
                              </span>
                            )}
                            {!isSuperadmin && (
                              <Button
                                variant="ghost"
                                size="sm"
                                disabled={busy}
                                onClick={() => openPermEdit(u)}
                              >
                                <ListChecks className="size-3.5" />
                                编辑权限
                              </Button>
                            )}
                          </div>
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
                          <div className="flex flex-wrap justify-end gap-2">
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
                        colSpan={5}
                        className="h-24 text-center text-sm text-muted-foreground"
                      >
                        暂无账号
                      </TableCell>
                    </TableRow>
                  )}
                  {loading && users.length === 0 && (
                    <TableRow className="hover:bg-transparent">
                      <TableCell
                        colSpan={5}
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

      {/* 配菜单弹窗 */}
      <Dialog
        open={permTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setPermTarget(null);
            setPermError(null);
          }
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>编辑菜单权限</DialogTitle>
            <DialogDescription>
              勾选{" "}
              <span className="font-medium text-foreground">
                {permTarget?.username}
              </span>{" "}
              可见的功能菜单；「可删除终端」需先启用「终端资产」。
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-4">
            <MenuPermissionEditor
              value={permDraft}
              onChange={setPermDraft}
              disabled={permSaving}
            />
            {permError && (
              <p className="text-sm text-destructive" role="alert">
                {permError}
              </p>
            )}
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setPermTarget(null)}
            >
              取消
            </Button>
            <Button
              type="button"
              disabled={permSaving}
              onClick={() => void handlePermSubmit()}
            >
              {permSaving ? "保存中…" : "保存"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

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
