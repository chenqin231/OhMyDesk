## Server Actions

Server Actions 提供类型安全的客户端-服务器通信。

### Basic Server Actions

```tsx
// ✅ Good: Server Action（app/actions.ts）
'use server'

import { z } from 'zod';
import { revalidatePath } from 'next/cache';

const createPostSchema = z.object({
  title: z.string().min(1).max(100),
  content: z.string().min(1),
});

export async function createPost(formData: FormData) {
  // 验证输入
  const rawData = {
    title: formData.get('title'),
    content: formData.get('content'),
  };

  const result = createPostSchema.safeParse(rawData);

  if (!result.success) {
    return {
      error: 'Validation failed',
      details: result.error.flatten(),
    };
  }

  // 创建文章
  const post = await db.post.create({
    data: result.data,
  });

  // 重新验证缓存
  revalidatePath('/posts');

  return { success: true, post };
}

// ✅ Good: 在 Client Component 中使用
'use client'

export function CreatePostForm() {
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(formData: FormData) {
    const result = await createPost(formData);

    if ('error' in result) {
      setError(result.error);
    } else {
      // 重定向到新文章
      window.location.href = `/posts/${result.post.id}`;
    }
  }

  return (
    <form action={handleSubmit}>
      <input name="title" required />
      <textarea name="content" required />
      <button type="submit">Create Post</button>
      {error && <p className="error">{error}</p>}
    </form>
  );
}
```

### Server Actions Security

```tsx
// ✅ Good: 验证用户身份
'use server'

import { auth } from '@/lib/auth';
import { db } from '@/lib/db';

export async function deletePost(postId: string) {
  // 验证用户登录
  const session = await auth();

  if (!session?.user) {
    throw new Error('Unauthorized');
  }

  // 验证权限
  const post = await db.post.findUnique({
    where: { id: postId },
  });

  if (post?.authorId !== session.user.id) {
    throw new Error('Forbidden');
  }

  // 执行删除
  await db.post.delete({ where: { id: postId } });

  revalidatePath('/posts');

  return { success: true };
}

// ❌ Bad: 未验证权限（安全漏洞！）
'use server'

export async function badDeletePost(postId: string) {
  // 危险！任何人都可以删除任何文章
  await db.post.delete({ where: { id: postId } });
}
```
