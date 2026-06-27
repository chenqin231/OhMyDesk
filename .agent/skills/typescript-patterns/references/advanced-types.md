## Advanced Type Features

### Type Narrowing

```typescript
// ✅ Good: typeof 守卫
function processValue(value: string | number) {
  if (typeof value === "string") {
    return value.toUpperCase();  // value 是 string
  } else {
    return value.toFixed(2);     // value 是 number
  }
}

// ✅ Good: instanceof 守卫
class Dog {
  bark() { console.log("Woof!"); }
}

class Cat {
  meow() { console.log("Meow!"); }
}

function handleAnimal(animal: Dog | Cat) {
  if (animal instanceof Dog) {
    animal.bark();  // animal 是 Dog
  } else {
    animal.meow();  // animal 是 Cat
  }
}

// ✅ Good: in 操作符守卫
type Fish = { swim: () => void };
type Bird = { fly: () => void };

function move(animal: Fish | Bird) {
  if ("swim" in animal) {
    animal.swim();  // animal 是 Fish
  } else {
    animal.fly();   // animal 是 Bird
  }
}

// ✅ Good: 自定义类型守卫
function isString(value: unknown): value is string {
  return typeof value === "string";
}

function processUnknown(value: unknown) {
  if (isString(value)) {
    console.log(value.toUpperCase()); // 类型安全！
  }
}

// ✅ Good: 非空断言守卫
function process(value: string | null) {
  if (value !== null) {
    console.log(value.length);  // value 是 string
  }
}
```

### Conditional Types

```typescript
// ✅ Good: 条件类型基础
type IsString<T> = T extends string ? true : false;

type A = IsString<string>;  // true
type B = IsString<number>;  // false

// ✅ Good: 提取函数返回类型
type ReturnType<T> = T extends (...args: any[]) => infer R ? R : never;

function getUser() {
  return { id: 1, name: "Alice" };
}

type User = ReturnType<typeof getUser>;  // { id: number; name: string; }

// ✅ Good: 提取 Promise 类型
type Awaited<T> = T extends Promise<infer U> ? U : T;

type Result = Awaited<Promise<string>>;  // string

// ✅ Good: 分布式条件类型
type ToArray<T> = T extends any ? T[] : never;

type Arrays = ToArray<string | number>;  // string[] | number[]
```

### Template Literal Types

```typescript
// ✅ Good: 模板字面量类型
type HTTPMethod = "GET" | "POST" | "PUT" | "DELETE";
type Endpoint = "/users" | "/posts" | "/comments";
type Route = `${HTTPMethod} ${Endpoint}`;

const route: Route = "GET /users";  // ✅
// const invalid: Route = "GET /invalid";  // ❌

// ✅ Good: 字符串操作类型
type Uppercase<S extends string> = Intrinsic; // 内置

type Greeting = "hello";
type LoudGreeting = Uppercase<Greeting>;  // "HELLO"

// ✅ Good: 从对象生成路由
type User = {
  id: number;
  name: string;
  email: string;
};

type UserRoutes = {
  [K in keyof User as `/user/${string & K}`]: User[K];
};

// 结果：
// {
//   "/user/id": number;
//   "/user/name": string;
//   "/user/email": string;
// }
```

### Mapped Types

```typescript
// ✅ Good: Partial（所有属性可选）
type Partial<T> = {
  [P in keyof T]?: T[P];
};

// ✅ Good: Required（所有属性必需）
type Required<T> = {
  [P in keyof T]-?: T[P];
};

// ✅ Good: Readonly（所有属性只读）
type Readonly<T> = {
  readonly [P in keyof T]: T[P];
};

// ✅ Good: Pick（选取部分属性）
type Pick<T, K extends keyof T> = {
  [P in K]: T[P];
};

type User = {
  id: number;
  name: string;
  email: string;
  password: string;
};

type PublicUser = Pick<User, "id" | "name" | "email">;

// ✅ Good: Omit（排除部分属性）
type Omit<T, K extends keyof T> = Pick<T, Exclude<keyof T, K>>;

type CreateUserInput = Omit<User, "id">;

// ✅ Good: Record（构造对象类型）
type Record<K extends string | number | symbol, T> = {
  [P in K]: T;
};

type UserRoles = Record<string, "admin" | "user" | "guest">;

const roles: UserRoles = {
  "alice": "admin",
  "bob": "user"
};
```
