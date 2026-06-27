## Metadata and SEO

### Static Metadata

```tsx
// ✅ Good: 静态元数据
// app/page.tsx
import { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Home | My App',
  description: 'Welcome to my awesome app',
  keywords: ['next.js', 'react', 'app'],
  authors: [{ name: 'Your Name' }],
  openGraph: {
    title: 'Home | My App',
    description: 'Welcome to my awesome app',
    images: ['/og-image.png'],
  },
  twitter: {
    card: 'summary_large_image',
    title: 'Home | My App',
    description: 'Welcome to my awesome app',
    images: ['/twitter-image.png'],
  },
};

export default function HomePage() {
  return <div>Home Page</div>;
}
```

### Dynamic Metadata

```tsx
// ✅ Good: 动态元数据
// app/posts/[id]/page.tsx
import { Metadata } from 'next';

export async function generateMetadata({
  params
}: {
  params: { id: string }
}): Promise<Metadata> {
  const post = await db.post.findUnique({
    where: { id: params.id },
  });

  if (!post) {
    return {
      title: 'Post Not Found',
    };
  }

  return {
    title: post.title,
    description: post.excerpt,
    openGraph: {
      title: post.title,
      description: post.excerpt,
      images: [post.coverImage],
    },
  };
}

export default async function PostPage({ params }: { params: { id: string } }) {
  // ...
}
```
