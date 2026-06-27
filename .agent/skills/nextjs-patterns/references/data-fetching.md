## Data Fetching Patterns

### Server-Side Data Fetching

Server Components 可以直接 fetch 数据，无需 API Routes。

```tsx
// ✅ Good: 在 Server Component 中直接 fetch
// app/posts/page.tsx
async function PostsPage() {
  const posts = await db.post.findMany({
    orderBy: { createdAt: 'desc' },
  });

  return <PostList posts={posts} />;
}

// ✅ Good: 使用 fetch 并配置缓存
async function getPost(id: string) {
  const res = await fetch(`https://api.example.com/posts/${id}`, {
    // 缓存策略
    next: { revalidate: 3600 }, // 每小时重新验证
  });

  if (!res.ok) {
    throw new Error('Failed to fetch post');
  }

  return res.json();
}

// ✅ Good: 并行数据获取
async function UserProfile({ id }: { id: string }) {
  // 并行 fetch，而非顺序
  const [user, posts] = await Promise.all([
    getUser(id),
    getUserPosts(id),
  ]);

  return (
    <div>
      <UserInfo user={user} />
      <UserPosts posts={posts} />
    </div>
  );
}

// ❌ Bad: 顺序 fetch（性能差）
async function BadUserProfile({ id }: { id: string }) {
  const user = await getUser(id);
  const posts = await getUserPosts(id);  // 等待上一个完成
  // ...
}
```

### Caching and Revalidation

Next.js 15 提供多层缓存机制。

```tsx
// ✅ Good: 静态数据（构建时生成）
async function StaticPage() {
  // 默认缓存，构建时生成
  const data = await fetch('https://api.example.com/static');
  return <div>{/* ... */}</div>;
}

// ✅ Good: 定时重新验证（ISR）
async function RevalidatedPage() {
  const data = await fetch('https://api.example.com/data', {
    next: { revalidate: 60 }, // 每 60 秒重新验证
  });
  return <div>{/* ... */}</div>;
}

// ✅ Good: 动态数据（每次请求）
async function DynamicPage() {
  const data = await fetch('https://api.example.com/dynamic', {
    cache: 'no-store', // 不缓存，每次请求都 fetch
  });
  return <div>{/* ... */}</div>;
}

// ✅ Good: 按需重新验证（需要时手动触发）
// app/actions.ts
'use server'

import { revalidatePath, revalidateTag } from 'next/cache';

export async function updatePost(id: string) {
  await db.post.update({ where: { id }, data: { /* ... */ } });

  // 重新验证特定路径
  revalidatePath('/posts');
  revalidatePath(`/posts/${id}`);
}

export async function updatePostByTag(id: string) {
  await db.post.update({ where: { id }, data: { /* ... */ } });

  // 重新验证特定标签
  revalidateTag('posts');
}

// Fetch 时添加标签
async function getPostsWithTags() {
  const res = await fetch('https://api.example.com/posts', {
    next: { tags: ['posts'] },
  });
  return res.json();
}
```

### Loading and Streaming

使用 Suspense 和 Streaming 提升用户体验。

```tsx
// ✅ Good: loading.tsx 文件（自动 Suspense 边界）
// app/dashboard/loading.tsx
export default function Loading() {
  return <Spinner />;
}

// ✅ Good: 使用 Suspense 分段加载
// app/dashboard/page.tsx
import { Suspense } from 'react';

export default function DashboardPage() {
  return (
    <div>
      <Header />

      {/* 快速内容立即显示 */}
      <Stats />

      {/* 慢速内容独立加载 */}
      <Suspense fallback={<ChartSkeleton />}>
        <Chart />
      </Suspense>

      <Suspense fallback={<RecentActivitySkeleton />}>
        <RecentActivity />
      </Suspense>
    </div>
  );
}

// Chart 是异步组件
async function Chart() {
  const data = await fetchChartData(); // 可能较慢
  return <ChartComponent data={data} />;
}

// ❌ Bad: 单个 Suspense 包裹所有内容
export default function BadDashboardPage() {
  return (
    <Suspense fallback={<PageSkeleton />}>
      {/* 所有内容等待最慢的部分 */}
      <Header />
      <Stats />
      <Chart />
      <RecentActivity />
    </Suspense>
  );
}
```
