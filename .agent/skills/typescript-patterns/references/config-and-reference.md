## Configuration Best Practices

### Monorepo TypeScript Configuration

```json
// ✅ Good: 根 tsconfig.json（共享配置）
// tsconfig.base.json
{
  "compilerOptions": {
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "moduleResolution": "bundler",
    "module": "ESNext",
    "target": "ES2022",
    "lib": ["ES2022"],
    "jsx": "react-jsx",
    "incremental": true,
    "composite": true,
    "declaration": true,
    "declarationMap": true,
    "sourceMap": true
  }
}

// packages/app/tsconfig.json（继承基础配置）
{
  "extends": "../../tsconfig.base.json",
  "compilerOptions": {
    "outDir": "./dist",
    "rootDir": "./src"
  },
  "include": ["src"],
  "references": [
    { "path": "../shared" }
  ]
}
```

### Path Aliases

```json
// ✅ Good: tsconfig.json
{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@/*": ["src/*"],
      "@/components/*": ["src/components/*"],
      "@/utils/*": ["src/utils/*"]
    }
  }
}
```

```typescript
// 使用
import { Button } from '@/components/Button';
import { formatDate } from '@/utils/date';
```

## Anti-Patterns to Avoid

```typescript
// ❌ Bad: 使用 any
function processData(data: any) {
  return data.someMethod();  // 运行时可能崩溃
}

// ❌ Bad: 类型断言（as）滥用
const user = {} as User;  // 危险！运行时不是 User
user.name.toUpperCase();  // 崩溃！

// ❌ Bad: 非空断言（!）滥用
function getUser(id: string) {
  return users.find(u => u.id === id)!;  // 如果找不到会崩溃
}

// ❌ Bad: 忽略类型错误
// @ts-ignore
const result = dangerousOperation();

// ❌ Bad: 过度嵌套的泛型
type ComplexType<A, B, C, D, E> = {
  // 太复杂，难以理解和维护
};

// ✅ Good: 替代方案
type SimpleType = {
  a: TypeA;
  b: TypeB;
  // 分解为更简单的类型
};
```

## Quick Reference

| 特性 | 使用场景 |
|------|---------|
| **unknown** | 不确定类型时，需要类型收窄 |
| **never** | 不可能的值，穷尽检查 |
| **Union Types** | 多个可能类型之一 |
| **Intersection Types** | 组合多个类型 |
| **Generics** | 类型参数化，提高复用性 |
| **Conditional Types** | 类型级别的条件逻辑 |
| **Mapped Types** | 转换对象类型 |
| **Template Literal Types** | 字符串类型操作 |
| **Type Guards** | 类型收窄和验证 |
| **Utility Types** | 内置类型工具 |

## ESLint Configuration

```json
// ✅ Good: .eslintrc.json
{
  "parser": "@typescript-eslint/parser",
  "plugins": ["@typescript-eslint"],
  "extends": [
    "eslint:recommended",
    "plugin:@typescript-eslint/recommended",
    "plugin:@typescript-eslint/recommended-requiring-type-checking"
  ],
  "parserOptions": {
    "project": "./tsconfig.json"
  },
  "rules": {
    "@typescript-eslint/no-explicit-any": "error",
    "@typescript-eslint/no-unused-vars": "error",
    "@typescript-eslint/explicit-function-return-type": "warn",
    "@typescript-eslint/no-floating-promises": "error",
    "@typescript-eslint/await-thenable": "error"
  }
}
```

**记住**: TypeScript 的威力在于其类型系统。充分利用类型推断、类型收窄和高级类型特性，构建类型安全、可维护的 JavaScript 应用。避免使用 `any`，始终启用严格模式，让编译器成为你的助手。
