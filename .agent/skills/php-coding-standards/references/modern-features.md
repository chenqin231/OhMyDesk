## PHP 8.x 现代特性

### 推荐使用的特性

```php
<?php
// ✅ 构造器属性提升（PHP 8.0+）
class User
{
    public function __construct(
        private readonly string $name,
        private readonly ?string $email = null,
    ) {}
}

// ✅ 命名参数
$response = sendEmail(
    to: 'user@example.com',
    subject: '欢迎',
    priority: 1
);

// ✅ Match 表达式（替代 switch）
$statusText = match($status) {
    'pending' => '待处理',
    'approved' => '已通过',
    'rejected' => '已拒绝',
    default => '未知状态',
};

// ✅ Null 安全运算符
$userName = $user?->profile?->name ?? '匿名';

// ✅ 联合类型
function processInput(string|array $input): void { }

// ✅ 枚举（用于状态等）
enum Status: string
{
    case Pending = 'pending';
    case Approved = 'approved';
    case Rejected = 'rejected';
}
```

### 禁止使用的过时语法

```php
<?php
// ❌ 禁止：短标签
<? echo $name; ?>  // 使用 <?php 或 <?=

// ❌ 禁止：全局变量
global $db;  // 使用依赖注入替代

// ❌ 禁止：@ 错误抑制符
$data = @file_get_contents($file);  // 使用 try-catch 或条件检查

// ❌ 禁止：动态属性（PHP 8.2+ 已废弃）
$obj->undeclaredProperty = 'value';  // 必须在类中声明属性
```
