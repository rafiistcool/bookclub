"""Club/shelf/pick/diary/detail-for-known-book paths must not contact Open Library."""

from app import openlibrary
from app.routers import books as books_router
from tests.conftest import register

CIRCE = {
    "ol_work_key": "/works/OL1W",
    "title": "Circe",
    "authors": "Madeline Miller",
    "cover_id": 123,
    "year": 2018,
}


class _Boom:
    def __init__(self, *args, **kwargs):
        raise AssertionError("Open Library should not be contacted")


def test_known_book_reads_never_touch_open_library(client, monkeypatch):
    register(client, "ada")
    assert client.post("/api/shelf", json={**CIRCE, "status": "currently_reading", "progress": 20}).status_code == 201
    assert client.put("/api/pick", json=CIRCE).status_code == 200
    diary = client.post(
        "/api/books/works/OL1W/posts",
        json={"body": "Halfway through.", "spoiler_upto": 40},
    )
    assert diary.status_code == 201, diary.text
    assert client.post("/api/quotes", json={**CIRCE, "body": "But in a story"}).status_code == 201
    assert client.post("/api/vote/nominations", json=CIRCE).status_code in {201, 400}

    monkeypatch.setattr(books_router.httpx, "AsyncClient", _Boom)
    monkeypatch.setattr(openlibrary.httpx, "AsyncClient", _Boom)

    assert client.get("/api/shelf").status_code == 200
    assert client.get("/api/pick").status_code == 200
    assert client.get("/api/pick/history").status_code == 200
    assert client.get("/api/overlap").status_code == 200
    assert client.get("/api/members").status_code == 200
    assert client.get("/api/vote").status_code == 200
    assert client.get("/api/vote/suggestions").status_code == 200
    assert client.get("/api/diary").status_code == 200
    assert client.get("/api/activity").status_code == 200
    assert client.get("/api/stats").status_code == 200
    assert client.get("/api/quotes").status_code == 200
    assert client.get("/api/books/works/OL1W").status_code == 200
    assert client.get("/api/books/work/OL1W").status_code == 200
    assert client.get("/api/books/works/OL1W/posts").status_code == 200


def test_shelving_persists_search_metadata_for_later_local_reads(client, monkeypatch):
    register(client, "ada")
    created = client.post(
        "/api/shelf",
        json={
            **CIRCE,
            "status": "want_to_read",
        },
    )
    assert created.status_code == 201
    book = created.json()["book"]
    assert book["ol_work_key"] == "/works/OL1W"
    assert book["title"] == "Circe"
    assert book["authors"] == "Madeline Miller"
    assert book["cover_id"] == 123
    assert book["year"] == 2018
    assert book["cover_url"].endswith("/123-L.jpg")

    monkeypatch.setattr(books_router.httpx, "AsyncClient", _Boom)
    monkeypatch.setattr(openlibrary.httpx, "AsyncClient", _Boom)

    detail = client.get("/api/books/works/OL1W").json()
    assert detail["title"] == "Circe"
    assert detail["authors"] == "Madeline Miller"
    assert detail["cover_id"] == 123
    assert detail["year"] == 2018
    assert detail["on_shelf"] == "want_to_read"
