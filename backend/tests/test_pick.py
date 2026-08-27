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


def test_pick_requires_auth(client):
    assert client.get("/api/pick").status_code == 401
    assert client.put("/api/pick", json=CIRCE).status_code == 401
    assert client.delete("/api/pick").status_code == 401


def test_empty_pick_then_set_and_read(client):
    register(client, "ada")
    empty = client.get("/api/pick")
    assert empty.status_code == 200
    assert empty.json()["pick"] is None
    assert empty.json()["timezone"] == "UTC"
    assert client.get("/api/config").json()["timezone"] == "UTC"

    created = client.put("/api/pick", json={**CIRCE, "note": "August"})
    assert created.status_code == 200
    body = created.json()
    assert body["book"]["title"] == "Circe"
    assert body["set_by"] == "ada"
    assert body["note"] == "August"
    assert body["ended_at"] is None
    assert body["meeting_at"] is None
    assert body["on_shelf"] is None
    assert body["readers"] == []
    assert body["reading"] == []
    assert body["finished"] == []

    current = client.get("/api/pick").json()["pick"]
    assert current["book"]["ol_work_key"] == "/works/OL1W"
    assert current["set_by"] == "ada"


def test_same_book_updates_meeting_without_history(client):
    register(client, "ada")
    client.put("/api/pick", json={**CIRCE, "meeting_at": "2026-08-22T19:00"})
    again = client.put("/api/pick", json={**CIRCE, "meeting_at": "2026-08-29T18:30"})
    assert again.status_code == 200
    assert again.json()["meeting_local"] == "2026-08-29T18:30"
    assert again.json()["meeting_label"]
    history = client.get("/api/pick/history").json()
    assert history["items"] == []
    assert history["timezone"] == "UTC"


def test_replacing_pick_archives_previous_with_meeting(client):
    register(client, "ada")
    client.put("/api/pick", json={**CIRCE, "meeting_at": "2026-08-22T19:00"})
    client.put("/api/pick", json=ACHILLES)
    current = client.get("/api/pick").json()["pick"]
    assert current["book"]["title"] == "The Song of Achilles"
    assert current["meeting_at"] is None

    history = client.get("/api/pick/history").json()["items"]
    assert len(history) == 1
    assert history[0]["book"]["title"] == "Circe"
    assert history[0]["ended_at"] is not None
    assert history[0]["meeting_local"] == "2026-08-22T19:00"
    assert history[0]["readers"] == []
    assert history[0]["reading"] == []
    assert history[0]["finished"] == []


def test_reading_vs_finished_and_clear(client):
    register(client, "ada")
    client.post("/api/shelf", json={**CIRCE, "status": "currently_reading"})
    client.put("/api/pick", json=CIRCE)

    pick = client.get("/api/pick").json()["pick"]
    assert pick["on_shelf"] == "currently_reading"
    assert pick["reading"] == ["ada"]
    assert pick["finished"] == []

    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    client.post("/api/shelf", json={**CIRCE, "status": "finished"})

    pick = client.get("/api/pick").json()["pick"]
    assert pick["on_shelf"] == "finished"
    assert pick["reading"] == ["ada"]
    assert pick["finished"] == ["grace"]

    cleared = client.delete("/api/pick")
    assert cleared.status_code == 200
    assert cleared.json()["pick"] is None
    assert client.get("/api/pick").json()["pick"] is None
    history = client.get("/api/pick/history").json()["items"]
    assert len(history) == 1
    assert history[0]["book"]["title"] == "Circe"


def test_meeting_uses_club_timezone(client, monkeypatch):
    monkeypatch.setenv("BOOKCLUB_TZ", "America/New_York")
    from app.config import get_settings

    get_settings.cache_clear()
    register(client, "ada")
    assert client.get("/api/config").json()["timezone"] == "America/New_York"

    created = client.put("/api/pick", json={**CIRCE, "meeting_at": "2026-08-22T19:00"})
    assert created.status_code == 200
    assert created.json()["meeting_local"] == "2026-08-22T19:00"
    assert created.json()["meeting_at"].startswith("2026-08-22T23:00:00")
    assert created.json()["meeting_label"]
    assert client.get("/api/pick").json()["timezone"] == "America/New_York"


def test_invalid_timezone_falls_back_to_utc(client, monkeypatch):
    monkeypatch.setenv("BOOKCLUB_TZ", "Not/AZone")
    from app.config import get_settings

    get_settings.cache_clear()
    register(client, "ada")
    assert client.get("/api/config").json()["timezone"] == "UTC"
    created = client.put("/api/pick", json={**CIRCE, "meeting_at": "2026-08-22T19:00"})
    assert created.json()["meeting_local"] == "2026-08-22T19:00"
    assert created.json()["meeting_at"].startswith("2026-08-22T19:00:00")


def test_invalid_pick_payload(client):
    register(client, "ada")
    missing_title = client.put(
        "/api/pick",
        json={"ol_work_key": "/works/OL1W", "title": "   "},
    )
    assert missing_title.status_code == 400
    bad_key = client.put(
        "/api/pick",
        json={"ol_work_key": "OL1W", "title": "Circe"},
    )
    assert bad_key.status_code == 400
    long_note = client.put(
        "/api/pick",
        json={**CIRCE, "note": "x" * 281},
    )
    assert long_note.status_code == 400
    bad_meeting = client.put(
        "/api/pick",
        json={**CIRCE, "meeting_at": "next Tuesday"},
    )
    assert bad_meeting.status_code == 400
