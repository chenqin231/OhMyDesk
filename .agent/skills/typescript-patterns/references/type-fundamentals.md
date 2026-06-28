## Type System Fundamentals

### Basic Types

```typescript
// ✅ Good: 基本类型使用
let isDone: boolean = false;
let count: number = 42;
let message: string = "Hello";
let nothing: null = null;
let notDefined: undefined = undefined;

// 数组
let numbers: number[] = [1, 2, 3];
let strings: Array<string> = ["a", "b", "c"];

// 元组
let tuple: [string, number] = ["Alice", 30];

// 枚举
enum Status {
  Pending = "PENDING",
  Approved = "APPROVED",
  Rejected = "REJECTED"
}

// 对象类型
type Point = {
  x: number;
  y: number;
};

// 函数类型
type AddFn = (a: number, b: number) => number;

const add: AddFn = (a, b) => a + b;
```

### Union and Intersection Types

```typescript
// ✅ Good: 联合类型（或）
type Status = "idle" | "loading" | "success" | "error";

function setStatus(status: Status) {
  // ...
}

setStatus("loading"); // ✅
// setStatus("invalid"); // ❌ 类型错误

// ✅ Good: 交叉类型（且）
type Timestamped = {
  createdAt: Date;
  updatedAt: Date;
};

type User = {
  id: string;
  name: string;
};

type TimestampedUser = User & Timestamped;

const user: TimestampedUser = {
  id: "1",
  name: "Alice",
  createdAt: new Date(),
  updatedAt: new Date()
};

// ✅ Good: 辨识联合类型（Discriminated Unions）
type SuccessResult = {
  success: true;
  data: string;
};

type ErrorResult = {
  success: false;
  error: string;
};

type Result = SuccessResult | ErrorResult;

function handleResult(result: Result) {
  if (result.success) {
    console.log(result.data); // 类型收窄：SuccessResult
  } else {
    console.log(result.error); // 类型收窄：ErrorResult
  }
}
```

### Generics

```typescript
// ✅ Good: 泛型函数
function identity<T>(value: T): T {
  return value;
}

const num = identity(42);       // T = number
const str = identity("hello");  // T = string

// ✅ Good: 泛型约束
function getProperty<T, K extends keyof T>(obj: T, key: K): T[K] {
  return obj[key];
}

const user = { id: 1, name: "Alice" };
const name = getProperty(user, "name"); // string
// getProperty(user, "invalid"); // ❌ 类型错误

// ✅ Good: 泛型接口
interface Repository<T> {
  findById(id: string): Promise<T | null>;
  findAll(): Promise<T[]>;
  create(data: Omit<T, 'id'>): Promise<T>;
  update(id: string, data: Partial<T>): Promise<T>;
  delete(id: string): Promise<void>;
}

class UserRepository implements Repository<User> {
  async findById(id: string): Promise<User | null> {
    // ...
  }
  // ...
}

// ✅ Good: 泛型默认值
type ApiResponse<T = unknown> = {
  success: boolean;
  data: T;
  error?: string;
};
```
