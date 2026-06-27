# Performance and Memory Optimization

## Using __slots__ for Memory Efficiency

```python
# Bad: Regular class uses __dict__ (more memory)
class Point:
    def __init__(self, x: float, y: float):
        self.x = x
        self.y = y

# Good: __slots__ reduces memory usage
class Point:
    __slots__ = ['x', 'y']

    def __init__(self, x: float, y: float):
        self.x = x
        self.y = y
```

## Generator for Large Data

```python
# Bad: Returns full list in memory
def read_lines(path: str) -> list[str]:
    with open(path) as f:
        return [line.strip() for line in f]

# Good: Yields lines one at a time
def read_lines(path: str) -> Iterator[str]:
    with open(path) as f:
        for line in f:
            yield line.strip()
```

## Use @lru_cache for Expensive Computations

```python
from functools import lru_cache

# ✅ Good: 缓存斐波那契计算
@lru_cache(maxsize=128)
def fibonacci(n: int) -> int:
    if n < 2:
        return n
    return fibonacci(n - 1) + fibonacci(n - 2)

# ❌ Bad: 重复计算（指数时间复杂度）
def fibonacci(n: int) -> int:
    if n < 2:
        return n
    return fibonacci(n - 1) + fibonacci(n - 2)
```

## Avoid String Concatenation in Loops

```python
# Bad: O(n²) due to string immutability
result = ""
for item in items:
    result += str(item)

# Good: O(n) using join
result = "".join(str(item) for item in items)

# Good: Using StringIO for building
from io import StringIO

buffer = StringIO()
for item in items:
    buffer.write(str(item))
result = buffer.getvalue()
```

## List Comprehensions vs Map/Filter

```python
# ✅ Good: 列表推导（简单场景）
numbers = [1, 2, 3, 4, 5]
squares = [x**2 for x in numbers]
evens = [x for x in numbers if x % 2 == 0]

# ✅ Good: 生成器表达式（大数据集）
big_numbers = range(1_000_000)
squares_gen = (x**2 for x in big_numbers)  # 不立即计算

# ❌ Bad: map/filter 通常可读性较差
squares = list(map(lambda x: x**2, numbers))
evens = list(filter(lambda x: x % 2 == 0, numbers))
```

## Use `itertools` for Efficient Iteration

```python
import itertools

# ✅ Good: 使用 itertools
# 无限计数器
counter = itertools.count(start=1, step=1)

# 循环迭代
cycler = itertools.cycle(['A', 'B', 'C'])  # A, B, C, A, B, C, ...

# 累积
累积和 = itertools.accumulate([1, 2, 3, 4])  # 1, 3, 6, 10

# 组合
combinations = itertools.combinations([1, 2, 3], 2)  # (1,2), (1,3), (2,3)

# 笛卡尔积
product = itertools.product([1, 2], ['a', 'b'])  # (1,'a'), (1,'b'), (2,'a'), (2,'b')
```

## Profiling and Optimization

```python
import cProfile
import pstats
from functools import wraps
import time

# ✅ Good: 性能分析装饰器
def profile(func):
    @wraps(func)
    def wrapper(*args, **kwargs):
        pr = cProfile.Profile()
        pr.enable()
        result = func(*args, **kwargs)
        pr.disable()

        stats = pstats.Stats(pr)
        stats.sort_stats('cumulative')
        stats.print_stats(10)  # 打印前 10 个最慢的函数

        return result
    return wrapper

# ✅ Good: 时间测量装饰器
def timeit(func):
    @wraps(func)
    def wrapper(*args, **kwargs):
        start = time.perf_counter()
        result = func(*args, **kwargs)
        end = time.perf_counter()
        print(f"{func.__name__} took {end - start:.4f} seconds")
        return result
    return wrapper

@timeit
def slow_function():
    # ...
    pass
```
