# Modern Python Features

## Pydantic for Data Validation

```python
from pydantic import BaseModel, Field, validator, EmailStr
from typing import Optional
from datetime import datetime

# ✅ Good: Pydantic 模型
class User(BaseModel):
    id: Optional[str] = None
    name: str = Field(..., min_length=1, max_length=100)
    email: EmailStr  # 自动验证邮箱格式
    age: int = Field(..., ge=0, le=150)
    created_at: datetime = Field(default_factory=datetime.now)

    @validator('name')
    def name_must_not_be_empty(cls, v):
        if not v.strip():
            raise ValueError('Name cannot be empty')
        return v.strip()

    @validator('age')
    def age_must_be_reasonable(cls, v):
        if v < 0 or v > 150:
            raise ValueError('Age must be between 0 and 150')
        return v

    class Config:
        # 允许从字典创建
        from_attributes = True
        # 验证赋值
        validate_assignment = True

# 使用
try:
    user = User(name="Alice", email="alice@example.com", age=30)
    print(user.dict())
except ValidationError as e:
    print(e.errors())
```

## Structural Pattern Matching (Python 3.10+)

```python
# ✅ Good: match-case 语句
def process_response(response: dict) -> str:
    match response:
        case {"status": 200, "data": data}:
            return f"Success: {data}"

        case {"status": 404}:
            return "Not found"

        case {"status": code, "error": message} if 400 <= code < 500:
            return f"Client error {code}: {message}"

        case {"status": code, "error": message} if 500 <= code < 600:
            return f"Server error {code}: {message}"

        case _:
            return "Unknown response"

# ✅ Good: 模式匹配类
from dataclasses import dataclass

@dataclass
class Point:
    x: int
    y: int

@dataclass
class Circle:
    center: Point
    radius: int

def describe_shape(shape):
    match shape:
        case Point(x=0, y=0):
            return "Origin point"

        case Point(x=0, y=y):
            return f"Point on Y axis at {y}"

        case Point(x=x, y=0):
            return f"Point on X axis at {x}"

        case Point(x=x, y=y):
            return f"Point at ({x}, {y})"

        case Circle(center=Point(x=0, y=0), radius=r):
            return f"Circle at origin with radius {r}"

        case Circle(center=c, radius=r):
            return f"Circle at {c} with radius {r}"

        case _:
            return "Unknown shape"
```
