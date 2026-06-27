## 性能优化建议

### 推荐做法

```php
<?php
// ✅ 使用 === 而非 ==
if ($status === 'active') { }

// ✅ 循环外获取数组长度
$count = count($items);
for ($i = 0; $i < $count; $i++) { }

// ✅ 使用生成器处理大数据集
function processLargeFile(string $path): Generator
{
    $handle = fopen($path, 'r');
    while (($line = fgets($handle)) !== false) {
        yield trim($line);
    }
    fclose($handle);
}

// ✅ 使用数组键而非 in_array 进行快速查找
$allowedRoles = ['admin' => true, 'editor' => true];
if (isset($allowedRoles[$role])) { }
```

### 避免的性能陷阱

```php
<?php
// ❌ 循环内调用 count
for ($i = 0; $i < count($items); $i++) { }  // 每次循环都计算

// ❌ 循环内执行 SQL 查询（N+1 问题）
foreach ($users as $user) {
    $posts = $db->query("SELECT * FROM posts WHERE user_id = ?", [$user->id]);
}

// ❌ 不必要的字符串连接
$str = '';
foreach ($items as $item) {
    $str .= $item . ',';  // 使用 implode 替代
}
```

---

## 错误处理规范

```php
<?php
// ✅ 正确：使用异常处理业务错误
class UserNotFoundException extends RuntimeException {}
class ValidationException extends InvalidArgumentException {}

function findUser(int $id): User
{
    $user = $this->repository->find($id);
    if ($user === null) {
        throw new UserNotFoundException("用户 #{$id} 不存在");
    }
    return $user;
}

// ✅ 正确：捕获异常并返回恰当响应
try {
    $user = $this->findUser($id);
} catch (UserNotFoundException $e) {
    return $this->jsonError($e->getMessage(), 404);
} catch (Throwable $e) {
    // 记录日志，返回通用错误（不暴露内部信息）
    error_log($e->getMessage());
    return $this->jsonError('服务器内部错误', 500);
}

// ❌ 禁止：捕获异常后不处理
try {
    risky_operation();
} catch (Exception $e) {
    // 空 catch 块 - 绝对禁止！
}
```

---

## 测试规范

```php
<?php
declare(strict_types=1);

namespace Tests\Unit;

use PHPUnit\Framework\TestCase;

class UserServiceTest extends TestCase
{
    /**
     * @test
     * 测试方法命名应清晰描述测试场景
     */
    public function it_returns_null_when_user_not_found(): void
    {
        // Arrange
        $service = new UserService();

        // Act
        $result = $service->findByEmail('nonexistent@example.com');

        // Assert
        $this->assertNull($result);
    }

    /**
     * @test
     * @dataProvider invalidEmailProvider
     */
    public function it_throws_exception_for_invalid_email(string $email): void
    {
        $this->expectException(InvalidArgumentException::class);

        $service = new UserService();
        $service->findByEmail($email);
    }

    public static function invalidEmailProvider(): array
    {
        return [
            'empty string' => [''],
            'missing @' => ['invalidemail.com'],
            'missing domain' => ['invalid@'],
        ];
    }
}
```
