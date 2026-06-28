---
name: django-patterns
description: Django architecture patterns, REST API design with DRF, ORM best practices, caching, signals, middleware, and production-grade Django apps.
triggers:
  keywords:
    primary: [django, django rest framework, drf, django orm]
    secondary: [django model, django view, django serializer, django cache]
  context_boost: [models.py, views.py, serializers.py, urls.py, settings.py]
  context_penalty: [flask, fastapi, express, spring]
  priority: high
tier: optional
stacks: [python]
---

# Django Development Patterns

Production-grade Django architecture patterns for scalable, maintainable applications.

## 参考资源

| 主题 | 说明 | 文件 |
|------|------|------|
| 项目结构与配置 | 推荐目录布局、Split Settings 模式（base/dev/prod） | [project-structure.md](references/project-structure.md) |
| Model 设计模式 | Model 最佳实践、自定义 QuerySet、Manager 方法 | [model-design.md](references/model-design.md) |
| DRF 模式 | Serializer、ViewSet、自定义 Action、权限控制 | [drf-patterns.md](references/drf-patterns.md) |
| Service Layer | 业务逻辑分离、事务管理、支付集成模式 | [service-layer.md](references/service-layer.md) |
| 缓存/信号/中间件 | 多级缓存策略、Signal 模式、自定义 Middleware | [caching-signals-middleware.md](references/caching-signals-middleware.md) |
| 性能优化 | N+1 查询防护、数据库索引、批量操作、快速参考表 | [performance.md](references/performance.md) |
