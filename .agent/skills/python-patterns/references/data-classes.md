# Data Classes and Named Tuples

## Data Classes

```python
from dataclasses import dataclass, field
from datetime import datetime

@dataclass
class User:
    """User entity with automatic __init__, __repr__, and __eq__."""
    id: str
    name: str
    email: str
    created_at: datetime = field(default_factory=datetime.now)
    is_active: bool = True

# Usage
user = User(
    id="123",
    name="Alice",
    email="alice@example.com"
)
```

## Data Classes with Validation

```python
@dataclass
class User:
    email: str
    age: int

    def __post_init__(self):
        # Validate email format
        if "@" not in self.email:
            raise ValueError(f"Invalid email: {self.email}")
        # Validate age range
        if self.age < 0 or self.age > 150:
            raise ValueError(f"Invalid age: {self.age}")
```

## Named Tuples

```python
from typing import NamedTuple

class Point(NamedTuple):
    """Immutable 2D point."""
    x: float
    y: float

    def distance(self, other: 'Point') -> float:
        return ((self.x - other.x) ** 2 + (self.y - other.y) ** 2) ** 0.5

# Usage
p1 = Point(0, 0)
p2 = Point(3, 4)
print(p1.distance(p2))  # 5.0
```

## Modern Dataclasses (Python 3.7+)

```python
from dataclasses import dataclass, field
from typing import List
from datetime import datetime

# ✅ Good: 使用 dataclass
@dataclass
class User:
    id: str
    name: str
    email: str
    created_at: datetime = field(default_factory=datetime.now)
    tags: List[str] = field(default_factory=list)

    def __post_init__(self):
        """在初始化后执行"""
        self.email = self.email.lower()

# 自动生成 __init__, __repr__, __eq__ 等方法
user = User(id="1", name="Alice", email="ALICE@EXAMPLE.COM")
print(user)  # User(id='1', name='Alice', email='alice@example.com', ...)

# ✅ Good: frozen dataclass（不可变）
@dataclass(frozen=True)
class Point:
    x: float
    y: float

point = Point(1.0, 2.0)
# point.x = 3.0  # ❌ 错误：不可修改
```
