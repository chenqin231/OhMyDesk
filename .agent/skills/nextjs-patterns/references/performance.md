## Performance Optimization

### Image Optimization

```tsx
// ✅ Good: 使用 next/image
import Image from 'next/image';

export function ProductCard({ product }: { product: Product }) {
  return (
    <div>
      <Image
        src={product.imageUrl}
        alt={product.name}
        width={300}
        height={200}
        // 优先级高的图片（首屏可见）
        priority
        // 或使用懒加载（默认）
        // loading="lazy"
      />
      <h3>{product.name}</h3>
    </div>
  );
}

// ❌ Bad: 使用原生 <img>（失去优化）
export function BadProductCard({ product }: { product: Product }) {
  return (
    <div>
      <img src={product.imageUrl} alt={product.name} />
      <h3>{product.name}</h3>
    </div>
  );
}
```

### Font Optimization

```tsx
// ✅ Good: app/layout.tsx
import { Inter, Roboto_Mono } from 'next/font/google';

// 优化的 Google Fonts 加载
const inter = Inter({
  subsets: ['latin'],
  display: 'swap',
  variable: '--font-inter',
});

const robotoMono = Roboto_Mono({
  subsets: ['latin'],
  display: 'swap',
  variable: '--font-roboto-mono',
});

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" className={`${inter.variable} ${robotoMono.variable}`}>
      <body className={inter.className}>{children}</body>
    </html>
  );
}

// CSS 中使用
// globals.css
// body {
//   font-family: var(--font-inter);
// }
//
// code {
//   font-family: var(--font-roboto-mono);
// }
```

### Code Splitting and Dynamic Imports

```tsx
// ✅ Good: 动态导入（按需加载）
import dynamic from 'next/dynamic';

// 组件懒加载
const HeavyChart = dynamic(() => import('@/components/HeavyChart'), {
  loading: () => <ChartSkeleton />,
  ssr: false, // 仅客户端渲染（可选）
});

export function DashboardPage() {
  return (
    <div>
      <h1>Dashboard</h1>
      {/* 仅在需要时加载 */}
      <HeavyChart />
    </div>
  );
}

// ✅ Good: 动态导入多个组件
const DynamicComponents = {
  Map: dynamic(() => import('@/components/Map')),
  Chart: dynamic(() => import('@/components/Chart')),
  Table: dynamic(() => import('@/components/Table')),
};

export function FlexibleView({ type }: { type: keyof typeof DynamicComponents }) {
  const Component = DynamicComponents[type];
  return <Component />;
}

// ❌ Bad: 导入所有组件（包体积大）
import { Map } from '@/components/Map';
import { Chart } from '@/components/Chart';
import { Table } from '@/components/Table';
```

### Static and Dynamic Rendering

```tsx
// ✅ Good: 静态页面（默认）
// app/about/page.tsx
export default function AboutPage() {
  return <div>About Us</div>;
}

// ✅ Good: 动态页面（使用动态函数）
// app/time/page.tsx
export default function TimePage() {
  // headers()、cookies() 等会使页面变为动态
  const userAgent = headers().get('user-agent');

  return (
    <div>
      <p>Current time: {new Date().toISOString()}</p>
      <p>Your browser: {userAgent}</p>
    </div>
  );
}

// ✅ Good: 静态生成 + 动态路径
// app/posts/[id]/page.tsx
export async function generateStaticParams() {
  const posts = await db.post.findMany();

  return posts.map((post) => ({
    id: post.id,
  }));
}

export default async function PostPage({ params }: { params: { id: string } }) {
  const post = await db.post.findUnique({
    where: { id: params.id },
  });

  return <Post post={post} />;
}

// ✅ Good: 强制静态生成
export const dynamic = 'force-static';

// ✅ Good: 强制动态渲染
export const dynamic = 'force-dynamic';

// ✅ Good: 配置重新验证时间
export const revalidate = 3600; // 每小时
```
