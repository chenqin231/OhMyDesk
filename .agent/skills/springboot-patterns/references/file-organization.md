## File Organization

### Single File Line Limits

| File Type | Recommended | Hard Limit | Notes |
|-----------|------------|------------|-------|
| Controller | 100 | 200 | Thin layer: routing + validation only |
| Service | 150 | 300 | Business logic, split by use case |
| Repository | 80 | 150 | Data access, one per aggregate root |
| DTO / Request / Response | 60 | 120 | Split by endpoint group |
| Config | 80 | 150 | One config class per concern |
| Entity | 100 | 200 | Use `@Embeddable` to split large entities |
| Exception / Handler | 80 | 150 | Group by domain |

### Project Structure (Recommended)

```
src/main/java/com/example/app/
├── config/              # Configuration classes
├── controller/          # REST controllers (thin)
├── service/             # Business logic
│   ├── UserService.java
│   └── OrderService.java
├── repository/          # Data access (Spring Data)
├── model/
│   ├── entity/          # JPA entities
│   ├── dto/             # Data transfer objects
│   └── enums/           # Enumerations
├── exception/           # Custom exceptions + handlers
├── mapper/              # Entity ↔ DTO mappers
├── security/            # Auth, filters, token
└── util/                # Utility classes
```

### Common Splitting Patterns

| Scenario | How to Split | Example |
|----------|-------------|---------|
| Service with 8+ methods | Split by use case | `UserService` → `UserAuthService` + `UserProfileService` |
| Controller with many endpoints | Split by resource | `AdminController` → `AdminUserController` + `AdminOrderController` |
| Large entity with 15+ fields | Use `@Embeddable` | `Order` embeds `ShippingAddress`, `BillingInfo` |
| Global exception handler growing | Split by domain | `UserExceptionHandler` + `OrderExceptionHandler` |
| Config class doing too much | One config per concern | `SecurityConfig` + `CacheConfig` + `AsyncConfig` |
