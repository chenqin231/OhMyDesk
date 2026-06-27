## Testing Patterns

### Component Testing

```tsx
// ✅ Good: 测试 Server Component
// __tests__/PostPage.test.tsx
import { render, screen } from '@testing-library/react';
import PostPage from '@/app/posts/[id]/page';

// Mock 数据库
jest.mock('@/lib/db', () => ({
  post: {
    findUnique: jest.fn(),
  },
}));

describe('PostPage', () => {
  it('renders post content', async () => {
    const mockPost = {
      id: '1',
      title: 'Test Post',
      content: 'Test content',
    };

    (db.post.findUnique as jest.Mock).mockResolvedValue(mockPost);

    const Component = await PostPage({ params: { id: '1' } });
    render(Component);

    expect(screen.getByText('Test Post')).toBeInTheDocument();
  });
});
```

### E2E Testing with Playwright

```typescript
// ✅ Good: E2E 测试
// e2e/posts.spec.ts
import { test, expect } from '@playwright/test';

test('create and view post', async ({ page }) => {
  // 导航到创建页面
  await page.goto('/posts/create');

  // 填写表单
  await page.fill('input[name="title"]', 'Test Post');
  await page.fill('textarea[name="content"]', 'Test content');

  // 提交
  await page.click('button[type="submit"]');

  // 验证跳转和内容
  await expect(page).toHaveURL(/\/posts\/[\w-]+/);
  await expect(page.locator('h1')).toHaveText('Test Post');
});
```

## Anti-Patterns to Avoid

```tsx
// ❌ Bad: 在 Server Component 中使用 hooks
async function BadServerComponent() {
  const [state, setState] = useState(0); // 错误！Server Component 不能用 hooks
  const data = await fetchData();
  return <div>{data}</div>;
}

// ❌ Bad: 在 Client Component 中直接访问数据库
'use client'

function BadClientComponent() {
  const users = await db.user.findMany(); // 错误！Client Component 不能直接访问数据库
  return <div>{/* ... */}</div>;
}

// ❌ Bad: 混用 App Router 和 Pages Router
// 避免在同一项目中混用两种路由系统

// ❌ Bad: 过度使用 'use client'
'use client' // 不必要！这个组件没有交互

function StaticList({ items }: { items: string[] }) {
  return (
    <ul>
      {items.map(item => <li key={item}>{item}</li>)}
    </ul>
  );
}

// ❌ Bad: 忽略类型安全
// app/api/users/route.ts
export async function POST(request: Request) {
  const body = await request.json();
  // 没有验证！直接使用 body
  await db.user.create({ data: body }); // 危险！
}
```

## Quick Reference

| 特性 | 使用场景 |
|------|---------|
| **Server Components** | 默认，数据获取、静态内容 |
| **Client Components** | 交互、hooks、浏览器 API |
| **Server Actions** | 表单提交、数据变更 |
| **Route Handlers** | RESTful API、Webhooks |
| **Middleware** | 认证、重定向、请求修改 |
| **Suspense** | 加载状态、流式渲染 |
| **Dynamic Imports** | 代码分割、按需加载 |
| **Image Component** | 图片优化、懒加载 |
| **Metadata API** | SEO、Open Graph |

## Configuration Best Practices

```typescript
// ✅ Good: next.config.js
/** @type {import('next').NextConfig} */
const nextConfig = {
  // 启用严格模式
  reactStrictMode: true,

  // 图片优化配置
  images: {
    domains: ['example.com', 'cdn.example.com'],
    formats: ['image/avif', 'image/webp'],
  },

  // 环境变量验证
  env: {
    NEXT_PUBLIC_API_URL: process.env.NEXT_PUBLIC_API_URL,
  },

  // 重定向
  async redirects() {
    return [
      {
        source: '/old-path',
        destination: '/new-path',
        permanent: true,
      },
    ];
  },

  // 安全头
  async headers() {
    return [
      {
        source: '/(.*)',
        headers: [
          {
            key: 'X-Frame-Options',
            value: 'DENY',
          },
          {
            key: 'X-Content-Type-Options',
            value: 'nosniff',
          },
          {
            key: 'Referrer-Policy',
            value: 'strict-origin-when-cross-origin',
          },
        ],
      },
    ];
  },
};

module.exports = nextConfig;
```

**记住**: Next.js 的强大在于其约定优于配置的设计哲学。遵循官方推荐的模式，充分利用 Server Components 和流式渲染，构建快速、可扩展的 React 应用。
