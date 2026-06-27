---
name: php-coding-standards
description: PHP 8.x coding standards, security practices, and MVC patterns for building secure and maintainable PHP applications.
triggers:
  keywords:
    primary: [PHP, Laravel, CodeIgniter, Symfony, PSR-12]
    secondary: [declare strict_types, PDO, Composer, PHPUnit]
  context_boost: [.php, "<?php", namespace App, use PHPUnit]
  context_penalty: [.ts, .py, .go, React, Node.js]
  priority: high
tier: optional
stacks: [php]
---

# PHP 8.x Coding Standards

PHP 8.x 编码规范与最佳实践，适用于 PHP 8.0+ 项目。

## 核心规范速查

| 类型 | 规则 | 示例 |
|------|------|------|
| 类名 | PascalCase | `UserController` |
| 方法名 | camelCase | `getUserById()` |
| 变量名 | camelCase | `$userName` |
| 常量 | SCREAMING_SNAKE_CASE | `MAX_FILE_SIZE` |

## 参考资源

| 主题 | 说明 | 文件 |
|------|------|------|
| MVC 架构规范 | Controller/Model/View 职责划分与禁止事项 | [mvc-architecture.md](references/mvc-architecture.md) |
| 安全编码规范 | 输入验证、SQL 注入、XSS、文件上传、密码、CSRF | [security.md](references/security.md) |
| PHP 8.x 现代特性 | 构造器提升、命名参数、Match、枚举、禁用语法 | [modern-features.md](references/modern-features.md) |
| 性能/错误处理/测试 | 性能优化、异常处理、PHPUnit 测试规范 | [performance-errors-testing.md](references/performance-errors-testing.md) |
| 提交前检查清单 | 必须检查项、安全检查项、性能检查项 | [checklist.md](references/checklist.md) |
