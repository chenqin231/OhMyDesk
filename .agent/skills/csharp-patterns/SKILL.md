---
name: csharp-patterns
description: C# development patterns, file organization, and best practices for building maintainable .NET applications.
tier: optional
stacks: [csharp]
---

# C# Development Patterns

Idiomatic C# patterns and best practices for building robust, maintainable .NET applications.

## When to Activate

- Writing new C# code (.cs files)
- Reviewing C# code
- Refactoring existing C# code
- Designing C# projects/solutions

## File Organization

### Single File Line Limits

| File Type | Recommended | Hard Limit | Notes |
|-----------|------------|------------|-------|
| Model / DTO | 80 | 150 | Pure data classes, split via partial if needed |
| Service | 150 | 300 | Business logic, extract sub-services when exceeded |
| Controller | 100 | 200 | Thin layer: routing + validation only |
| Repository | 120 | 250 | Data access, split by aggregate root |
| P/Invoke / Native | 200 | 400 | Split by DLL (exempt category) |
| Constants / Addresses | 300 | 500 | Split by domain group (exempt category) |
| Extensions | 100 | 200 | Group by target type |

### Project Structure (Recommended)

```
ProjectName/
├── Models/              # Data models, DTOs, value objects
│   ├── Player.cs
│   └── ServerConfig.cs
├── Services/            # Business logic
│   ├── AuthService.cs
│   └── SessionService.cs
├── Controllers/         # API routing (if web)
├── Repositories/        # Data access
├── Native/              # P/Invoke declarations (if desktop)
│   ├── Kernel32.cs
│   └── User32.cs
├── Helpers/             # Utility functions
├── Extensions/          # Extension methods
├── Interfaces/          # Contracts / abstractions
└── Constants/           # Configuration, enums, constants
```

### Common Splitting Patterns

| Scenario | How to Split | Example |
|----------|-------------|---------|
| Model with 20+ properties | Partial class by domain | `Player.cs` + `Player.Combat.cs` + `Player.Inventory.cs` |
| Service with 8+ methods | Sub-services by responsibility | `UserService.cs` → `UserAuthService.cs` + `UserProfileService.cs` |
| Large constants / address table | Group by functional area | `MemoryAddresses.cs` → `Addresses.Player.cs` + `Addresses.Map.cs` + `Addresses.Combat.cs` |
| Class with logic + data | Separate model and service | `BridgeHost.cs` → `BridgeRouter.cs` (routing) + `BridgeHandler.cs` (processing) |
| Extension methods for many types | One file per target type | `StringExtensions.cs` + `CollectionExtensions.cs` |
| P/Invoke mixed in business code | Dedicated Native/ directory | Extract all `[DllImport]` to `Native/<DllName>.cs` |

### Partial Classes (C# Specific)

Use `partial class` to split large classes while maintaining a single type:

```csharp
// Player.cs — core properties
public partial class Player
{
    public string Name { get; set; }
    public int Level { get; set; }
    public int Health { get; set; }
}

// Player.Combat.cs — combat-related
public partial class Player
{
    public int Attack { get; set; }
    public int Defense { get; set; }

    public int CalculateDamage(Player target)
    {
        return Math.Max(1, Attack - target.Defense);
    }
}

// Player.Inventory.cs — inventory-related
public partial class Player
{
    public List<Item> Items { get; set; } = new();

    public bool AddItem(Item item)
    {
        if (Items.Count >= MaxInventorySize) return false;
        Items.Add(item);
        return true;
    }
}
```

## Core Principles

### 1. Immutability by Default

Prefer immutable types, especially for models and DTOs.

```csharp
// Good: Immutable record
public record UserCredentials(string Username, string PasswordHash);

// Good: Init-only properties
public class ServerConfig
{
    public required string Host { get; init; }
    public required int Port { get; init; }
    public bool UseTls { get; init; } = true;
}

// Bad: Mutable with public setters
public class ServerConfig
{
    public string Host { get; set; }
    public int Port { get; set; }
}
```

### 2. Nullable Reference Types

Always enable nullable reference types and handle null explicitly.

```csharp
// In .csproj
// <Nullable>enable</Nullable>

// Good: Explicit nullability
public User? FindUser(string id)
{
    return _users.FirstOrDefault(u => u.Id == id);
}

public string GetDisplayName(User? user)
{
    return user?.Name ?? "Anonymous";
}
```

### 3. Error Handling

Use exceptions for exceptional cases, Result pattern for expected failures.

```csharp
// Good: Result pattern for expected failures
public record Result<T>
{
    public bool Success { get; init; }
    public T? Data { get; init; }
    public string? Error { get; init; }

    public static Result<T> Ok(T data) => new() { Success = true, Data = data };
    public static Result<T> Fail(string error) => new() { Success = false, Error = error };
}

// Usage
public Result<User> ValidateLogin(string username, string password)
{
    var user = _repo.FindByUsername(username);
    if (user is null)
        return Result<User>.Fail("User not found");

    if (!VerifyPassword(password, user.PasswordHash))
        return Result<User>.Fail("Invalid password");

    return Result<User>.Ok(user);
}
```

### 4. Dependency Injection

Constructor injection for required dependencies.

```csharp
public class AuthService
{
    private readonly IUserRepository _userRepo;
    private readonly ITokenService _tokenService;
    private readonly ILogger<AuthService> _logger;

    public AuthService(
        IUserRepository userRepo,
        ITokenService tokenService,
        ILogger<AuthService> logger)
    {
        _userRepo = userRepo;
        _tokenService = tokenService;
        _logger = logger;
    }
}
```

### 5. Async / Await

Use async throughout, never block with `.Result` or `.Wait()`.

```csharp
// Good: Async all the way
public async Task<User> GetUserAsync(string id, CancellationToken ct = default)
{
    var user = await _repo.FindByIdAsync(id, ct);
    if (user is null)
        throw new NotFoundException($"User {id} not found");
    return user;
}

// Bad: Blocking async
public User GetUser(string id)
{
    return _repo.FindByIdAsync(id).Result; // Deadlock risk!
}
```

## Naming Conventions

| Element | Convention | Example |
|---------|-----------|---------|
| Class / Record | PascalCase | `UserService`, `PlayerConfig` |
| Interface | I + PascalCase | `IUserRepository`, `ILogger` |
| Method | PascalCase | `GetUser`, `ValidateInput` |
| Property | PascalCase | `UserName`, `IsActive` |
| Private field | _camelCase | `_userRepo`, `_logger` |
| Local variable | camelCase | `userName`, `isValid` |
| Constant | PascalCase | `MaxRetryCount`, `DefaultTimeout` |
| Async method | Suffix with Async | `GetUserAsync`, `SaveAsync` |

## P/Invoke Best Practices

For desktop applications that call native Windows APIs:

```csharp
// Good: Organized by DLL, use SafeHandle
internal static partial class Kernel32
{
    [LibraryImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    internal static partial bool ReadProcessMemory(
        SafeProcessHandle hProcess,
        nint lpBaseAddress,
        byte[] lpBuffer,
        nuint nSize,
        out nuint lpNumberOfBytesRead);
}

// Good: Wrapper for safe usage
public class ProcessMemory : IDisposable
{
    private readonly SafeProcessHandle _handle;

    public byte[] Read(nint address, int size)
    {
        var buffer = new byte[size];
        if (!Kernel32.ReadProcessMemory(_handle, address, buffer, (nuint)size, out _))
            throw new Win32Exception(Marshal.GetLastWin32Error());
        return buffer;
    }

    public void Dispose() => _handle.Dispose();
}
```

## Security Patterns

### Input Validation

```csharp
// Good: Validate all external input
public Result<LoginResponse> Login(LoginRequest request)
{
    if (string.IsNullOrWhiteSpace(request.Username))
        return Result<LoginResponse>.Fail("Username is required");

    if (request.Username.Length > 50)
        return Result<LoginResponse>.Fail("Username too long");

    // Sanitize before use
    var sanitized = request.Username.Trim().ToLowerInvariant();
    // ...
}
```

### Sensitive Data

```csharp
// Good: Never log sensitive data
_logger.LogInformation("Login attempt for user: {Username}", username);

// Bad: Logging password
_logger.LogInformation("Login: {Username} / {Password}", username, password);

// Good: SecureString for passwords in memory (when applicable)
// Good: Clear byte arrays after use
Array.Clear(passwordBytes, 0, passwordBytes.Length);
```

## Testing Patterns

```csharp
// Arrange-Act-Assert pattern
[Fact]
public async Task GetUser_ValidId_ReturnsUser()
{
    // Arrange
    var repo = new Mock<IUserRepository>();
    repo.Setup(r => r.FindByIdAsync("user-1", default))
        .ReturnsAsync(new User { Id = "user-1", Name = "Test" });
    var service = new UserService(repo.Object);

    // Act
    var result = await service.GetUserAsync("user-1");

    // Assert
    Assert.Equal("Test", result.Name);
}

[Theory]
[InlineData(null)]
[InlineData("")]
[InlineData("   ")]
public async Task GetUser_InvalidId_ThrowsArgumentException(string? id)
{
    var service = new UserService(Mock.Of<IUserRepository>());
    await Assert.ThrowsAsync<ArgumentException>(
        () => service.GetUserAsync(id!));
}
```
