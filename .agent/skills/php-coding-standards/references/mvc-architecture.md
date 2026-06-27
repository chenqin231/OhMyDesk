## MVC 架构规范

### Controller（控制器）

**职责**：接收请求 → 验证参数 → 调用服务/模型 → 返回响应

```php
<?php
declare(strict_types=1);

namespace App\Controllers;

class UserController
{
    public function __construct(
        private readonly UserService $userService
    ) {}

    public function show(int $id): Response
    {
        // ✅ 1. 验证参数
        if ($id <= 0) {
            return $this->jsonError('无效的用户ID', 400);
        }

        // ✅ 2. 调用服务层
        $user = $this->userService->findById($id);

        if ($user === null) {
            return $this->jsonError('用户不存在', 404);
        }

        // ✅ 3. 返回响应
        return $this->json(['user' => $user->toArray()]);
    }
}
```

**禁止事项**：
- ❌ 控制器中直接写 SQL
- ❌ 控制器中写 HTML 字符串
- ❌ 复杂的业务逻辑

### Model（模型）

**职责**：数据访问（CRUD）→ 数据关系 → 简单业务验证

```php
<?php
declare(strict_types=1);

namespace App\Models;

class User extends Model
{
    protected string $table = 'users';
    protected array $fillable = ['name', 'email', 'password'];
    protected array $hidden = ['password'];

    public function findByEmail(string $email): ?self
    {
        return $this->where('email', $email)->first();
    }

    public function isActive(): bool
    {
        return $this->status === 'active';
    }
}
```

**禁止事项**：
- ❌ 模型中访问 `$_POST` / `$_GET`
- ❌ 模型中处理 HTTP 响应

### View（视图）

**职责**：展示数据 → 简单条件判断和循环

```php
<?php // views/user/profile.php ?>
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <title><?= htmlspecialchars($title, ENT_QUOTES, 'UTF-8') ?></title>
</head>
<body>
    <!-- ✅ 正确：转义输出 -->
    <h1><?= $this->escape($user->name) ?></h1>

    <!-- ✅ 允许：简单条件判断 -->
    <?php if ($user->isAdmin()): ?>
        <span class="badge">管理员</span>
    <?php endif; ?>
</body>
</html>
```

**禁止事项**：
- ❌ 视图中写数据库查询
- ❌ 视图中写复杂业务逻辑
