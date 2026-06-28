## Background Jobs

Use Spring's `@Scheduled` or integrate with queues (e.g., Kafka, SQS, RabbitMQ). Keep handlers idempotent and observable.

## Observability

- Structured logging (JSON) via Logback encoder
- Metrics: Micrometer + Prometheus/OTel
- Tracing: Micrometer Tracing with OpenTelemetry or Brave backend

## Production Defaults

- Prefer constructor injection, avoid field injection
- Enable `spring.mvc.problemdetails.enabled=true` for RFC 7807 errors (Spring Boot 3+)
- Configure HikariCP pool sizes for workload, set timeouts
- Use `@Transactional(readOnly = true)` for queries
- Enforce null-safety via `@NonNull` and `Optional` where appropriate

**Remember**: Keep controllers thin, services focused, repositories simple, and errors handled centrally. Optimize for maintainability and testability.
