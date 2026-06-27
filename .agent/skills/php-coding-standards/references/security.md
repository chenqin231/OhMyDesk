## 安全编码规范（强制）

### 1. 输入验证

```php
<?php
// ✅ 正确：使用 filter_input 和严格验证
$id = filter_input(INPUT_GET, 'id', FILTER_VALIDATE_INT);
if ($id === false || $id === null) {
    throw new InvalidArgumentException('无效的ID参数');
}

$email = filter_input(INPUT_POST, 'email', FILTER_VALIDATE_EMAIL);
if ($email === false) {
    throw new InvalidArgumentException('无效的邮箱格式');
}

// ✅ 正确：白名单验证
$allowedActions = ['view', 'edit', 'delete'];
$action = $_GET['action'] ?? '';
if (!in_array($action, $allowedActions, true)) {
    throw new InvalidArgumentException('无效的操作');
}

// ❌ 错误：直接使用未验证的输入
$id = $_GET['id'];  // 危险！
```

### 2. SQL 注入防护

```php
<?php
// ✅ 正确：使用预处理语句
$stmt = $pdo->prepare("SELECT * FROM users WHERE id = ?");
$stmt->execute([$id]);

$stmt = $pdo->prepare("SELECT * FROM users WHERE email = :email");
$stmt->execute(['email' => $email]);

// ❌ 严禁：字符串拼接 SQL
$sql = "SELECT * FROM users WHERE id = " . $id;  // SQL 注入风险！
$sql = "SELECT * FROM users WHERE name = '$name'";  // SQL 注入风险！
```

### 3. XSS 防护

```php
<?php
// ✅ 正确：输出时转义
echo htmlspecialchars($userInput, ENT_QUOTES, 'UTF-8');

// ✅ 正确：JSON 输出
echo json_encode($data, JSON_HEX_TAG | JSON_HEX_APOS | JSON_HEX_QUOT | JSON_UNESCAPED_UNICODE);

// ❌ 严禁：直接输出用户数据
echo $userInput;  // XSS 漏洞！
echo "<div>$_GET['name']</div>";  // 极度危险！
```

### 4. 文件上传安全

```php
<?php
function handleFileUpload(array $file): string
{
    // 1. 验证上传状态
    if ($file['error'] !== UPLOAD_ERR_OK) {
        throw new RuntimeException('文件上传失败');
    }

    // 2. 验证文件大小
    $maxSize = 10 * 1024 * 1024;  // 10MB
    if ($file['size'] > $maxSize) {
        throw new RuntimeException('文件过大');
    }

    // 3. ✅ 验证 MIME 类型（使用 finfo，不信任 $_FILES['type']）
    $finfo = finfo_open(FILEINFO_MIME_TYPE);
    $mimeType = finfo_file($finfo, $file['tmp_name']);
    finfo_close($finfo);

    $allowedMimes = ['image/jpeg', 'image/png', 'image/gif'];
    if (!in_array($mimeType, $allowedMimes, true)) {
        throw new RuntimeException('不支持的文件类型');
    }

    // 4. ✅ 验证文件扩展名
    $allowedExtensions = ['jpg', 'jpeg', 'png', 'gif'];
    $extension = strtolower(pathinfo($file['name'], PATHINFO_EXTENSION));
    if (!in_array($extension, $allowedExtensions, true)) {
        throw new RuntimeException('不支持的文件扩展名');
    }

    // 5. ✅ 生成安全的文件名（不使用用户提供的文件名）
    $newFileName = bin2hex(random_bytes(16)) . '.' . $extension;

    // 6. ✅ 保存到安全目录
    $uploadDir = '/data/uploads/';
    $targetPath = $uploadDir . $newFileName;

    if (!move_uploaded_file($file['tmp_name'], $targetPath)) {
        throw new RuntimeException('文件保存失败');
    }

    return $newFileName;
}

// ❌ 严禁：不安全的做法
$targetPath = 'uploads/' . $_FILES['file']['name'];  // 使用用户文件名！
move_uploaded_file($_FILES['file']['tmp_name'], $targetPath);  // 无任何验证！
```

### 5. 密码处理

```php
<?php
// ✅ 正确：使用 password_hash 和 password_verify
$hash = password_hash($password, PASSWORD_DEFAULT);

if (password_verify($inputPassword, $storedHash)) {
    // 密码正确
}

// ❌ 严禁：使用 MD5/SHA1 等不安全算法
$hash = md5($password);  // 不安全！
$hash = sha1($password);  // 不安全！
```

### 6. CSRF 防护

```php
<?php
// ✅ 生成 CSRF Token
function generateCsrfToken(): string
{
    if (!isset($_SESSION['csrf_token'])) {
        $_SESSION['csrf_token'] = bin2hex(random_bytes(32));
    }
    return $_SESSION['csrf_token'];
}

// ✅ 验证 CSRF Token
function verifyCsrfToken(string $token): bool
{
    return isset($_SESSION['csrf_token'])
        && hash_equals($_SESSION['csrf_token'], $token);
}

// ✅ 表单中加入 Token
?>
<form method="POST">
    <input type="hidden" name="csrf_token" value="<?= generateCsrfToken() ?>">
    <!-- 其他表单字段 -->
</form>
```
