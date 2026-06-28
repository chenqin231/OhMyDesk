# Concurrency and Async Patterns

## Threading for I/O-Bound Tasks

```python
import concurrent.futures
import threading

def fetch_url(url: str) -> str:
    """Fetch a URL (I/O-bound operation)."""
    import urllib.request
    with urllib.request.urlopen(url) as response:
        return response.read().decode()

def fetch_all_urls(urls: list[str]) -> dict[str, str]:
    """Fetch multiple URLs concurrently using threads."""
    with concurrent.futures.ThreadPoolExecutor(max_workers=10) as executor:
        future_to_url = {executor.submit(fetch_url, url): url for url in urls}
        results = {}
        for future in concurrent.futures.as_completed(future_to_url):
            url = future_to_url[future]
            try:
                results[url] = future.result()
            except Exception as e:
                results[url] = f"Error: {e}"
    return results
```

## Multiprocessing for CPU-Bound Tasks

```python
def process_data(data: list[int]) -> int:
    """CPU-intensive computation."""
    return sum(x ** 2 for x in data)

def process_all(datasets: list[list[int]]) -> list[int]:
    """Process multiple datasets using multiple processes."""
    with concurrent.futures.ProcessPoolExecutor() as executor:
        results = list(executor.map(process_data, datasets))
    return results
```

## Async/Await for Concurrent I/O

```python
import asyncio

async def fetch_async(url: str) -> str:
    """Fetch a URL asynchronously."""
    import aiohttp
    async with aiohttp.ClientSession() as session:
        async with session.get(url) as response:
            return await response.text()

async def fetch_all(urls: list[str]) -> dict[str, str]:
    """Fetch multiple URLs concurrently."""
    tasks = [fetch_async(url) for url in urls]
    results = await asyncio.gather(*tasks, return_exceptions=True)
    return dict(zip(urls, results))
```

## Basic Async/Await Patterns

```python
import asyncio
from typing import List

# ✅ Good: 异步函数
async def fetch_user(user_id: str) -> User:
    """异步获取用户数据"""
    async with httpx.AsyncClient() as client:
        response = await client.get(f"https://api.example.com/users/{user_id}")
        response.raise_for_status()
        return User(**response.json())

# ✅ Good: 并发执行多个任务
async def fetch_all_users(user_ids: List[str]) -> List[User]:
    """并发获取多个用户"""
    tasks = [fetch_user(user_id) for user_id in user_ids]
    return await asyncio.gather(*tasks)

# ✅ Good: 使用 asyncio.create_task
async def process_with_background_task():
    # 启动后台任务，不等待完成
    task = asyncio.create_task(send_email_async())

    # 继续执行其他工作
    result = await do_main_work()

    # 可选：等待后台任务完成
    await task

    return result
```

## Async Context Managers

```python
from contextlib import asynccontextmanager

# ✅ Good: 异步上下文管理器
class AsyncDatabaseConnection:
    async def __aenter__(self):
        await self.connect()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        await self.close()

    async def connect(self):
        # 建立连接
        pass

    async def close(self):
        # 关闭连接
        pass

# 使用
async def query_database():
    async with AsyncDatabaseConnection() as db:
        result = await db.query("SELECT * FROM users")
        return result

# ✅ Good: 使用 @asynccontextmanager 装饰器
@asynccontextmanager
async def get_db_session():
    session = await create_session()
    try:
        yield session
    finally:
        await session.close()

async def use_session():
    async with get_db_session() as session:
        await session.execute("SELECT * FROM users")
```

## Async Generators

```python
from typing import AsyncGenerator

# ✅ Good: 异步生成器
async def fetch_pages(url: str) -> AsyncGenerator[dict, None]:
    """异步分页获取数据"""
    page = 1
    async with httpx.AsyncClient() as client:
        while True:
            response = await client.get(f"{url}?page={page}")
            data = response.json()

            if not data:
                break

            yield data
            page += 1

# 使用
async def process_all_pages():
    async for page_data in fetch_pages("https://api.example.com/items"):
        process(page_data)
```

## Error Handling in Async Code

```python
# ✅ Good: 异步错误处理
async def safe_fetch_user(user_id: str) -> User | None:
    try:
        return await fetch_user(user_id)
    except httpx.HTTPError as e:
        logger.error(f"Failed to fetch user {user_id}: {e}")
        return None

# ✅ Good: gather 的错误处理
async def fetch_users_safe(user_ids: List[str]) -> List[User | None]:
    # return_exceptions=True 使失败的任务返回异常而非抛出
    results = await asyncio.gather(
        *[fetch_user(uid) for uid in user_ids],
        return_exceptions=True
    )

    # 过滤异常
    users = []
    for result in results:
        if isinstance(result, Exception):
            logger.error(f"Fetch failed: {result}")
            users.append(None)
        else:
            users.append(result)

    return users
```
