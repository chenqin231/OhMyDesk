## Error Handling

### Result Type Pattern

```typescript
// ✅ Good: Result 类型（类似 Rust）
type Result<T, E = Error> =
  | { ok: true; value: T }
  | { ok: false; error: E };

function divide(a: number, b: number): Result<number> {
  if (b === 0) {
    return {
      ok: false,
      error: new Error("Division by zero")
    };
  }

  return {
    ok: true,
    value: a / b
  };
}

// 使用
const result = divide(10, 2);

if (result.ok) {
  console.log(result.value);  // 类型安全！
} else {
  console.error(result.error);
}

// ✅ Good: 辅助函数
function unwrap<T, E>(result: Result<T, E>): T {
  if (!result.ok) {
    throw result.error;
  }
  return result.value;
}

const value = unwrap(divide(10, 2));
```
