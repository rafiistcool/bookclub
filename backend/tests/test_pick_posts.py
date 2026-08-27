from tests.conftest import register


CIRCE = {
    "ol_work_key": "/works/OL1W",
    "title": "Circe",
    "authors": "Madeline Miller",
    "cover_id": 123,
    "year": 2018,
}

ACHILLES = {
    "ol_work_key": "/works/OL2W",
    "title": "The Song of Achilles",
    "authors": "Madeline Miller",
    "cover_id": 456,
    "year": 2011,
}


def test_posts_require_auth_and_a_pick(client):
    assert client.get("/api/pick/posts").status_code == 401
    assert client.post("/api/pick/posts", json={"body": "hello"}).status_code == 401
    register(client, "ada")
    assert client.get("/api/pick/posts").status_code == 404
    assert client.post("/api/pick/posts", json={"body": "hello"}).status_code == 404


def test_members_can_post_and_read_oldest_first(client):
    register(client, "ada")
    pick_id = client.put("/api/pick", json=CIRCE).json()["id"]
    first = client.post("/api/pick/posts", json={"body": "  Opening note  "})
    assert first.status_code == 201
    assert first.json()["author"] == "ada"
    assert first.json()["body"] == "Opening note"
    assert first.json()["created_label"]

    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    second = client.post("/api/pick/posts", json={"body": "Grace was here"})
    assert second.status_code == 201

    thread = client.get("/api/pick/posts").json()
    assert thread["pick_id"] == pick_id
    assert thread["can_post"] is True
    assert [row["body"] for row in thread["items"]] == ["Opening note", "Grace was here"]
    assert [row["author"] for row in thread["items"]] == ["ada", "grace"]

    by_id = client.get(f"/api/pick/{pick_id}/posts").json()
    assert [row["body"] for row in by_id["items"]] == ["Opening note", "Grace was here"]


def test_old_thread_stays_when_pick_changes(client):
    register(client, "ada")
    old_id = client.put("/api/pick", json=CIRCE).json()["id"]
    client.post("/api/pick/posts", json={"body": "Circe notes"})
    new_pick = client.put("/api/pick", json=ACHILLES).json()
    new_id = new_pick["id"]
    assert new_id != old_id

    current = client.get("/api/pick/posts").json()
    assert current["pick_id"] == new_id
    assert current["items"] == []
    assert current["can_post"] is True

    archived = client.get(f"/api/pick/{old_id}/posts").json()
    assert archived["can_post"] is False
    assert [row["body"] for row in archived["items"]] == ["Circe notes"]

    blocked = client.post(f"/api/pick/{old_id}/posts", json={"body": "too late"})
    assert blocked.status_code == 400


def test_post_time_uses_club_timezone(client, monkeypatch):
    monkeypatch.setenv("BOOKCLUB_TZ", "America/New_York")
    from app.config import get_settings

    get_settings.cache_clear()
    register(client, "ada")
    client.put("/api/pick", json=CIRCE)
    post = client.post("/api/pick/posts", json={"body": "Hello from the meeting"}).json()
    assert "EDT" in post["created_label"] or "EST" in post["created_label"]
    thread = client.get("/api/pick/posts").json()
    assert thread["timezone"] == "America/New_York"


def test_invalid_post_body(client):
    register(client, "ada")
    client.put("/api/pick", json=CIRCE)
    empty = client.post("/api/pick/posts", json={"body": "   "})
    assert empty.status_code == 400
    long = client.post("/api/pick/posts", json={"body": "x" * 1001})
    assert long.status_code == 400
