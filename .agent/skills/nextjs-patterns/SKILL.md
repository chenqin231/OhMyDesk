---
name: nextjs-patterns
description: Next.js 15+ 架构模式、App Router、Server Components、性能优化和最佳实践
triggers:
  keywords:
    primary: [nextjs, next.js, app router, server component]
    secondary: [server action, route handler, ISR, revalidate]
  context_boost: [next.config, app/page.tsx, app/layout.tsx]
  context_penalty: [.go, .py, .rs, angular]
  priority: high
tier: optional
stacks: [node,frontend]
---

# Next.js Development Patterns

Next.js 15+ 架构模式、最佳实践和性能优化指南。

## 参考资源

| 主题 | 说明 | 文件 |
|------|------|------|
| 核心原则 | App Router、Server Components、文件组织 | [core-principles.md](references/core-principles.md) |
| 数据获取 | 服务端 fetch、缓存策略、Streaming | [data-fetching.md](references/data-fetching.md) |
| Server Actions | 类型安全的表单提交、权限验证 | [server-actions.md](references/server-actions.md) |
| Route Handlers | RESTful API、Middleware | [route-handlers.md](references/route-handlers.md) |
| 性能优化 | 图片/字体优化、动态导入、静态/动态渲染 | [performance.md](references/performance.md) |
| Metadata 与 SEO | 静态/动态元数据、Open Graph | [metadata-seo.md](references/metadata-seo.md) |
| 错误处理 | Error Boundaries、Not Found 页面 | [error-handling.md](references/error-handling.md) |
| 测试与速查 | 组件测试、E2E、反模式、配置 | [testing-and-reference.md](references/testing-and-reference.md) |
