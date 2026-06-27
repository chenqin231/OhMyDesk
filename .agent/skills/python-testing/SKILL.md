---
name: python-testing
description: Python 测试模式：pytest / TDD / fixtures / mocking / parametrize / 覆盖率。
triggers:
  keywords:
    primary: [pytest, python test, tdd python, python coverage]
    secondary: [fixture, mock, parametrize, conftest, pytest-asyncio]
  context_boost: [test, testing, coverage, assert]
  context_penalty: [frontend, javascript, go test]
  priority: high
tier: optional
stacks: [python]
---

# Python Testing Patterns

pytest 测试全流程指南，含 TDD、fixtures、mocking、参数化、异步测试。

## 参考资源

| 主题 | 说明 | 文件 |
|------|------|------|
| TDD 与基础 | TDD 流程 + 覆盖率要求 + pytest 基本结构 + 断言 | [tdd-and-fundamentals.md](references/tdd-and-fundamentals.md) |
| Fixtures | 基础/作用域/参数化/autouse/conftest fixture 模式 | [fixtures.md](references/fixtures.md) |
| 参数化与标记 | parametrize / markers / 测试选择 / pytest.ini 配置 | [parametrize-markers.md](references/parametrize-markers.md) |
| Mocking | patch / side_effect / autospec / Mock 属性 / 上下文管理器 | [mocking.md](references/mocking.md) |
| 异步与异常 | pytest-asyncio / 异常测试 / 文件操作 / tmp_path | [async-exceptions-side-effects.md](references/async-exceptions-side-effects.md) |
| 组织与配置 | 目录结构 / 最佳实践 / API/DB 测试模式 / 运行命令 | [organization-config-patterns.md](references/organization-config-patterns.md) |
