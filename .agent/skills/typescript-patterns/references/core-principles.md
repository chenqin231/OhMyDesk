## When to Activate

- 编写 TypeScript 代码
- 审查 TypeScript 代码
- 配置 TypeScript 项目
- 迁移 JavaScript 到 TypeScript
- 设计类型安全的 API

## File Organization

### Single File Line Limits

| File Type | Recommended | Hard Limit | Notes |
|-----------|------------|------------|-------|
| Component (.tsx) | 120 | 250 | Split into sub-components when exceeded |
| Hook | 80 | 150 | Extract utility hooks |
| Service / API | 150 | 300 | Split by domain |
| Types / Interfaces | 100 | 200 | Group by domain |
| Utils / Helpers | 80 | 150 | One concern per file |
| Store / State | 120 | 250 | Split by slice |

### Project Structure (Recommended)

```
src/
├── components/          # UI components
│   ├── Button/
│   │   ├── Button.tsx
│   │   ├── Button.test.tsx
│   │   └── index.ts
│   └── Form/
├── hooks/               # Custom hooks
├── services/            # API calls, business logic
├── stores/              # State management
├── types/               # Shared type definitions
├── utils/               # Pure utility functions
├── constants/           # App-wide constants
└── lib/                 # Third-party wrappers
```

### Common Splitting Patterns

| Scenario | How to Split | Example |
|----------|-------------|---------|
| Component > 200 lines | Extract sub-components | `Dashboard.tsx` → `DashboardHeader.tsx` + `DashboardChart.tsx` + `DashboardTable.tsx` |
| Component with complex logic | Extract custom hook | `useFormValidation.ts` extracted from `RegistrationForm.tsx` |
| Types mixed in component | Separate types file | `types.ts` alongside `Component.tsx` |
| Multiple API calls in one file | Split by resource | `userApi.ts` + `orderApi.ts` instead of `api.ts` |
| Barrel exports growing large | Split by domain | `components/index.ts` per feature folder |

## Core Principles

### 1. Strict Mode Always

始终启用严格模式，获得最佳类型安全。

```json
// ✅ Good: tsconfig.json
{
  "compilerOptions": {
    "strict": true,              // 启用所有严格检查
    "noUncheckedIndexedAccess": true,  // 数组/对象索引返回 T | undefined
    "noImplicitOverride": true,        // 覆盖方法需要 override 关键字
    "noUnusedLocals": true,            // 检查未使用的局部变量
    "noUnusedParameters": true,        // 检查未使用的参数
    "noFallthroughCasesInSwitch": true, // 检查 switch fallthrough
    "forceConsistentCasingInFileNames": true,
    "skipLibCheck": true,
    "esModuleInterop": true,
    "moduleResolution": "bundler",
    "module": "ESNext",
    "target": "ES2022"
  }
}

// ❌ Bad: 宽松配置
{
  "compilerOptions": {
    "strict": false,  // 失去类型安全！
    "noImplicitAny": false
  }
}
```

### 2. Avoid `any` - Use `unknown` Instead

`any` 会破坏类型安全，使用 `unknown` 进行类型收窄。

```typescript
// ✅ Good: 使用 unknown 并进行类型收窄
function processData(data: unknown) {
  if (typeof data === 'string') {
    return data.toUpperCase(); // 类型收窄：data 是 string
  }

  if (typeof data === 'number') {
    return data.toFixed(2); // 类型收窄：data 是 number
  }

  throw new Error('Invalid data type');
}

// ✅ Good: 使用类型守卫
function isUser(value: unknown): value is User {
  return (
    typeof value === 'object' &&
    value !== null &&
    'id' in value &&
    'name' in value
  );
}

function handleUser(data: unknown) {
  if (isUser(data)) {
    console.log(data.name); // 类型安全！
  }
}

// ❌ Bad: 使用 any（失去类型安全）
function processData(data: any) {
  return data.toUpperCase(); // 运行时可能崩溃！
}
```

### 3. Prefer Type Inference

让 TypeScript 推断类型，仅在必要时显式标注。

```typescript
// ✅ Good: 类型推断
const name = "Alice";  // 推断为 string
const age = 30;        // 推断为 number
const user = {         // 推断为 { id: number; name: string; }
  id: 1,
  name: "Alice"
};

function getLength(str: string) {
  return str.length;   // 推断返回 number
}

// ✅ Good: 需要显式标注的场景
interface User {
  id: number;
  name: string;
}

// 函数参数必须标注
function createUser(name: string): User {
  return {
    id: Math.random(),
    name
  };
}

// ❌ Bad: 不必要的类型标注
const name: string = "Alice";  // 冗余
const age: number = 30;        // 冗余
```
