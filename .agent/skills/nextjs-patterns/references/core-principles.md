## When to Activate

- 开发 Next.js 应用
- 审查 Next.js 代码
- 迁移到 App Router
- 优化 Next.js 性能
- 设计 Next.js 架构

## Core Principles

### 1. App Router First (Next.js 13+)

App Router 是推荐的路由系统，提供更好的性能和开发体验。

```tsx
// ✅ Good: App Router 结构
app/
├── layout.tsx          // 根布局
├── page.tsx            // 首页
├── error.tsx           // 错误处理
├── loading.tsx         // 加载状态
├── not-found.tsx       // 404 页面
├── (auth)/             // 路由组（不影响 URL）
│   ├── login/
│   │   └── page.tsx
│   └── register/
│       └── page.tsx
├── dashboard/
│   ├── layout.tsx      // Dashboard 布局
│   ├── page.tsx
│   └── settings/
│       └── page.tsx
└── api/                // API Routes
    └── users/
        └── route.ts

// ❌ Bad: Pages Router（旧系统，避免混用）
pages/
├── index.tsx
├── dashboard.tsx
└── api/
    └── users.ts
```

### 2. Server Components by Default

默认所有组件都是 Server Components，仅在需要交互时使用 Client Components。

```tsx
// ✅ Good: Server Component（默认）
// app/users/page.tsx
async function UsersPage() {
  // 可以直接访问数据库、文件系统等
  const users = await db.user.findMany();

  return (
    <div>
      <h1>Users</h1>
      <UserList users={users} />
    </div>
  );
}

export default UsersPage;

// ✅ Good: Client Component（需要交互）
// app/components/UserCard.tsx
'use client'

import { useState } from 'react';

export function UserCard({ user }: { user: User }) {
  const [liked, setLiked] = useState(false);

  return (
    <div onClick={() => setLiked(!liked)}>
      {user.name} {liked && '❤️'}
    </div>
  );
}

// ❌ Bad: 不必要的 Client Component
'use client'  // 不需要交互，应该是 Server Component

export function UserList({ users }: { users: User[] }) {
  return (
    <ul>
      {users.map(user => <li key={user.id}>{user.name}</li>)}
    </ul>
  );
}
```

### 3. Colocation and Organization

将相关文件放在同一目录，提高可维护性。

```tsx
// ✅ Good: 文件就近放置
app/
├── dashboard/
│   ├── page.tsx              // 页面
│   ├── layout.tsx            // 布局
│   ├── loading.tsx           // 加载状态
│   ├── error.tsx             // 错误边界
│   ├── components/           // 本地组件
│   │   ├── DashboardChart.tsx
│   │   └── StatsCard.tsx
│   └── actions.ts            // Server Actions
└── components/               // 全局共享组件
    ├── Header.tsx
    └── Footer.tsx

// ❌ Bad: 组件和页面分离过远
app/
├── dashboard/
│   └── page.tsx
components/
└── dashboard/
    ├── DashboardChart.tsx
    └── StatsCard.tsx
```
