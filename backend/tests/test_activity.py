from tests.conftest import register

CIRCE = {
    "ol_work_key": "/works/OL1W",
    "title": "Circe",
    "authors": "Madeline Miller",
    "cover_id": 123,
    "year": 2018,
}
ACHILLES = {**CIRCE, "ol_work_key": "/works/OL2W", "title": "The Song of Achilles"}


def _kinds(client, **params):
    body = client.get("/api/activity", params=params).json()
    return [(row["actor"], row["kind"]) for row in body["items"]]


def test_feed_requires_auth(client):
    assert client.get("/api/activity").status_code == 401


def test_feed_records_the_club_story_newest_first(client):
    register(client, "ada")
    assert _kinds(client) == [("ada", "member_joined")]

    client.post("/api/shelf", json={**CIRCE, "status": "want_to_read"})
    shelf_id = client.get("/api/shelf").json()["items"][0]["id"]
    client.patch(f"/api/shelf/{shelf_id}", json={"status": "currently_reading", "position": 0})
    client.patch(f"/api/shelf/{shelf_id}", json={"progress": 50})
    client.patch(f"/api/shelf/{shelf_id}", json={"progress": 50})  # unchanged: no event
    client.patch(f"/api/shelf/{shelf_id}", json={"status": "finished", "position": 0, "rating": 4, "take": "Great"})
    client.put("/api/pick", json=ACHILLES)
    post = client.post("/api/pick/posts", json={"body": "Chapter one!"}).json()
    client.post(f"/api/pick/posts/{post['id']}/reactions", json={"emoji": "❤️"})
    client.post("/api/vote/nominations", json=CIRCE)
    nomination_id = client.get("/api/vote").json()["nominations"][0]["id"]
    client.post("/api/vote/cast", json={"nomination_id": nomination_id})
    client.post("/api/vote/cast", json={"nomination_id": nomination_id})  # re-cast: no event
    client.delete(f"/api/shelf/{shelf_id}")

    kinds = [kind for _, kind in _kinds(client)]
    assert kinds == [
        "shelf_removed",
        "voted",
        "nominated",
        "reaction",
        "note_posted",
        "pick_set",
        "shelf_finished",
        "progress",
        "shelf_moved",
        "shelf_added",
        "member_joined",
    ]

    items = client.get("/api/activity").json()["items"]
    finished = next(row for row in items if row["kind"] == "shelf_finished")
    assert finished["book"]["title"] == "Circe"
    assert finished["payload"] == {"rating": 4, "take": "Great"}
    assert finished["mine"] is True
    assert finished["created_label"]
    progress = next(row for row in items if row["kind"] == "progress")
    assert progress["payload"] == {"progress": 50}
    reaction = next(row for row in items if row["kind"] == "reaction")
    assert reaction["payload"]["emoji"] == "❤️"
    assert reaction["payload"]["author"] == "ada"
    pick_set = next(row for row in items if row["kind"] == "pick_set")
    assert pick_set["pick_id"] is not None
    assert pick_set["book"]["title"] == "The Song of Achilles"


def test_feed_paginates_and_filters_by_member(client):
    register(client, "ada")
    for index in range(5):
        client.post(
            "/api/shelf",
            json={**CIRCE, "ol_work_key": f"/works/OL{index}W", "title": f"Book {index}", "status": "want_to_read"},
        )
    first = client.get("/api/activity", params={"limit": 3}).json()
    assert len(first["items"]) == 3
    assert first["has_more"] is True
    last_id = first["items"][-1]["id"]
    second = client.get("/api/activity", params={"limit": 3, "before": last_id}).json()
    assert len(second["items"]) == 3
    assert second["has_more"] is False
    assert {row["id"] for row in first["items"]}.isdisjoint({row["id"] for row in second["items"]})

    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    assert _kinds(client, username="grace") == [("grace", "member_joined")]
    assert all(actor == "ada" for actor, _ in _kinds(client, username="ada"))
    assert client.get("/api/activity", params={"username": "nobody"}).json()["items"] == []
    mine = client.get("/api/activity", params={"username": "grace"}).json()["items"][0]
    assert mine["mine"] is True
