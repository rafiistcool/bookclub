import sqlite3
import threading

from fastapi.testclient import TestClient

from tests.conftest import login, register


def test_health(client):
    assert client.get("/api/health").json() == {"ok": True}


def test_register_consumes_invite_and_sets_session(client):
    response = register(client, "ada")
    assert response.status_code == 201
    assert response.json() == {
        "id": 1,
        "username": "ada",
        "theme": "paper",
        "color_mode": "system",
    }
    me = client.get("/api/auth/me")
    assert me.status_code == 200
    assert me.json()["username"] == "ada"


def test_invalid_invite_does_not_create_user(client):
    response = register(client, "ada", invite="NOPE")
    assert response.status_code == 400
    assert login(client, "ada").status_code == 401


def test_invite_cannot_be_reused(client):
    assert register(client, "ada").status_code == 201
    client.post("/api/auth/logout")
    again = register(client, "grace")
    assert again.status_code == 400
    assert "invite" in again.json()["detail"].lower()


def test_invite_cannot_be_used_twice_concurrently(client):
    statuses: list[int] = []
    lock = threading.Lock()
    start = threading.Barrier(2)

    def attempt(username: str) -> None:
        local = TestClient(client.app)
        start.wait()
        response = local.post(
            "/api/auth/register",
            json={
                "username": username,
                "password": "password1",
                "invite_code": "TESTINVITE",
            },
        )
        with lock:
            statuses.append(response.status_code)

    threads = [
        threading.Thread(target=attempt, args=("ada",)),
        threading.Thread(target=attempt, args=("grace",)),
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=20)

    assert sorted(statuses) == [201, 400]
    ada_ok = login(client, "ada").status_code == 204
    client.post("/api/auth/logout")
    grace_ok = login(client, "grace").status_code == 204
    assert ada_ok ^ grace_ok


def test_username_taken(client):
    assert register(client, "ada").status_code == 201
    minted = client.post("/api/invites")
    assert minted.status_code == 201
    client.post("/api/auth/logout")
    taken = register(client, "Ada", invite=minted.json()["code"])
    assert taken.status_code == 409


def test_bad_username_and_short_password(client):
    bad_name = register(client, "Ada Lovelace")
    assert bad_name.status_code == 400
    short = client.post(
        "/api/auth/register",
        json={"username": "ada", "password": "short", "invite_code": "TESTINVITE"},
    )
    assert short.status_code == 400


def test_me_requires_session(client):
    assert client.get("/api/auth/me").status_code == 401


def test_login_and_logout(client):
    register(client, "ada")
    client.post("/api/auth/logout")
    assert client.get("/api/auth/me").status_code == 401
    assert login(client, "ada").status_code == 204
    assert client.get("/api/auth/me").json()["username"] == "ada"
    assert login(client, "ada", "wrong-password").status_code == 401


def test_login_throttle(client):
    register(client, "ada")
    client.post("/api/auth/logout")
    for _ in range(10):
        assert login(client, "ada", "nope").status_code == 401
    blocked = login(client, "ada", "nope")
    assert blocked.status_code == 429


def test_me_carries_default_preferences(client):
    register(client, "ada")
    me = client.get("/api/auth/me").json()
    assert me["theme"] == "paper"
    assert me["color_mode"] == "system"


def test_update_theme_alone_leaves_color_mode(client):
    register(client, "ada")
    response = client.patch("/api/auth/me/preferences", json={"theme": "forest"})
    assert response.status_code == 200
    assert response.json() == {
        "id": 1,
        "username": "ada",
        "theme": "forest",
        "color_mode": "system",
    }


def test_update_color_mode_alone_leaves_theme(client):
    register(client, "ada")
    response = client.patch("/api/auth/me/preferences", json={"color_mode": "dark"})
    assert response.status_code == 200
    assert response.json()["theme"] == "paper"
    assert response.json()["color_mode"] == "dark"


def test_update_both_preferences(client):
    register(client, "ada")
    response = client.patch(
        "/api/auth/me/preferences", json={"theme": "ink", "color_mode": "light"}
    )
    assert response.status_code == 200
    assert response.json()["theme"] == "ink"
    assert response.json()["color_mode"] == "light"


def test_empty_preferences_payload_is_a_no_op(client):
    register(client, "ada")
    client.patch("/api/auth/me/preferences", json={"theme": "slate"})
    response = client.patch("/api/auth/me/preferences", json={})
    assert response.status_code == 200
    assert response.json()["theme"] == "slate"


def test_preferences_persist_across_sessions(client):
    register(client, "ada")
    client.patch(
        "/api/auth/me/preferences", json={"theme": "slate", "color_mode": "dark"}
    )
    client.post("/api/auth/logout")
    assert login(client, "ada").status_code == 204
    me = client.get("/api/auth/me").json()
    assert me["theme"] == "slate"
    assert me["color_mode"] == "dark"


def test_invalid_theme_rejected(client):
    register(client, "ada")
    response = client.patch("/api/auth/me/preferences", json={"theme": "neon"})
    assert response.status_code == 400
    assert "paper" in response.json()["detail"]
    assert client.get("/api/auth/me").json()["theme"] == "paper"


def test_invalid_color_mode_rejected(client):
    register(client, "ada")
    response = client.patch("/api/auth/me/preferences", json={"color_mode": "sepia"})
    assert response.status_code == 400
    assert "color_mode" in response.json()["detail"]
    assert client.get("/api/auth/me").json()["color_mode"] == "system"


def test_preferences_require_a_session(client):
    register(client, "ada")
    client.post("/api/auth/logout")
    response = client.patch("/api/auth/me/preferences", json={"theme": "ink"})
    assert response.status_code == 401


def test_preference_columns_backfill_pre_existing_users(tmp_path):
    from app.db import init_db

    path = tmp_path / "legacy.db"
    legacy = sqlite3.connect(path)
    legacy.execute(
        "CREATE TABLE users ("
        " id INTEGER NOT NULL PRIMARY KEY,"
        " username VARCHAR(32) NOT NULL,"
        " password_hash VARCHAR NOT NULL,"
        " created_at DATETIME NOT NULL)"
    )
    legacy.execute(
        "INSERT INTO users (id, username, password_hash, created_at)"
        " VALUES (1, 'ada', 'hash', '2024-01-01 00:00:00')"
    )
    legacy.commit()
    legacy.close()

    engine = init_db(path)
    try:
        # Re-running must not fail on the already-added columns.
        init_db(path).dispose()
        probe = sqlite3.connect(path)
        row = probe.execute("SELECT theme, color_mode FROM users WHERE id = 1").fetchone()
        probe.close()
    finally:
        engine.dispose()
    assert row == ("paper", "system")
