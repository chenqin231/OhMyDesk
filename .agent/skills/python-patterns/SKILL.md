---
name: python-patterns
description: Pythonic idioms, PEP 8 standards, type hints, and best practices for building robust, efficient, and maintainable Python applications.
triggers:
  keywords:
    primary: [Python, python, PEP8, pytest, FastAPI]
    secondary: [pip, pyproject, pydantic, asyncio, dataclass]
  context_boost: [.py, python, django, flask, fastapi]
  context_penalty: [.go, .ts, .java, .cs]
  priority: high
tier: optional
stacks: [python]
---

# Python Development Patterns

Idiomatic Python patterns and best practices for robust, efficient, and maintainable applications.

## 参考资源

| 主题 | 说明 | 文件 |
|------|------|------|
| 核心原则 | 可读性、显式优于隐式、EAFP、反模式速查 | [core-principles.md](references/core-principles.md) |
| 类型提示 | 基础注解、现代语法、TypeVar、Protocol | [type-hints.md](references/type-hints.md) |
| 错误处理 | 特定异常、异常链、自定义异常层级 | [error-handling.md](references/error-handling.md) |
| 上下文管理器 | 资源管理、自定义 context manager | [context-managers.md](references/context-managers.md) |
| 推导式与生成器 | 列表推导、生成器表达式、生成器函数 | [comprehensions-generators.md](references/comprehensions-generators.md) |
| 数据类 | dataclass、NamedTuple、验证、frozen | [data-classes.md](references/data-classes.md) |
| 装饰器 | 函数装饰器、参数化装饰器、类装饰器 | [decorators.md](references/decorators.md) |
| 并发与异步 | threading、multiprocessing、async/await | [concurrency-async.md](references/concurrency-async.md) |
| 包组织与工具 | 项目布局、import 规范、pyproject.toml | [package-tooling.md](references/package-tooling.md) |
| 性能优化 | __slots__、lru_cache、itertools、profiling | [performance.md](references/performance.md) |
| FastAPI 模式 | 路由、依赖注入、后台任务、异常处理 | [fastapi-patterns.md](references/fastapi-patterns.md) |
| 现代特性 | Pydantic、Pattern Matching、类型联合 | [modern-features.md](references/modern-features.md) |
