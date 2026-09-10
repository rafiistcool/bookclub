import threading

from fastapi.testclient import TestClient

from tests.conftest import login, register


def _add(client, *, work: str, title: str, **overrides):
    payload = {
        "ol_work_key": work,
        "title": title,
        "authors": "An Author",
        "cover_id": 1,
        "year": 2018,
        "status": "want_to_read",
        **overrides,
    }
    return client.post("/api/shelf", json=payload)


def _book_id(client, title: str) -> int:
    items = client.get("/api/shelf").json()["items"]
    return next(row["book"]["id"] for row in items if row["book"]["title"] == title)


def test_replace_order_clear_and_eligibility(client):
    register(client, "ada")
    _add(client, work="/works/OL1W", title="Circe")
    _add(client, work="/works/OL2W", title="Song of Achilles")
    _add(client, work="/works/OL3W", title="Galatea")
    circe = _book_id(client, "Circe")
    song = _book_id(client, "Song of Achilles")
    galatea = _book_id(client, "Galatea")

    empty = client.get("/api/auth/me/favorites")
    assert empty.status_code == 200
    assert empty.json() == {"items": []}
    assert client.get("/api/shelf").json()["favorites"] == []

    replaced = client.put("/api/auth/me/favorites", json={"book_ids": [song, circe]})
    assert replaced.status_code == 200, replaced.text
    titles = [row["book"]["title"] for row in replaced.json()["items"]]
    assert titles == ["Song of Achilles", "Circe"]
    assert [row["position"] for row in replaced.json()["items"]] == [1, 2]

    reordered = client.put("/api/auth/me/favorites", json={"book_ids": [circe, galatea, song]})
    assert reordered.status_code == 200
    assert [row["book"]["title"] for row in reordered.json()["items"]] == [
        "Circe",
        "Galatea",
        "Song of Achilles",
    ]
    assert [row["position"] for row in reordered.json()["items"]] == [1, 2, 3]

    shelf = client.get("/api/shelf").json()
    assert [row["book"]["title"] for row in shelf["favorites"]] == [
        "Circe",
        "Galatea",
        "Song of Achilles",
    ]
    assert all("cover_url" in row["book"] for row in shelf["favorites"])

    detail = client.get("/api/books/works/OL1W").json()
    assert detail["id"] == circe
    assert detail["favorite_position"] == 1
    assert client.get("/api/books/works/OL2W").json()["favorite_position"] == 3
    assert client.get("/api/books/works/OL3W").json()["favorite_position"] == 2

    cleared = client.put("/api/auth/me/favorites", json={"book_ids": []})
    assert cleared.status_code == 200
    assert cleared.json() == {"items": []}
    assert client.get("/api/shelf").json()["favorites"] == []
    assert client.get("/api/books/works/OL1W").json()["favorite_position"] is None


def test_unknown_and_duplicate_book_ids_are_rejected(client):
    register(client, "ada")
    _add(client, work="/works/OL1W", title="Circe")
    circe = _book_id(client, "Circe")

    unknown = client.put("/api/auth/me/favorites", json={"book_ids": [circe, 9999]})
    assert unknown.status_code == 400
    assert "not in the club" in unknown.json()["detail"]
    assert client.get("/api/auth/me/favorites").json() == {"items": []}

    duplicate = client.put("/api/auth/me/favorites", json={"book_ids": [circe, circe]})
    assert duplicate.status_code == 400
    assert "once" in duplicate.json()["detail"]

    _add(client, work="/works/OL2W", title="Song of Achilles")
    _add(client, work="/works/OL3W", title="Galatea")
    _add(client, work="/works/OL4W", title="Circe leftover")
    too_many = client.put(
        "/api/auth/me/favorites",
        json={
            "book_ids": [
                circe,
                _book_id(client, "Song of Achilles"),
                _book_id(client, "Galatea"),
                _book_id(client, "Circe leftover"),
            ]
        },
    )
    assert too_many.status_code == 400
    assert "At most 3" in too_many.json()["detail"]
    assert client.get("/api/auth/me/favorites").json() == {"items": []}


def test_favourite_does_not_require_own_shelf(client):
    register(client, "ada")
    _add(client, work="/works/OL1W", title="Circe")
    circe = _book_id(client, "Circe")
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)

    added = client.post(f"/api/auth/me/favorites/{circe}")
    assert added.status_code == 200, added.text
    assert [row["book"]["title"] for row in added.json()["items"]] == ["Circe"]
    assert client.get("/api/shelf").json()["items"] == []
    assert client.get("/api/shelf").json()["favorites"][0]["book"]["title"] == "Circe"

    ada = client.get("/api/shelf", params={"username": "ada"}).json()
    assert ada["favorites"] == []


def test_add_helper_appends_and_rejects_a_fourth(client):
    register(client, "ada")
    _add(client, work="/works/OL1W", title="Circe")
    _add(client, work="/works/OL2W", title="Song of Achilles")
    _add(client, work="/works/OL3W", title="Galatea")
    _add(client, work="/works/OL4W", title="Circe leftover")
    ids = [
        _book_id(client, "Circe"),
        _book_id(client, "Song of Achilles"),
        _book_id(client, "Galatea"),
        _book_id(client, "Circe leftover"),
    ]

    assert client.post(f"/api/auth/me/favorites/{ids[0]}").status_code == 200
    again = client.post(f"/api/auth/me/favorites/{ids[0]}")
    assert again.status_code == 200
    assert len(again.json()["items"]) == 1

    client.post(f"/api/auth/me/favorites/{ids[1]}")
    client.post(f"/api/auth/me/favorites/{ids[2]}")
    full = client.post(f"/api/auth/me/favorites/{ids[3]}")
    assert full.status_code == 409
    assert "3 favourites" in full.json()["detail"]
    titles = [row["book"]["title"] for row in client.get("/api/auth/me/favorites").json()["items"]]
    assert titles == ["Circe", "Song of Achilles", "Galatea"]

    unknown = client.post("/api/auth/me/favorites/9999")
    assert unknown.status_code == 400
    assert "not in the club" in unknown.json()["detail"]


def test_remove_helper_compacts_positions(client):
    register(client, "ada")
    _add(client, work="/works/OL1W", title="Circe")
    _add(client, work="/works/OL2W", title="Song of Achilles")
    _add(client, work="/works/OL3W", title="Galatea")
    circe = _book_id(client, "Circe")
    song = _book_id(client, "Song of Achilles")
    galatea = _book_id(client, "Galatea")
    client.put("/api/auth/me/favorites", json={"book_ids": [circe, song, galatea]})

    removed = client.delete(f"/api/auth/me/favorites/{song}")
    assert removed.status_code == 200
    items = removed.json()["items"]
    assert [row["book"]["title"] for row in items] == ["Circe", "Galatea"]
    assert [row["position"] for row in items] == [1, 2]

    missing = client.delete(f"/api/auth/me/favorites/{song}")
    assert missing.status_code == 200
    assert [row["book"]["title"] for row in missing.json()["items"]] == ["Circe", "Galatea"]


def test_member_shelf_shows_their_favourites_not_yours(client):
    register(client, "ada")
    _add(client, work="/works/OL1W", title="Circe")
    _add(client, work="/works/OL2W", title="Song of Achilles")
    circe = _book_id(client, "Circe")
    song = _book_id(client, "Song of Achilles")
    client.put("/api/auth/me/favorites", json={"book_ids": [circe]})
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    client.put("/api/auth/me/favorites", json={"book_ids": [song]})

    mine = client.get("/api/shelf").json()["favorites"]
    assert [row["book"]["title"] for row in mine] == ["Song of Achilles"]
    theirs = client.get("/api/shelf", params={"username": "ada"}).json()["favorites"]
    assert [row["book"]["title"] for row in theirs] == ["Circe"]


def test_omitted_book_ids_clears_favourites(client):
    register(client, "ada")
    _add(client, work="/works/OL1W", title="Circe")
    circe = _book_id(client, "Circe")
    client.put("/api/auth/me/favorites", json={"book_ids": [circe]})
    cleared = client.put("/api/auth/me/favorites", json={})
    assert cleared.status_code == 200
    assert cleared.json() == {"items": []}


def test_concurrent_add_for_last_slot_is_409_not_500(client):
    register(client, "ada")
    _add(client, work="/works/OL1W", title="Circe")
    _add(client, work="/works/OL2W", title="Song of Achilles")
    _add(client, work="/works/OL3W", title="Galatea")
    _add(client, work="/works/OL4W", title="Circe leftover")
    circe = _book_id(client, "Circe")
    song = _book_id(client, "Song of Achilles")
    galatea = _book_id(client, "Galatea")
    leftover = _book_id(client, "Circe leftover")
    client.put("/api/auth/me/favorites", json={"book_ids": [circe, song]})

    statuses: list[int] = []
    lock = threading.Lock()
    start = threading.Barrier(2)

    def attempt(book_id: int) -> None:
        local = TestClient(client.app)
        assert login(local, "ada").status_code == 204
        start.wait()
        response = local.post(f"/api/auth/me/favorites/{book_id}")
        with lock:
            statuses.append(response.status_code)

    threads = [
        threading.Thread(target=attempt, args=(galatea,)),
        threading.Thread(target=attempt, args=(leftover,)),
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=20)

    assert 500 not in statuses
    assert sorted(statuses) == [200, 409]
    assert len(client.get("/api/auth/me/favorites").json()["items"]) == 3


def test_favorites_require_a_session(client):
    assert client.get("/api/auth/me/favorites").status_code == 401
    assert client.put("/api/auth/me/favorites", json={"book_ids": []}).status_code == 401
    assert client.post("/api/auth/me/favorites/1").status_code == 401
    assert client.delete("/api/auth/me/favorites/1").status_code == 401
