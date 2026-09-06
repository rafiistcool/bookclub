from datetime import datetime, timedelta, timezone

from sqlmodel import Session, select

from app.models import Book, NextUpVote
from app.openlibrary import subjects_to_json
from tests.conftest import register

CIRCE = {"ol_work_key": "/works/OL1W", "title": "Circe", "authors": "Madeline Miller", "cover_id": 1, "year": 2018}
ACHILLES = {"ol_work_key": "/works/OL2W", "title": "The Song of Achilles", "authors": "Madeline Miller", "cover_id": 2, "year": 2011}
DUNE = {"ol_work_key": "/works/OL3W", "title": "Dune", "authors": "Frank Herbert", "cover_id": 3, "year": 1965}
PIRANESI = {"ol_work_key": "/works/OL4W", "title": "Piranesi", "authors": "Susanna Clarke", "cover_id": 4, "year": 2020}


def _add_member(client, name):
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, name, invite=invite)


def _switch(client, name):
    client.post("/api/auth/logout")
    client.post("/api/auth/login", json={"username": name, "password": "password1"})


def _expire_deadline(client):
    engine = client.app.state.engine
    with Session(engine) as session:
        vote = session.exec(select(NextUpVote).where(NextUpVote.ended_at.is_(None))).one()
        vote.closes_at = datetime.now(timezone.utc) - timedelta(minutes=1)
        session.add(vote)
        session.commit()


def test_deadline_and_turnout_reporting(client):
    register(client, "ada")
    assert client.patch("/api/vote", json={"closes_at": "2030-01-01T18:00"}).status_code == 404

    client.post("/api/vote/nominations", json=CIRCE)
    vote = client.get("/api/vote").json()
    assert vote["closes_at"] is None
    assert vote["member_count"] == 1
    assert vote["not_voted"] == ["ada"]
    assert vote["voted_count"] == 0
    assert vote["leader_id"] is None

    past = client.patch("/api/vote", json={"closes_at": "2000-01-01T18:00"})
    assert past.status_code == 400
    bad = client.patch("/api/vote", json={"closes_at": "not a date"})
    assert bad.status_code == 400

    set_deadline = client.patch("/api/vote", json={"closes_at": "2030-01-01T18:00"})
    assert set_deadline.status_code == 200
    assert set_deadline.json()["closes_local"] == "2030-01-01T18:00"
    assert set_deadline.json()["closes_label"]

    _add_member(client, "grace")
    nomination_id = client.get("/api/vote").json()["nominations"][0]["id"]
    client.post("/api/vote/cast", json={"nomination_id": nomination_id})
    vote = client.get("/api/vote").json()
    assert vote["member_count"] == 2
    assert vote["voted_count"] == 1
    assert vote["not_voted"] == ["ada"]
    assert vote["leader_id"] == nomination_id

    cleared = client.patch("/api/vote", json={"closes_at": None}).json()
    assert cleared["closes_at"] is None


def test_deadline_auto_applies_the_leader(client):
    register(client, "ada")
    client.post("/api/vote/nominations", json=CIRCE)
    _add_member(client, "grace")
    client.post("/api/vote/nominations", json=ACHILLES)
    nominations = client.get("/api/vote").json()["nominations"]
    achilles_id = next(n["id"] for n in nominations if n["book"]["title"] == "The Song of Achilles")
    client.post("/api/vote/cast", json={"nomination_id": achilles_id})
    client.patch("/api/vote", json={"closes_at": "2030-01-01T18:00"})

    _expire_deadline(client)

    after = client.get("/api/vote").json()
    assert after["vote_id"] is None
    assert after["nominations"] == []
    pick = client.get("/api/pick").json()["pick"]
    assert pick["book"]["title"] == "The Song of Achilles"
    assert pick["set_by"] == "grace"  # nominator of the winner
    items = client.get("/api/activity").json()["items"]
    assert [row["kind"] for row in items[:2]] == ["vote_closed", "pick_set"]
    assert items[0]["payload"] == {"votes": 1, "auto": True}
    assert items[0]["actor"] == "grace"


def test_deadline_with_no_ballots_reopens_without_deadline(client):
    register(client, "ada")
    client.post("/api/vote/nominations", json=CIRCE)
    client.patch("/api/vote", json={"closes_at": "2030-01-01T18:00"})
    _expire_deadline(client)
    after = client.get("/api/vote").json()
    assert after["vote_id"] is not None
    assert after["closes_at"] is None
    assert len(after["nominations"]) == 1
    assert client.get("/api/pick").json()["pick"] is None


def test_tie_goes_to_earliest_nomination(client):
    register(client, "ada")
    client.post("/api/vote/nominations", json=CIRCE)
    _add_member(client, "grace")
    client.post("/api/vote/nominations", json=ACHILLES)
    nominations = client.get("/api/vote").json()["nominations"]
    circe_id = next(n["id"] for n in nominations if n["book"]["title"] == "Circe")
    achilles_id = next(n["id"] for n in nominations if n["book"]["title"] == "The Song of Achilles")
    client.post("/api/vote/cast", json={"nomination_id": achilles_id})
    _switch(client, "ada")
    client.post("/api/vote/cast", json={"nomination_id": circe_id})
    assert client.get("/api/vote").json()["leader_id"] == circe_id
    client.patch("/api/vote", json={"closes_at": "2030-01-01T18:00"})
    _expire_deadline(client)
    client.get("/api/vote")
    assert client.get("/api/pick").json()["pick"]["book"]["title"] == "Circe"


def test_suggestions_rank_shared_tbr_and_subject_affinity(client):
    register(client, "ada")
    assert client.get("/api/vote/suggestions").json() == {"items": []}

    # A past pick with subjects the club evidently likes.
    client.put("/api/pick", json=DUNE)
    engine = client.app.state.engine
    with Session(engine) as session:
        dune = session.exec(select(Book).where(Book.ol_work_key == "/works/OL3W")).one()
        dune.subjects = subjects_to_json(["Science fiction", "Politics"])
        session.add(dune)
        session.commit()
    client.delete("/api/pick")

    client.post("/api/shelf", json={**CIRCE, "status": "want_to_read"})
    client.post("/api/shelf", json={**PIRANESI, "status": "want_to_read"})
    client.post("/api/shelf", json={**DUNE, "status": "want_to_read"})  # past pick: excluded
    _add_member(client, "grace")
    client.post("/api/shelf", json={**CIRCE, "status": "want_to_read"})
    client.post("/api/shelf", json={**ACHILLES, "status": "want_to_read"})
    with Session(engine) as session:
        achilles = session.exec(select(Book).where(Book.ol_work_key == "/works/OL2W")).one()
        achilles.subjects = subjects_to_json(["Greek mythology", "Science fiction"])
        achilles.ol_rating = 4.4
        session.add(achilles)
        session.commit()

    items = client.get("/api/vote/suggestions").json()["items"]
    titles = [row["book"]["title"] for row in items]
    assert titles[0] == "Circe"  # on two shelves
    assert "Dune" not in titles
    circe = items[0]
    assert circe["score"] == 6
    assert circe["reasons"] == ["On 2 shelves: ada, grace"]
    achilles_row = next(row for row in items if row["book"]["title"] == "The Song of Achilles")
    assert achilles_row["score"] == 3  # 1 want + 1 subject + 1 rating
    assert "Like past picks: Science fiction" in achilles_row["reasons"]
    assert "Open Library rating 4.4" in achilles_row["reasons"]

    # Nominating removes a book from the suggestions.
    client.post("/api/vote/nominations", json=CIRCE)
    titles = [row["book"]["title"] for row in client.get("/api/vote/suggestions").json()["items"]]
    assert "Circe" not in titles
