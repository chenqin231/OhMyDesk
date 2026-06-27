---
name: springboot-patterns
description: Spring Boot architecture patterns, REST API design, layered services, data access, caching, async processing, and logging.
triggers:
  keywords:
    primary: [spring boot, spring, java rest api java]
    secondary: [controller, service layer, jpa, repository pattern]
  context_boost: [spring boot project, java api, spring data]
  context_penalty: [frontend, react, python, golang]
  priority: medium
tier: optional
stacks: [java]
---

# Spring Boot Development Patterns

Spring Boot architecture and API patterns for scalable, production-grade services.

## References

| Topic | Description | File |
|-------|-------------|------|
| File Organization | Line limits, project structure, splitting patterns | [file-organization.md](references/file-organization.md) |
| REST API & Service Layer | Controllers, repositories, transactions, DTOs, exception handling | [rest-api-service.md](references/rest-api-service.md) |
| Caching, Async & Logging | @Cacheable, @Async, SLF4J logging, request filters | [caching-async-logging.md](references/caching-async-logging.md) |
| Resilience & Rate Limiting | Pagination, retry with backoff, Bucket4j rate limiting | [resilience-ratelimit.md](references/resilience-ratelimit.md) |
| Production Defaults | Background jobs, observability, HikariCP, constructor injection | [production-defaults.md](references/production-defaults.md) |
