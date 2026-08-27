from tests.conftest import register


def test_backup_requires_auth(client):
    assert client.get("/api/backup").status_code == 401


def test_backup_returns_sqlite_for_member(client):
    register(client, "ada")
    response = client.get("/api/backup")
    assert response.status_code == 200
    assert "sqlite" in response.headers["content-type"]
    assert "bookclub.db" in response.headers.get("content-disposition", "")
    assert response.content.startswith(b"SQLite format 3")
