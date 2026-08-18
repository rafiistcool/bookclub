from tests.conftest import register


BOOK = {
    "ol_work_key": "/works/OL1W",
    "title": "Circe",
    "authors": "Madeline Miller",
    "cover_id": 123,
    "year": 2018,
    "status": "want_to_read",
}


def _add(client, **overrides):
    payload = {**BOOK, **overrides}
    return client.post("/api/shelf", json=payload)


def test_add_move_remove_and_unique(client):
    register(client, "ada")
    created = _add(client)
    assert created.status_code == 201
    item = created.json()
    assert item["status"] == "want_to_read"
    assert item["position"] == 0
    assert item["book"]["title"] == "Circe"

    duplicate = _add(client)
    assert duplicate.status_code == 409
    assert duplicate.json()["item"]["id"] == item["id"]

    moved = client.patch(f"/api/shelf/{item['id']}", json={"status": "currently_reading"})
    assert moved.status_code == 200
    assert moved.json()["status"] == "currently_reading"

    shelf = client.get("/api/shelf").json()
    assert len(shelf["items"]) == 1
    assert shelf["items"][0]["status"] == "currently_reading"

    assert client.delete(f"/api/shelf/{item['id']}").status_code == 204
    assert client.get("/api/shelf").json()["items"] == []


def test_cannot_edit_someone_elses_row(client):
    register(client, "ada")
    item_id = _add(client).json()["id"]
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    assert client.patch(f"/api/shelf/{item_id}", json={"status": "finished"}).status_code == 404
    assert client.delete(f"/api/shelf/{item_id}").status_code == 404
    friend = client.get("/api/shelf", params={"username": "ada"})
    assert friend.status_code == 200
    assert len(friend.json()["items"]) == 1


def test_reorder_within_and_across_columns(client):
    register(client, "ada")
    first = _add(client, ol_work_key="/works/OL1W", title="Circe").json()
    second = _add(client, ol_work_key="/works/OL2W", title="Song of Achilles").json()
    # newest insert is at the top
    items = client.get("/api/shelf").json()["items"]
    assert [row["book"]["title"] for row in items] == ["Song of Achilles", "Circe"]
    assert items[0]["position"] == 0
    assert items[1]["position"] == 1

    client.patch(f"/api/shelf/{first['id']}", json={"position": 0, "status": "want_to_read"})
    titles = [row["book"]["title"] for row in client.get("/api/shelf").json()["items"]]
    assert titles == ["Circe", "Song of Achilles"]

    client.patch(
        f"/api/shelf/{second['id']}",
        json={"status": "currently_reading", "position": 0},
    )
    shelf = client.get("/api/shelf").json()["items"]
    want = [row for row in shelf if row["status"] == "want_to_read"]
    reading = [row for row in shelf if row["status"] == "currently_reading"]
    assert [row["book"]["title"] for row in want] == ["Circe"]
    assert want[0]["position"] == 0
    assert [row["book"]["title"] for row in reading] == ["Song of Achilles"]


def test_unknown_member_shelf_is_404(client):
    register(client, "ada")
    assert client.get("/api/shelf", params={"username": "nobody"}).status_code == 404
