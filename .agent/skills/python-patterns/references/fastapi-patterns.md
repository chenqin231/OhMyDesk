# FastAPI Patterns

## Basic FastAPI Application Structure

```python
# ✅ Good: FastAPI 应用结构
from fastapi import FastAPI, Depends, HTTPException
from pydantic import BaseModel, Field
from typing import List, Optional

app = FastAPI(title="My API", version="1.0.0")

# Pydantic 模型（请求/响应）
class UserCreate(BaseModel):
    name: str = Field(..., min_length=1, max_length=100)
    email: str = Field(..., regex=r'^[\w\.-]+@[\w\.-]+\.\w+$')
    age: Optional[int] = Field(None, ge=0, le=150)

class UserResponse(BaseModel):
    id: str
    name: str
    email: str
    age: Optional[int]

    class Config:
        from_attributes = True  # 允许从 ORM 模型创建

# 路由
@app.post("/users", response_model=UserResponse, status_code=201)
async def create_user(user: UserCreate) -> UserResponse:
    """创建新用户"""
    db_user = await db.users.create(**user.dict())
    return UserResponse.from_orm(db_user)

@app.get("/users/{user_id}", response_model=UserResponse)
async def get_user(user_id: str) -> UserResponse:
    """获取用户信息"""
    user = await db.users.find_one({"id": user_id})

    if not user:
        raise HTTPException(status_code=404, detail="User not found")

    return UserResponse.from_orm(user)

@app.get("/users", response_model=List[UserResponse])
async def list_users(
    skip: int = 0,
    limit: int = 10,
    search: Optional[str] = None
) -> List[UserResponse]:
    """列出用户（分页）"""
    query = {}
    if search:
        query["name"] = {"$regex": search, "$options": "i"}

    users = await db.users.find(query).skip(skip).limit(limit).to_list()
    return [UserResponse.from_orm(u) for u in users]
```

## Dependency Injection

```python
from fastapi import Depends, Header, HTTPException
from typing import Annotated

# ✅ Good: 数据库依赖
async def get_db():
    """数据库会话依赖"""
    async with AsyncSessionLocal() as session:
        yield session

# ✅ Good: 认证依赖
async def get_current_user(
    authorization: Annotated[str, Header()]
) -> User:
    """从 token 获取当前用户"""
    if not authorization.startswith("Bearer "):
        raise HTTPException(status_code=401, detail="Invalid token")

    token = authorization.replace("Bearer ", "")
    user = await verify_token(token)

    if not user:
        raise HTTPException(status_code=401, detail="Invalid credentials")

    return user

# 使用依赖
@app.get("/me", response_model=UserResponse)
async def get_current_user_info(
    current_user: Annotated[User, Depends(get_current_user)]
) -> UserResponse:
    """获取当前用户信息"""
    return UserResponse.from_orm(current_user)

@app.post("/posts")
async def create_post(
    post: PostCreate,
    current_user: Annotated[User, Depends(get_current_user)],
    db: Annotated[AsyncSession, Depends(get_db)]
) -> PostResponse:
    """创建文章（需要认证）"""
    db_post = Post(**post.dict(), author_id=current_user.id)
    db.add(db_post)
    await db.commit()
    await db.refresh(db_post)
    return PostResponse.from_orm(db_post)
```

## Background Tasks

```python
from fastapi import BackgroundTasks

# ✅ Good: 后台任务
async def send_welcome_email(email: str, name: str):
    """发送欢迎邮件（后台任务）"""
    await email_service.send(
        to=email,
        subject="Welcome!",
        body=f"Welcome {name}!"
    )

@app.post("/users")
async def create_user(
    user: UserCreate,
    background_tasks: BackgroundTasks
) -> UserResponse:
    """创建用户并发送欢迎邮件"""
    db_user = await db.users.create(**user.dict())

    # 添加后台任务
    background_tasks.add_task(send_welcome_email, db_user.email, db_user.name)

    return UserResponse.from_orm(db_user)
```

## Exception Handling

```python
from fastapi import Request
from fastapi.responses import JSONResponse

# ✅ Good: 自定义异常处理器
class CustomException(Exception):
    def __init__(self, detail: str):
        self.detail = detail

@app.exception_handler(CustomException)
async def custom_exception_handler(request: Request, exc: CustomException):
    return JSONResponse(
        status_code=400,
        content={"detail": exc.detail}
    )

# ✅ Good: 全局异常处理
@app.exception_handler(Exception)
async def global_exception_handler(request: Request, exc: Exception):
    logger.error(f"Unhandled exception: {exc}", exc_info=True)
    return JSONResponse(
        status_code=500,
        content={"detail": "Internal server error"}
    )
```
