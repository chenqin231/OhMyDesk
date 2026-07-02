import { AppShell } from "@/components/shell/app-shell";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

// 用户管理页占位（Task 6）——真实的用户增删改查 / 角色调整由 Task 7 实现替换本文件。
// 现只做占位，让 /users 路由与「用户管理」菜单入口闭环可达，不留死链。
export function Users() {
  return (
    <AppShell title="用户管理">
      <div className="mx-auto max-w-lg">
        <Card>
          <CardHeader>
            <CardTitle>用户管理</CardTitle>
            <CardDescription>功能建设中，敬请期待。</CardDescription>
          </CardHeader>
          <CardContent className="text-sm text-muted-foreground">
            用户列表与角色分配即将上线。
          </CardContent>
        </Card>
      </div>
    </AppShell>
  );
}
