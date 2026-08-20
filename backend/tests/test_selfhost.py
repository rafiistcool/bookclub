import pytest
from fastapi.testclient import TestClient
from pydantic import ValidationError

from app.branding import cors_origins, open_library_ua, public_config
from app.config import Settings, get_settings
from app.runtime import (
    INVITE_FILENAME,
    SECRET_FILENAME,
    https_mode,
    is_placeholder_invite,
    is_placeholder_secret,
    prepare_environment,
)
from app.security import reset_rate_limits
from tests.conftest import register


def test_placeholder_detection():
    assert is_placeholder_secret(None)
    assert is_placeholder_secret("dev-secret-change-me")
    assert is_placeholder_secret("change-me-to-a-long-random-string")
    assert is_placeholder_secret("short")
    assert not is_placeholder_secret("a" * 32)
    assert is_placeholder_invite(None)
    assert is_placeholder_invite("DEV-ONLY")
    assert is_placeholder_invite("dev only")
    assert not is_placeholder_invite("FRIENDS")
    assert https_mode("auto") == "auto"
    assert https_mode("1") == "always"
    assert https_mode("0") == "never"
    assert https_mode(True) == "always"


def test_production_generates_and_reuses_secrets(tmp_path, monkeypatch):
    monkeypatch.setenv("DEBUG", "0")
    monkeypatch.setenv("SECRET_KEY", "")
    monkeypatch.setenv("BOOKCLUB_BOOTSTRAP_INVITE", "")
    monkeypatch.setenv("DATABASE_PATH", str(tmp_path / "bookclub.db"))

    first = prepare_environment()
    assert first["generated_secret"] is True
    assert first["generated_invite"] is True
    secret = (tmp_path / SECRET_FILENAME).read_text(encoding="utf-8").strip()
    invite = (tmp_path / INVITE_FILENAME).read_text(encoding="utf-8").strip()
    assert len(secret) >= 32
    assert invite
    assert invite != "DEVONLY"

    monkeypatch.setenv("SECRET_KEY", "")
    monkeypatch.setenv("BOOKCLUB_BOOTSTRAP_INVITE", "")
    second = prepare_environment()
    assert second["generated_secret"] is False
    assert second["generated_invite"] is False
    assert (tmp_path / SECRET_FILENAME).read_text(encoding="utf-8").strip() == secret
    assert (tmp_path / INVITE_FILENAME).read_text(encoding="utf-8").strip() == invite


def _production_client(tmp_path, monkeypatch, **env) -> TestClient:
    base_url = env.pop("base_url", "http://test")
    monkeypatch.setenv("DATABASE_PATH", str(tmp_path / "bookclub.db"))
    monkeypatch.setenv("DEBUG", env.pop("DEBUG", "0"))
    monkeypatch.setenv("SECRET_KEY", env.pop("SECRET_KEY", "p" * 32))
    monkeypatch.setenv(
        "BOOKCLUB_BOOTSTRAP_INVITE",
        env.pop("BOOKCLUB_BOOTSTRAP_INVITE", "SELFHOST1"),
    )
    monkeypatch.setenv("BOOKCLUB_HTTPS", env.pop("BOOKCLUB_HTTPS", "auto"))
    for key, value in env.items():
        monkeypatch.setenv(key, value)
    get_settings.cache_clear()
    reset_rate_limits()
    from app.main import create_app

    return TestClient(create_app(), base_url=base_url)


def test_production_app_registers_and_hides_docs(tmp_path, monkeypatch):
    with _production_client(tmp_path, monkeypatch) as client:
        assert client.get("/api/health").json() == {"ok": True}
        assert client.get("/api/docs").status_code == 404
        vite = client.options(
            "/api/health",
            headers={
                "Origin": "http://localhost:5173",
                "Access-Control-Request-Method": "GET",
            },
        )
        assert vite.headers.get("access-control-allow-origin") is None
        response = register(client, "ada", invite="SELFHOST1")
        assert response.status_code == 201
        assert client.get("/api/auth/me").json()["username"] == "ada"


def test_session_cookie_not_secure_on_plain_http(tmp_path, monkeypatch):
    with _production_client(tmp_path, monkeypatch, base_url="http://bookclub.test") as client:
        response = register(client, "ada", invite="SELFHOST1")
        cookie = response.headers.get("set-cookie", "")
        assert "bookclub_session=" in cookie
        assert "secure" not in cookie.lower()


def test_session_cookie_secure_on_https_and_forwarded_proto(tmp_path, monkeypatch):
    https_dir = tmp_path / "https"
    https_dir.mkdir()
    with _production_client(https_dir, monkeypatch, base_url="https://bookclub.test") as client:
        response = register(client, "ada", invite="SELFHOST1")
        cookie = response.headers.get("set-cookie", "")
        assert "bookclub_session=" in cookie
        assert "secure" in cookie.lower()

    proxy_dir = tmp_path / "proxy"
    proxy_dir.mkdir()
    with _production_client(proxy_dir, monkeypatch) as client:
        response = client.post(
            "/api/auth/register",
            json={
                "username": "ada",
                "password": "password1",
                "invite_code": "SELFHOST1",
            },
            headers={"X-Forwarded-Proto": "https"},
        )
        cookie = response.headers.get("set-cookie", "")
        assert response.status_code == 201
        assert "bookclub_session=" in cookie
        assert "secure" in cookie.lower()


def test_backup_copies_sqlite(tmp_path, monkeypatch):
    monkeypatch.setenv("DEBUG", "1")
    monkeypatch.setenv("SECRET_KEY", "test-secret-key-which-is-long-enough")
    monkeypatch.setenv("BOOKCLUB_BOOTSTRAP_INVITE", "TESTINVITE")
    monkeypatch.setenv("DATABASE_PATH", str(tmp_path / "bookclub.db"))
    get_settings.cache_clear()
    reset_rate_limits()
    from app.backup import backup_database
    from app.main import create_app

    with TestClient(create_app()) as client:
        assert register(client, "ada").status_code == 201

    dest = tmp_path / "copy.db"
    backup_database(tmp_path / "bookclub.db", dest)
    assert dest.is_file()
    assert dest.stat().st_size > 0


def test_production_refuses_demo_secret_and_invite(tmp_path, monkeypatch):
    monkeypatch.setenv("DEBUG", "0")
    monkeypatch.setenv("DATABASE_PATH", str(tmp_path / "bookclub.db"))
    monkeypatch.setenv("SECRET_KEY", "dev-secret-change-me")
    monkeypatch.setenv("BOOKCLUB_BOOTSTRAP_INVITE", "FRIENDS")
    with pytest.raises(RuntimeError, match="SECRET_KEY"):
        prepare_environment()

    monkeypatch.setenv("SECRET_KEY", "a" * 32)
    monkeypatch.setenv("BOOKCLUB_BOOTSTRAP_INVITE", "DEV-ONLY")
    with pytest.raises(RuntimeError, match="DEV-ONLY"):
        prepare_environment()


def test_public_config_and_cors_origins():
    branded = Settings(
        bookclub_name="Thursday Readers",
        bookclub_theme="#224466",
        bookclub_public_url="https://books.example.com/app",
        debug=False,
    )
    config = public_config(branded)
    assert config["name"] == "Thursday Readers"
    assert config["theme"] == "#224466"
    assert config["theme_dark"].startswith("#")
    assert config["public_url"] == "https://books.example.com"
    assert cors_origins(branded) == ["https://books.example.com"]
    assert "ThursdayReaders/1.0" in open_library_ua(branded)
    assert "https://books.example.com" in open_library_ua(branded)

    dev = Settings(debug=True, bookclub_public_url="https://books.example.com")
    origins = cors_origins(dev)
    assert "http://localhost:5173" in origins
    assert "https://books.example.com" in origins

    with pytest.raises(ValidationError):
        Settings(bookclub_public_url="not-a-url")


def test_branded_app_config_cors_and_user_agent(tmp_path, monkeypatch):
    with _production_client(
        tmp_path,
        monkeypatch,
        BOOKCLUB_NAME="Thursday Readers",
        BOOKCLUB_THEME="#336699",
        BOOKCLUB_PUBLIC_URL="https://books.example.com",
    ) as client:
        assert client.app.title == "Thursday Readers"
        assert client.get("/api/docs").status_code == 404
        body = client.get("/api/config").json()
        assert body["name"] == "Thursday Readers"
        assert body["theme"] == "#336699"
        assert body["public_url"] == "https://books.example.com"
        manifest = client.get("/manifest.webmanifest").json()
        assert manifest["name"] == "Thursday Readers"
        assert manifest["theme_color"] == "#336699"

        allowed = client.options(
            "/api/health",
            headers={
                "Origin": "https://books.example.com",
                "Access-Control-Request-Method": "GET",
            },
        )
        assert allowed.headers.get("access-control-allow-origin") == (
            "https://books.example.com"
        )
        denied = client.options(
            "/api/health",
            headers={
                "Origin": "http://localhost:5173",
                "Access-Control-Request-Method": "GET",
            },
        )
        assert denied.headers.get("access-control-allow-origin") != (
            "http://localhost:5173"
        )


def test_debug_cors_keeps_vite(client):
    config = client.get("/api/config").json()
    assert config["name"] == "Bookclub"
    preflight = client.options(
        "/api/health",
        headers={
            "Origin": "http://localhost:5173",
            "Access-Control-Request-Method": "GET",
        },
    )
    assert preflight.headers.get("access-control-allow-origin") == (
        "http://localhost:5173"
    )
    assert preflight.headers.get("access-control-allow-credentials") == "true"
