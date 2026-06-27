## Integration Patterns

### TypeScript with React

```tsx
// ✅ Good: 函数组件类型
import { FC, ReactNode } from 'react';

type ButtonProps = {
  children: ReactNode;
  onClick: () => void;
  variant?: "primary" | "secondary";
  disabled?: boolean;
};

const Button: FC<ButtonProps> = ({
  children,
  onClick,
  variant = "primary",
  disabled = false
}) => {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`btn btn-${variant}`}
    >
      {children}
    </button>
  );
};

// ✅ Good: Hooks 类型
import { useState, useEffect } from 'react';

function useUser(id: string) {
  const [user, setUser] = useState<User | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    fetchUser(id)
      .then(setUser)
      .catch(setError)
      .finally(() => setLoading(false));
  }, [id]);

  return { user, loading, error };
}
```

### TypeScript with Node.js

```typescript
// ✅ Good: Express 类型
import { Request, Response, NextFunction } from 'express';

type User = {
  id: string;
  name: string;
};

// 扩展 Request 类型
declare global {
  namespace Express {
    interface Request {
      user?: User;
    }
  }
}

// 中间件类型
type AuthMiddleware = (
  req: Request,
  res: Response,
  next: NextFunction
) => void | Promise<void>;

const authMiddleware: AuthMiddleware = async (req, res, next) => {
  const token = req.headers.authorization;

  if (!token) {
    res.status(401).json({ error: "Unauthorized" });
    return;
  }

  try {
    const user = await verifyToken(token);
    req.user = user;
    next();
  } catch (error) {
    res.status(401).json({ error: "Invalid token" });
  }
};

// 路由处理器类型
type RouteHandler<T = any> = (
  req: Request,
  res: Response
) => Promise<void> | void;

const getUser: RouteHandler = async (req, res) => {
  const { id } = req.params;
  const user = await db.user.findUnique({ where: { id } });

  if (!user) {
    res.status(404).json({ error: "User not found" });
    return;
  }

  res.json(user);
};
```

## Testing with TypeScript

```typescript
// ✅ Good: Jest 类型
import { describe, it, expect, jest } from '@jest/globals';

describe('User Service', () => {
  it('creates a user', async () => {
    const mockUser: User = {
      id: "1",
      name: "Alice",
      email: "alice@example.com"
    };

    const createUser = jest.fn<() => Promise<User>>()
      .mockResolvedValue(mockUser);

    const result = await createUser();

    expect(result).toEqual(mockUser);
  });
});

// ✅ Good: 类型安全的 Mock
type MockedFunction<T extends (...args: any[]) => any> = jest.MockedFunction<T>;

const mockFetch: MockedFunction<typeof fetch> = jest.fn();
```
