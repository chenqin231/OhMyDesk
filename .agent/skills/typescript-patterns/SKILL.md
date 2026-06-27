---
name: typescript-patterns
description: TypeScript 5.x 类型系统、严格模式、高级类型特性和最佳实践
triggers:
  keywords:
    primary: [typescript, ts, tsx, type system]
    secondary: [tsconfig, generic, union type, type guard]
  context_boost: [.ts, .tsx, tsconfig.json]
  context_penalty: [.go, .py, .rs, .java]
  priority: high
tier: optional
stacks: [node]
---

# TypeScript Development Patterns

TypeScript 5.x 类型系统、最佳实践和高级特性指南。

## 参考资源

| 主题 | 说明 | 文件 |
|------|------|------|
| 核心原则 | 严格模式、unknown vs any、类型推断、文件组织 | [core-principles.md](references/core-principles.md) |
| 类型基础 | 基本类型、联合/交叉类型、泛型 | [type-fundamentals.md](references/type-fundamentals.md) |
| 高级类型 | 类型收窄、条件类型、模板字面量、映射类型 | [advanced-types.md](references/advanced-types.md) |
| 工具类型与安全模式 | Utility Types、辨识联合、Builder、Branded Types | [utility-and-safety.md](references/utility-and-safety.md) |
| 错误处理 | Result 类型模式 | [error-handling.md](references/error-handling.md) |
| 集成与测试 | React/Node.js 集成、Jest 类型化测试 | [integration-and-testing.md](references/integration-and-testing.md) |
| 配置与速查 | tsconfig、ESLint、反模式、速查表 | [config-and-reference.md](references/config-and-reference.md) |
