## Utility Types

```typescript
// ✅ Good: Awaited（提取 Promise 类型）
type A = Awaited<Promise<string>>;  // string
type B = Awaited<Promise<Promise<number>>>;  // number

// ✅ Good: ReturnType（提取函数返回类型）
function getUser() {
  return { id: 1, name: "Alice" };
}

type User = ReturnType<typeof getUser>;

// ✅ Good: Parameters（提取函数参数类型）
function createUser(name: string, age: number) {
  // ...
}

type CreateUserParams = Parameters<typeof createUser>;  // [string, number]

// ✅ Good: Exclude（排除联合类型成员）
type T = Exclude<"a" | "b" | "c", "a">;  // "b" | "c"

// ✅ Good: Extract（提取联合类型成员）
type T = Extract<"a" | "b" | "c", "a" | "f">;  // "a"

// ✅ Good: NonNullable（排除 null 和 undefined）
type T = NonNullable<string | number | null | undefined>;  // string | number
```

## Type Safety Patterns

### Discriminated Unions for State Management

```typescript
// ✅ Good: 使用辨识联合类型管理状态
type AsyncState<T> =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "success"; data: T }
  | { status: "error"; error: string };

function renderUser(state: AsyncState<User>) {
  switch (state.status) {
    case "idle":
      return <div>Not started</div>;

    case "loading":
      return <div>Loading...</div>;

    case "success":
      // TypeScript 知道 state.data 存在！
      return <div>User: {state.data.name}</div>;

    case "error":
      // TypeScript 知道 state.error 存在！
      return <div>Error: {state.error}</div>;
  }
}

// ❌ Bad: 使用可选属性（类型不安全）
type BadAsyncState<T> = {
  status: "idle" | "loading" | "success" | "error";
  data?: T;
  error?: string;
};

function badRenderUser(state: BadAsyncState<User>) {
  if (state.status === "success") {
    // data 可能是 undefined！需要额外检查
    console.log(state.data?.name);
  }
}
```

### Builder Pattern with Type Safety

```typescript
// ✅ Good: 类型安全的构建器模式
type Query<T = {}> = {
  where: <K extends string>(field: K, value: any) => Query<T & { where: true }>;
  orderBy: <K extends string>(field: K) => Query<T & { orderBy: true }>;
  limit: (n: number) => Query<T & { limit: true }>;
  execute: T extends { where: true } ? () => Promise<any[]> : never;
};

function createQuery<T>(): Query<T> {
  // 实现...
}

// 使用
const results = await createQuery()
  .where("name", "Alice")
  .orderBy("createdAt")
  .limit(10)
  .execute();  // ✅ 可以调用

// const invalid = await createQuery()
//   .orderBy("createdAt")
//   .execute();  // ❌ 错误：必须先调用 where
```

### Branded Types

```typescript
// ✅ Good: 使用 Branded Types 防止混淆
type UserId = string & { __brand: "UserId" };
type PostId = string & { __brand: "PostId" };

function createUserId(id: string): UserId {
  return id as UserId;
}

function createPostId(id: string): PostId {
  return id as PostId;
}

function getUser(id: UserId) {
  // ...
}

function getPost(id: PostId) {
  // ...
}

const userId = createUserId("user-123");
const postId = createPostId("post-456");

getUser(userId);   // ✅
// getUser(postId);   // ❌ 类型错误！

// ❌ Bad: 使用普通 string（容易混淆）
function badGetUser(id: string) { }
function badGetPost(id: string) { }

badGetUser("post-456");  // ✅ 编译通过，但可能是错误！
```
