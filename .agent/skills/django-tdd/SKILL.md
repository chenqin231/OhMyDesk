---
name: django-tdd
description: Django testing strategies with pytest-django, TDD methodology, factory_boy, mocking, coverage, and testing Django REST Framework APIs.
triggers:
  keywords:
    primary: [django test, pytest-django, factory_boy, django tdd]
    secondary: [django mock, drf test, django coverage, django api test]
  context_boost: [test_*.py, conftest.py, factories.py, pytest.ini]
  context_penalty: [flask, fastapi, express]
  priority: high
tier: optional
stacks: [python]
---

# Django Testing with TDD

Test-driven development for Django applications using pytest, factory_boy, and Django REST Framework.

## 参考资源

| 主题 | 说明 | 文件 |
|------|------|------|
| TDD 工作流 | 激活条件与 Red-Green-Refactor 循环 | [tdd-workflow.md](references/tdd-workflow.md) |
| 测试环境配置 | pytest 配置、测试 settings、conftest fixtures | [setup.md](references/setup.md) |
| Factory Boy | 工厂定义、SubFactory、PostGeneration、批量创建 | [factory-boy.md](references/factory-boy.md) |
| Model 与 View 测试 | Django Model 测试模式、View 测试模式 | [model-view-testing.md](references/model-view-testing.md) |
| DRF API 测试 | Serializer 测试、ViewSet 测试、过滤与搜索 | [drf-api-testing.md](references/drf-api-testing.md) |
| Mock 与集成测试 | 外部服务 Mock、邮件测试、完整流程集成测试 | [mocking-integration.md](references/mocking-integration.md) |
| 最佳实践与覆盖率 | DO/DON'T 清单、覆盖率目标、快速参考表 | [best-practices-coverage.md](references/best-practices-coverage.md) |
