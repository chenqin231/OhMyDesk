---
name: frontend-patterns
description: Frontend development patterns for React, Next.js, state management, performance optimization, and UI best practices.
triggers:
  keywords:
    primary: [React, Next.js, frontend, component, hook]
    secondary: [useState, useEffect, useMemo, useCallback, Context, Reducer]
  context_boost: [.tsx, .jsx, React.FC, ReactNode, framer-motion]
  context_penalty: [backend, server, database, SQL, CLI]
  priority: high
tier: optional
stacks: [frontend]
---

# Frontend Development Patterns

Modern frontend patterns for React, Next.js, and performant user interfaces.

## 参考资源

| 主题 | 说明 | 文件 |
|------|------|------|
| 组件模式 | Composition、Compound Components、Render Props | [component-patterns.md](references/component-patterns.md) |
| 自定义 Hooks | useToggle、useQuery、useDebounce | [custom-hooks.md](references/custom-hooks.md) |
| 状态管理 | Context + Reducer 模式 | [state-management.md](references/state-management.md) |
| 性能优化 | Memoization、Code Splitting、虚拟列表 | [performance.md](references/performance.md) |
| 表单/错误/动画/无障碍 | Form Validation、ErrorBoundary、Framer Motion、A11y | [form-error-animation-a11y.md](references/form-error-animation-a11y.md) |
