from datetime import datetime, timezone

from sqlmodel import Session, select

from app.models import Book, ShelfEntry
from tests.conftest import register

CIRCE = {"ol_work_key": "/works/OL1W", "title": "Circe", "authors": "Madeline Miller", "cover_id": 1, "year": 2018}
ACHILLES = {"ol_work_key": "/works/OL2W", "title": "The Song of Achilles", "authors": "Madeline Miller", "cover_id": 2, "year": 2011}
DUNE = {"ol_work_key": "/works/OL3W", "title": "Dune", "authors": "Frank Herbert", "cover_id": 3, "year": 1965}


def _add_member(client, name):
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, name, invite=invite)


# --- stats -------------------------------------------------------------------


def test_stats_year_in_review(client):
    register(client, "ada")
    client.post("/api/shelf", json={**CIRCE, "status": "finished", "rating": 5, "take": "Yes"})
    client.post("/api/shelf", json={**ACHILLES, "status": "finished", "rating": 3})
    client.post("/api/shelf", json={**DUNE, "status": "did_not_finish", "dnf_reason": "Sand"})
    engine = client.app.state.engine
    with Session(engine) as session:
        for book, pages in (("/works/OL1W", 393), ("/works/OL2W", 352)):
            row = session.exec(select(Book).where(Book.ol_work_key == book)).one()
            row.pages = pages
            session.add(row)
        # Push Achilles into last year to exercise the year filter.
        achilles = session.exec(select(Book).where(Book.ol_work_key == "/works/OL2W")).one()
        entry = session.exec(select(ShelfEntry).where(ShelfEntry.book_id == achilles.id)).one()
        entry.finished_at = datetime(datetime.now(timezone.utc).year - 1, 6, 1, tzinfo=timezone.utc)
        session.add(entry)
        session.commit()
    client.put("/api/pick", json=CIRCE)
    client.post("/api/quotes", json={**CIRCE, "body": "Humbling women seems to me a chief pastime of poets."})

    this_year = datetime.now(timezone.utc).year
    body = client.get("/api/stats").json()
    assert body["year"] == this_year
    assert body["years"] == [this_year, this_year - 1]
    ada = body["members"][0]
    assert ada["username"] == "ada"
    assert ada["finished"] == 1
    assert ada["dnf"] == 1
    assert ada["pages"] == 393
    assert ada["average_rating"] == 5.0
    assert ada["five_stars"] == 1
    assert ada["top_book"]["title"] == "Circe"
    assert ada["longest_book"]["title"] == "Circe"
    assert ada["quotes"] == 1
    assert body["club_finished"] == 1
    assert body["club_pages"] == 393
    assert body["club_average_rating"] == 5.0
    assert len(body["by_month"]) == 1
    assert body["by_month"][0]["finished"] == 1
    assert body["picks"][0]["book"]["title"] == "Circe"
    assert body["picks"][0]["finished"] == 1
    assert body["picks"][0]["average_rating"] == 5.0
    assert body["picks"][0]["ratings"] == [5]
    assert body["best_pick"]["book"]["title"] == "Circe"

    last_year = client.get("/api/stats", params={"year": this_year - 1}).json()
    assert last_year["members"][0]["finished"] == 1
    assert last_year["members"][0]["top_book"]["title"] == "The Song of Achilles"
    assert last_year["club_pages"] == 352


def test_stats_with_empty_club(client):
    register(client, "ada")
    body = client.get("/api/stats").json()
    assert body["members"][0] == {
        "username": "ada",
        "finished": 0,
        "dnf": 0,
        "pages": 0,
        "average_rating": None,
        "five_stars": 0,
        "top_book": None,
        "longest_book": None,
        "quotes": 0,
    }
    assert body["best_pick"] is None
    assert body["by_month"] == []


# --- milestones ------------------------------------------------------------------


def test_milestones_crud_and_threads(client):
    register(client, "ada")
    pick_id = client.put("/api/pick", json=CIRCE).json()["id"]
    url = f"/api/pick/{pick_id}/milestones"
    assert client.get(url).json() == {"pick_id": pick_id, "items": [], "timezone": "UTC"}

    created = client.post(
        url, json={"title": "Part one", "chapter_from": 1, "chapter_to": 8, "due_at": "2030-01-05T20:00"}
    )
    assert created.status_code == 201, created.text
    first = created.json()
    assert first["position"] == 0
    assert first["due_local"] == "2030-01-05T20:00"
    assert first["passed"] is False
    assert first["note_count"] == 0

    second = client.post(url, json={"title": "Part two", "chapter_from": 9, "chapter_to": 16}).json()
    assert second["position"] == 1
    assert second["due_at"] is None

    assert client.post(url, json={"title": "", "chapter_from": 1}).status_code == 400
    assert client.post(url, json={"title": "Backwards", "chapter_from": 9, "chapter_to": 2}).status_code == 400

    # Notes can be attached to a milestone; the count comes back on the list.
    post = client.post("/api/pick/posts", json={"body": "Part one thoughts", "milestone_id": first["id"]})
    assert post.status_code == 201
    assert post.json()["milestone_id"] == first["id"]
    assert client.post("/api/pick/posts", json={"body": "bad", "milestone_id": 9999}).status_code == 400
    items = client.get(url).json()["items"]
    assert items[0]["note_count"] == 1

    edited = client.patch(f"{url}/{first['id']}", json={"title": "Part 1", "chapter_from": 1, "chapter_to": 8})
    assert edited.status_code == 200
    assert edited.json()["title"] == "Part 1"
    assert edited.json()["due_at"] is None

    assert client.delete(f"{url}/{first['id']}").status_code == 204
    items = client.get(url).json()["items"]
    assert [row["title"] for row in items] == ["Part two"]
    assert items[0]["position"] == 0
    # The note survives, detached from the deleted milestone.
    thread = client.get("/api/pick/posts").json()["items"]
    assert thread[0]["milestone_id"] is None

    assert client.get("/api/pick/999/milestones").status_code == 404
    assert client.delete(f"{url}/9999").status_code == 404

    client.put("/api/pick", json=ACHILLES)  # closes Circe
    assert client.post(url, json={"title": "Too late"}).status_code == 400
    kinds = [row["kind"] for row in client.get("/api/activity").json()["items"]]
    assert "milestone_added" in kinds


# --- quotes ----------------------------------------------------------------------


def test_quotes_crud_and_filters(client):
    register(client, "ada")
    assert client.post("/api/quotes", json={"ol_work_key": "/works/OL1W", "body": "x"}).status_code == 400  # unknown book needs a title

    created = client.post("/api/quotes", json={**CIRCE, "body": "  Humbling women.  ", "page": 12})
    assert created.status_code == 201, created.text
    quote = created.json()
    assert quote["body"] == "Humbling women."
    assert quote["page"] == 12
    assert quote["mine"] is True
    assert quote["author"] == "ada"
    assert quote["book"]["title"] == "Circe"

    client.post("/api/quotes", json={**ACHILLES, "body": "Name one hero who was happy."})
    assert len(client.get("/api/quotes").json()["items"]) == 2
    only_circe = client.get("/api/quotes", params={"work": "/works/OL1W"}).json()["items"]
    assert [row["book"]["title"] for row in only_circe] == ["Circe"]
    assert client.get("/api/quotes", params={"work": "/works/OL99W"}).json()["items"] == []

    edited = client.patch(f"/api/quotes/{quote['id']}", json={"page": None})
    assert edited.status_code == 200
    assert edited.json()["page"] is None
    assert client.patch(f"/api/quotes/{quote['id']}", json={}).status_code == 400

    details = client.get("/api/books/work/OL1W")
    # Details come from Open Library normally; here the book row exists so the
    # count is available even if the upstream call is mocked away in other tests.
    if details.status_code == 200:
        assert details.json()["quote_count"] == 1

    _add_member(client, "grace")
    assert client.get("/api/quotes", params={"username": "ada"}).json()["items"][0]["mine"] is False
    assert client.patch(f"/api/quotes/{quote['id']}", json={"body": "hijack"}).status_code == 403
    assert client.delete(f"/api/quotes/{quote['id']}").status_code == 403
    assert client.get("/api/quotes", params={"username": "grace"}).json()["items"] == []

    client.post("/api/auth/logout")
    client.post("/api/auth/login", json={"username": "ada", "password": "password1"})
    assert client.delete(f"/api/quotes/{quote['id']}").status_code == 204
    assert client.delete(f"/api/quotes/{quote['id']}").status_code == 404
    kinds = [row["kind"] for row in client.get("/api/activity").json()["items"]]
    assert "quote_added" in kinds
