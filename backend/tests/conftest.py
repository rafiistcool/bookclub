import pytest
from fastapi.testclient import TestClient

from app.config import get_settings
from app.routers.books import clear_search_cache
from app.security import reset_rate_limits


@pytest.fixture
def client(tmp_path, monkeypatch):
    monkeypatch.delenv("BOOKCLUB_ENV_FILE", raising=False)
    monkeypatch.setenv("DATABASE_PATH", str(tmp_path / "bookclub.db"))
    monkeypatch.setenv("SECRET_KEY", "test-secret-key-which-is-long-enough")
    monkeypatch.setenv("BOOKCLUB_BOOTSTRAP_INVITE", "TESTINVITE")
    monkeypatch.setenv("DEBUG", "true")
    monkeypatch.setenv("BOOKCLUB_NAME", "Bookclub")
    monkeypatch.setenv("BOOKCLUB_THEME", "#b44a2a")
    monkeypatch.setenv("BOOKCLUB_PUBLIC_URL", "")
    monkeypatch.setenv("BOOKCLUB_TRUSTED_PROXIES", "*")
    get_settings.cache_clear()
    reset_rate_limits()
    clear_search_cache()
    from app.main import create_app

    with TestClient(create_app()) as test_client:
        yield test_client


def register(
    client: TestClient,
    username: str,
    password: str = "password1",
    invite: str = "TESTINVITE",
):
    return client.post(
        "/api/auth/register",
        json={"username": username, "password": password, "invite_code": invite},
    )


def login(client: TestClient, username: str, password: str = "password1"):
    return client.post(
        "/api/auth/login", json={"username": username, "password": password}
    )
