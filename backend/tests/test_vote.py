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

WOLF = {
    "ol_work_key": "/works/OL3W",
    "title": "The Wolf Den",
    "authors": "Elodie Harper",
    "cover_id": 789,
    "year": 2021,
}


def _book(n: int) -> dict:
    return {
        "ol_work_key": f"/works/OL{n}W",
        "title": f"Book {n}",
        "authors": "Author",
        "cover_id": n,
        "year": 2000 + n,
    }


def test_vote_requires_auth(client):
    assert client.get("/api/vote").status_code == 401
    assert client.post("/api/vote/nominations", json=CIRCE).status_code == 401
    assert client.post("/api/vote/cast", json={"nomination_id": 1}).status_code == 401
    assert client.post("/api/vote/apply", json={"nomination_id": 1}).status_code == 401


def test_empty_vote_then_nominate_and_count(client):
    register(client, "ada")
    empty = client.get("/api/vote")
    assert empty.status_code == 200
    body = empty.json()
    assert body["vote_id"] is None
    assert body["nominations"] == []
    assert body["my_vote_id"] is None
    assert body["can_nominate"] is True
    assert body["nomination_limit"] == 6

    created = client.post("/api/vote/nominations", json=CIRCE)
    assert created.status_code == 201
    vote = created.json()
    assert vote["vote_id"]
    assert vote["can_nominate"] is True
    assert len(vote["nominations"]) == 1
    row = vote["nominations"][0]
    assert row["book"]["title"] == "Circe"
    assert row["nominated_by"] == "ada"
    assert row["votes"] == 0
    assert row["mine"] is False

    pick = client.get("/api/pick").json()["pick"]
    assert pick is None


def test_members_can_vote_and_change_vote(client):
    register(client, "ada")
    circe_id = client.post("/api/vote/nominations", json=CIRCE).json()["nominations"][0]["id"]
    achilles_id = client.post("/api/vote/nominations", json=ACHILLES).json()["nominations"][1]["id"]

    first = client.post("/api/vote/cast", json={"nomination_id": circe_id})
    assert first.status_code == 200
    assert first.json()["my_vote_id"] == circe_id
    by_title = {row["book"]["title"]: row for row in first.json()["nominations"]}
    assert by_title["Circe"]["votes"] == 1
    assert by_title["Circe"]["voters"] == ["ada"]
    assert by_title["Circe"]["mine"] is True
    assert by_title["The Song of Achilles"]["votes"] == 0

    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    second = client.post("/api/vote/cast", json={"nomination_id": circe_id})
    assert second.status_code == 200
    by_title = {row["book"]["title"]: row for row in second.json()["nominations"]}
    assert by_title["Circe"]["votes"] == 2
    assert by_title["Circe"]["voters"] == ["ada", "grace"]

    switched = client.post("/api/vote/cast", json={"nomination_id": achilles_id})
    by_title = {row["book"]["title"]: row for row in switched.json()["nominations"]}
    assert by_title["Circe"]["votes"] == 1
    assert by_title["The Song of Achilles"]["votes"] == 1
    assert switched.json()["my_vote_id"] == achilles_id
    assert client.get("/api/pick").json()["pick"] is None


def test_cannot_nominate_current_pick_or_duplicate(client):
    register(client, "ada")
    client.put("/api/pick", json=CIRCE)
    blocked = client.post("/api/vote/nominations", json=CIRCE)
    assert blocked.status_code == 400

    client.post("/api/vote/nominations", json=ACHILLES)
    again = client.post("/api/vote/nominations", json=ACHILLES)
    assert again.status_code == 400


def test_nomination_limit(client):
    register(client, "ada")
    for n in range(1, 7):
        response = client.post("/api/vote/nominations", json=_book(n))
        assert response.status_code == 201
    extra = client.post("/api/vote/nominations", json=_book(7))
    assert extra.status_code == 400
    assert extra.json()["detail"] == "This vote already has enough books"


def test_apply_winner_uses_club_pick_flow(client):
    register(client, "ada")
    old = client.put("/api/pick", json=CIRCE).json()
    old_id = old["id"]
    client.post("/api/pick/posts", json={"body": "Circe notes"})

    nom = client.post("/api/vote/nominations", json=ACHILLES).json()["nominations"][0]
    client.post("/api/vote/cast", json={"nomination_id": nom["id"]})
    assert client.get("/api/pick").json()["pick"]["book"]["title"] == "Circe"

    applied = client.post(
        "/api/vote/apply",
        json={"nomination_id": nom["id"], "meeting_at": "2026-09-12T19:00"},
    )
    assert applied.status_code == 200
    pick = applied.json()["pick"]
    assert pick["id"] != old_id
    assert pick["book"]["title"] == "The Song of Achilles"
    assert pick["set_by"] == "ada"
    assert pick["meeting_local"] == "2026-09-12T19:00"
    assert applied.json()["vote"]["vote_id"] is None
    assert applied.json()["vote"]["nominations"] == []

    current = client.get("/api/pick").json()["pick"]
    assert current["id"] == pick["id"]
    history = client.get("/api/pick/history").json()["items"]
    assert history[0]["id"] == old_id
    notes = client.get(f"/api/pick/{old_id}/posts").json()
    assert [row["body"] for row in notes["items"]] == ["Circe notes"]
    assert notes["can_post"] is False

    fresh = client.get("/api/vote").json()
    assert fresh["vote_id"] is None
    again = client.post("/api/vote/nominations", json=WOLF)
    assert again.status_code == 201
    assert again.json()["nominations"][0]["book"]["title"] == "The Wolf Den"


def test_apply_requires_an_open_nomination(client):
    register(client, "ada")
    missing = client.post("/api/vote/apply", json={"nomination_id": 99})
    assert missing.status_code == 404
    nom_id = client.post("/api/vote/nominations", json=CIRCE).json()["nominations"][0]["id"]
    client.post("/api/vote/apply", json={"nomination_id": nom_id})
    closed = client.post("/api/vote/apply", json={"nomination_id": nom_id})
    assert closed.status_code == 404
