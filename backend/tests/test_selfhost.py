from fastapi.testclient import TestClient

from app.config import get_settings
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
    monkeypatch.setenv("DATABASE_PATH", str(tmp_path / "bookclub.db"))
    monkeypatch.setenv("DEBUG", "0")
    monkeypatch.setenv("SECRET_KEY", env.get("SECRET_KEY", "p" * 32))
    monkeypatch.setenv(
        "BOOKCLUB_BOOTSTRAP_INVITE", env.get("BOOKCLUB_BOOTSTRAP_INVITE", "SELFHOST1")
    )
    monkeypatch.setenv("BOOKCLUB_HTTPS", env.get("BOOKCLUB_HTTPS", "auto"))
    get_settings.cache_clear()
    reset_rate_limits()
    from app.main import create_app

    return TestClient(create_app(), base_url=env.get("base_url", "http://test"))


def test_production_app_registers_and_hides_docs(tmp_path, monkeypatch):
    with _production_client(tmp_path, monkeypatch) as client:
        assert client.get("/api/health").json() == {"ok": True}
        assert client.get("/api/docs").status_code == 404
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
