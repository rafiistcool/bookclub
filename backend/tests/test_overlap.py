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


def _add(client, book, status="want_to_read"):
    return client.post("/api/shelf", json={**book, "status": status})


def _two_members(client):
    register(client, "ada")
    invite = client.post("/api/invites").json()["code"]
    return invite


def test_overlap_requires_auth(client):
    assert client.get("/api/overlap").status_code == 401


def test_two_want_same_book_shows_one_does_not(client):
    invite = _two_members(client)
    _add(client, CIRCE)
    _add(client, ACHILLES)

    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    _add(client, CIRCE)

    overlap = client.get("/api/overlap").json()
    assert overlap["include_reading"] is False
    titles = [row["book"]["title"] for row in overlap["items"]]
    assert titles == ["Circe"]
    row = overlap["items"][0]
    assert row["count"] == 2
    assert [m["username"] for m in row["members"]] == ["ada", "grace"]
    assert all(m["status"] == "want_to_read" for m in row["members"])


def test_finished_and_solo_want_are_excluded(client):
    invite = _two_members(client)
    _add(client, CIRCE, "finished")
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    _add(client, CIRCE)
    assert client.get("/api/overlap").json()["items"] == []


def test_include_reading_counts_want_plus_reading(client):
    invite = _two_members(client)
    _add(client, CIRCE, "currently_reading")
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    _add(client, CIRCE)

    hidden = client.get("/api/overlap").json()["items"]
    assert hidden == []

    shown = client.get("/api/overlap", params={"include_reading": True}).json()
    assert shown["include_reading"] is True
    assert shown["items"][0]["book"]["title"] == "Circe"
    assert shown["items"][0]["count"] == 2
    statuses = {row["username"]: row["status"] for row in shown["items"][0]["members"]}
    assert statuses == {"ada": "currently_reading", "grace": "want_to_read"}
