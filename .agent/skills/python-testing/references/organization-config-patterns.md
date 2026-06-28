## Test Organization

### Directory Structure

```
tests/
├── conftest.py                 # Shared fixtures
├── __init__.py
├── unit/                       # Unit tests
│   ├── __init__.py
│   ├── test_models.py
│   ├── test_utils.py
│   └── test_services.py
├── integration/                # Integration tests
│   ├── __init__.py
│   ├── test_api.py
│   └── test_database.py
└── e2e/                        # End-to-end tests
    ├── __init__.py
    └── test_user_flow.py
```

### Test Classes

```python
class TestUserService:
    """Group related tests in a class."""

    @pytest.fixture(autouse=True)
    def setup(self):
        """Setup runs before each test in this class."""
        self.service = UserService()

    def test_create_user(self):
        """Test user creation."""
        user = self.service.create_user("Alice")
        assert user.name == "Alice"

    def test_delete_user(self):
        """Test user deletion."""
        user = User(id=1, name="Bob")
        self.service.delete_user(user)
        assert not self.service.user_exists(1)
```

## Best Practices

### DO

- **Follow TDD**: Write tests before code (red-green-refactor)
- **Test one thing**: Each test should verify a single behavior
- **Use descriptive names**: `test_user_login_with_invalid_credentials_fails`
- **Use fixtures**: Eliminate duplication with fixtures
- **Mock external dependencies**: Don't depend on external services
- **Test edge cases**: Empty inputs, None values, boundary conditions
- **Aim for 80%+ coverage**: Focus on critical paths
- **Keep tests fast**: Use marks to separate slow tests

### DON'T

- **Don't test implementation**: Test behavior, not internals
- **Don't use complex conditionals in tests**: Keep tests simple
- **Don't ignore test failures**: All tests must pass
- **Don't test third-party code**: Trust libraries to work
- **Don't share state between tests**: Tests should be independent
- **Don't catch exceptions in tests**: Use `pytest.raises`
- **Don't use print statements**: Use assertions and pytest output
- **Don't write tests that are too brittle**: Avoid over-specific mocks

## Common Patterns

### Testing API Endpoints (FastAPI/Flask)

```python
@pytest.fixture
def client():
    app = create_app(testing=True)
    return app.test_client()

def test_get_user(client):
    response = client.get("/api/users/1")
    assert response.status_code == 200
    assert response.json["id"] == 1

def test_create_user(client):
    response = client.post("/api/users", json={
        "name": "Alice",
        "email": "alice@example.com"
    })
    assert response.status_code == 201
    assert response.json["name"] == "Alice"
```

### Testing Database Operations

```python
@pytest.fixture
def db_session():
    """Create a test database session."""
    session = Session(bind=engine)
    session.begin_nested()
    yield session
    session.rollback()
    session.close()

def test_create_user(db_session):
    user = User(name="Alice", email="alice@example.com")
    db_session.add(user)
    db_session.commit()

    retrieved = db_session.query(User).filter_by(name="Alice").first()
    assert retrieved.email == "alice@example.com"
```

### Testing Class Methods

```python
class TestCalculator:
    @pytest.fixture
    def calculator(self):
        return Calculator()

    def test_add(self, calculator):
        assert calculator.add(2, 3) == 5

    def test_divide_by_zero(self, calculator):
        with pytest.raises(ZeroDivisionError):
            calculator.divide(10, 0)
```

## pytest Configuration

### pytest.ini

```ini
[pytest]
testpaths = tests
python_files = test_*.py
python_classes = Test*
python_functions = test_*
addopts =
    --strict-markers
    --disable-warnings
    --cov=mypackage
    --cov-report=term-missing
    --cov-report=html
markers =
    slow: marks tests as slow
    integration: marks tests as integration tests
    unit: marks tests as unit tests
```

### pyproject.toml

```toml
[tool.pytest.ini_options]
testpaths = ["tests"]
python_files = ["test_*.py"]
python_classes = ["Test*"]
python_functions = ["test_*"]
addopts = [
    "--strict-markers",
    "--cov=mypackage",
    "--cov-report=term-missing",
    "--cov-report=html",
]
markers = [
    "slow: marks tests as slow",
    "integration: marks tests as integration tests",
    "unit: marks tests as unit tests",
]
```

## Running Tests

```bash
# Run all tests
pytest

# Run specific file
pytest tests/test_utils.py

# Run specific test
pytest tests/test_utils.py::test_function

# Run with verbose output
pytest -v

# Run with coverage
pytest --cov=mypackage --cov-report=html

# Run only fast tests
pytest -m "not slow"

# Run until first failure
pytest -x

# Run and stop on N failures
pytest --maxfail=3

# Run last failed tests
pytest --lf

# Run tests with pattern
pytest -k "test_user"

# Run with debugger on failure
pytest --pdb
```

## Quick Reference

| Pattern | Usage |
|---------|-------|
| `pytest.raises()` | Test expected exceptions |
| `@pytest.fixture()` | Create reusable test fixtures |
| `@pytest.mark.parametrize()` | Run tests with multiple inputs |
| `@pytest.mark.slow` | Mark slow tests |
| `pytest -m "not slow"` | Skip slow tests |
| `@patch()` | Mock functions and classes |
| `tmp_path` fixture | Automatic temp directory |
| `pytest --cov` | Generate coverage report |
| `assert` | Simple and readable assertions |

**Remember**: Tests are code too. Keep them clean, readable, and maintainable. Good tests catch bugs; great tests prevent them.
