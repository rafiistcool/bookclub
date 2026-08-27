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


def test_finish_rating_and_take_show_on_friend_shelf(client):
    register(client, "ada")
    item_id = _add(client).json()["id"]
    moved = client.patch(
        f"/api/shelf/{item_id}",
        json={"status": "finished", "rating": 4, "take": "  Witchy and sad  "},
    )
    assert moved.status_code == 200
    assert moved.json()["status"] == "finished"
    assert moved.json()["rating"] == 4
    assert moved.json()["take"] == "Witchy and sad"
    assert moved.json()["dnf_reason"] == ""

    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    friend = client.get("/api/shelf", params={"username": "ada"}).json()["items"][0]
    assert friend["rating"] == 4
    assert friend["take"] == "Witchy and sad"


def test_dnf_reason_shows_on_friend_shelf(client):
    register(client, "ada")
    item_id = _add(client).json()["id"]
    moved = client.patch(
        f"/api/shelf/{item_id}",
        json={"status": "did_not_finish", "dnf_reason": "  Too grim  "},
    )
    assert moved.status_code == 200
    assert moved.json()["status"] == "did_not_finish"
    assert moved.json()["dnf_reason"] == "Too grim"
    assert moved.json()["rating"] is None
    assert moved.json()["take"] == ""

    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    friend = client.get("/api/shelf", params={"username": "ada"}).json()["items"][0]
    assert friend["dnf_reason"] == "Too grim"
    assert friend["rating"] is None


def test_dnf_reason_is_optional(client):
    register(client, "ada")
    item_id = _add(client).json()["id"]
    moved = client.patch(f"/api/shelf/{item_id}", json={"status": "did_not_finish"})
    assert moved.status_code == 200
    assert moved.json()["dnf_reason"] == ""


def test_moving_off_finished_clears_notes(client):
    register(client, "ada")
    item_id = _add(client, status="finished", rating=5, take="Loved it").json()["id"]
    moved = client.patch(f"/api/shelf/{item_id}", json={"status": "currently_reading"})
    assert moved.json()["rating"] is None
    assert moved.json()["take"] == ""
    assert moved.json()["dnf_reason"] == ""


def test_finish_notes_appear_on_club_pick(client):
    register(client, "ada")
    client.put(
        "/api/pick",
        json={
            "ol_work_key": BOOK["ol_work_key"],
            "title": BOOK["title"],
            "authors": BOOK["authors"],
            "cover_id": BOOK["cover_id"],
            "year": BOOK["year"],
        },
    )
    item_id = _add(client).json()["id"]
    client.patch(
        f"/api/shelf/{item_id}",
        json={"status": "finished", "rating": 5, "take": "What a book"},
    )
    pick = client.get("/api/pick").json()["pick"]
    assert pick["finished"] == ["ada"]
    ada = next(row for row in pick["readers"] if row["username"] == "ada")
    assert ada["rating"] == 5
    assert ada["take"] == "What a book"


def test_reading_progress_shows_on_friend_shelf_and_pick(client):
    register(client, "ada")
    client.put(
        "/api/pick",
        json={
            "ol_work_key": BOOK["ol_work_key"],
            "title": BOOK["title"],
            "authors": BOOK["authors"],
            "cover_id": BOOK["cover_id"],
            "year": BOOK["year"],
        },
    )
    item_id = _add(client, status="currently_reading").json()["id"]
    updated = client.patch(f"/api/shelf/{item_id}", json={"progress": 35})
    assert updated.status_code == 200
    assert updated.json()["status"] == "currently_reading"
    assert updated.json()["progress"] == 35

    pick = client.get("/api/pick").json()["pick"]
    assert pick["reading"] == ["ada"]
    ada = next(row for row in pick["readers"] if row["username"] == "ada")
    assert ada["progress"] == 35

    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    friend = client.get("/api/shelf", params={"username": "ada"}).json()["items"][0]
    assert friend["progress"] == 35
    pick = client.get("/api/pick").json()["pick"]
    assert pick["readers"][0]["progress"] == 35


def test_progress_is_optional_and_clears_when_leaving_reading(client):
    register(client, "ada")
    created = _add(client, status="currently_reading")
    assert created.json()["progress"] is None
    item_id = created.json()["id"]
    client.patch(f"/api/shelf/{item_id}", json={"progress": 80})
    moved = client.patch(f"/api/shelf/{item_id}", json={"status": "finished"})
    assert moved.json()["progress"] is None


def test_invalid_progress(client):
    register(client, "ada")
    item_id = _add(client, status="currently_reading").json()["id"]
    low = client.patch(f"/api/shelf/{item_id}", json={"progress": -1})
    assert low.status_code == 400
    high = client.patch(f"/api/shelf/{item_id}", json={"progress": 101})
    assert high.status_code == 400


def test_invalid_finish_fields(client):
    register(client, "ada")
    item_id = _add(client).json()["id"]
    low = client.patch(f"/api/shelf/{item_id}", json={"status": "finished", "rating": 0})
    assert low.status_code == 400
    high = client.patch(f"/api/shelf/{item_id}", json={"status": "finished", "rating": 6})
    assert high.status_code == 400
    long_take = client.patch(
        f"/api/shelf/{item_id}",
        json={"status": "finished", "take": "x" * 141},
    )
    assert long_take.status_code == 400
    long_reason = client.patch(
        f"/api/shelf/{item_id}",
        json={"status": "did_not_finish", "dnf_reason": "x" * 201},
    )
    assert long_reason.status_code == 400
