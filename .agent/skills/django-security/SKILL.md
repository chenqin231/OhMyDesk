---
name: django-security
description: Django security best practices, authentication, authorization, CSRF protection, SQL injection prevention, XSS prevention, and secure deployment configurations.
triggers:
  keywords:
    primary: [django security, django auth, csrf, xss django]
    secondary: [django permission, rate limiting, django deploy, sql injection]
  context_boost: [settings.py, permissions.py, middleware.py, .env]
  context_penalty: [flask, fastapi, express]
  priority: high
tier: optional
stacks: [python]
---

# Django Security Best Practices

Comprehensive security guidelines for Django applications to protect against common vulnerabilities.

## 参考资源

| 主题 | 说明 | 文件 |
|------|------|------|
| 核心安全配置与认证 | 生产环境 settings、自定义 User Model、密码哈希、Session 管理 | [core-settings-auth.md](references/core-settings-auth.md) |
| 授权与 RBAC | Django 权限、DRF 自定义权限、角色访问控制 | [authorization-rbac.md](references/authorization-rbac.md) |
| 注入/XSS/CSRF 防护 | SQL 注入防护、模板转义、安全字符串处理、CSRF Token | [injection-xss-csrf.md](references/injection-xss-csrf.md) |
| 文件上传与 API 安全 | 文件校验、安全存储、速率限制、API 认证配置 | [file-upload-api-security.md](references/file-upload-api-security.md) |
| 安全头/环境变量/日志 | CSP 策略、django-environ 管理密钥、安全事件日志、检查清单 | [headers-env-logging.md](references/headers-env-logging.md) |
