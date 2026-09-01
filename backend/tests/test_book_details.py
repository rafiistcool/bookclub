import pytest

from app import openlibrary
from tests.conftest import register

CIRCE = {
    "ol_work_key": "/works/OL1W",
    "title": "Circe",
    "authors": "Madeline Miller",
    "cover_id": 123,
    "year": 2018,
}

WORK_JSON = {
    "title": "Circe",
    "description": {"value": "A witch on an island.\n\n----------\n[Source](https://x)"},
    "subjects": ["Greek mythology", "Fiction", "Witches", "Fiction, fantasy, general", "x" * 50],
    "covers": [123],
}
SEARCH_JSON = {
    "numFound": 1,
    "docs": [
        {
            "key": "/works/OL1W",
            "title": "Circe",
            "author_name": ["Madeline Miller"],
            "cover_i": 123,
            "first_publish_year": 2018,
            "number_of_pages_median": 393,
            "ratings_average": 4.27,
            "ratings_count": 812,
            "subject": ["Greek mythology"],
        }
    ],
}


class _Resp:
    def __init__(self, payload):
        self._payload = payload
        self.status_code = 200

    def raise_for_status(self):
        return None

    def json(self):
        return self._payload


class _Client:
    calls: list[str] = []

    def __init__(self, *a, **k):
        pass

    async def __aenter__(self):
        return self

    async def __aexit__(self, *a):
        return None

    async def get(self, url, params=None, headers=None):
        _Client.calls.append(url)
        if url.endswith("/search.json"):
            if params and str(params.get("q", "")).startswith("isbn:"):
                if "0000000000" in params["q"]:
                    return _Resp({"numFound": 0, "docs": []})
                return _Resp(SEARCH_JSON)
            return _Resp(SEARCH_JSON)
        return _Resp(WORK_JSON)


@pytest.fixture
def ol(monkeypatch):
    _Client.calls = []
    monkeypatch.setattr(openlibrary.httpx, "AsyncClient", _Client)
    return _Client


def test_work_details_for_unknown_book(client, ol):
    register(client, "ada")
    response = client.get("/api/books/work/OL1W")
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["title"] == "Circe"
    assert body["authors"] == "Madeline Miller"
    assert body["pages"] == 393
    assert body["ol_rating"] == 4.27
    assert body["ol_rating_count"] == 812
    assert body["description"] == "A witch on an island."
    # Subjects: de-duplicated, long/comma-laden entries dropped.
    assert body["subjects"] == ["Greek mythology", "Fiction", "Witches"]
    assert body["on_shelf"] is None
    assert body["members"] == []
    assert body["club_pick"] is False
    assert body["cover_url"].endswith("/123-L.jpg")
    assert len(ol.calls) == 2

    # Second call within TTL hits the in-memory cache.
    client.get("/api/books/work/OL1W")
    assert len(ol.calls) == 2


def test_work_details_persist_on_known_book_and_list_members(client, ol):
    register(client, "ada")
    client.post("/api/shelf", json={**CIRCE, "status": "finished", "rating": 5, "take": "Loved it"})
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    client.post("/api/shelf", json={**CIRCE, "status": "currently_reading", "progress": 40})
    client.put("/api/pick", json=CIRCE)

    body = client.get("/api/books/work/OL1W").json()
    assert body["club_pick"] is True
    assert body["on_shelf"] == "currently_reading"
    assert body["shelf_id"] is not None
    assert [m["username"] for m in body["members"]] == ["grace", "ada"]  # viewer first
    assert body["members"][1] == {
        "username": "ada",
        "status": "finished",
        "rating": 5,
        "take": "Loved it",
        "progress": None,
        "finished_at": body["members"][1]["finished_at"],
    }
    assert body["members"][0]["progress"] == 40

    # Details were stored on the Book row, so the shelf now carries pages.
    shelf = client.get("/api/shelf").json()["items"][0]
    assert shelf["book"]["pages"] == 393

    # Stored details are served without another upstream call.
    openlibrary.clear_details_cache()
    calls_before = len(ol.calls)
    client.get("/api/books/work/OL1W")
    assert len(ol.calls) == calls_before


def test_work_details_rejects_bad_ids_and_upstream_failure(client, monkeypatch):
    register(client, "ada")
    assert client.get("/api/books/work/not-a-key").status_code == 400

    class Boom(_Client):
        async def get(self, url, params=None, headers=None):
            raise openlibrary.httpx.ConnectError("down")

    monkeypatch.setattr(openlibrary.httpx, "AsyncClient", Boom)
    assert client.get("/api/books/work/OL9W").status_code == 502


def test_isbn_lookup(client, ol):
    register(client, "ada")
    assert client.get("/api/books/isbn/12345").status_code == 400
    missing = client.get("/api/books/isbn/0000000000")
    assert missing.status_code == 404

    hit = client.get("/api/books/isbn/978-0-316-55634-7")
    assert hit.status_code == 200, hit.text
    body = hit.json()
    assert body["isbn"] == "9780316556347"
    assert body["ol_work_key"] == "/works/OL1W"
    assert body["title"] == "Circe"
    assert body["pages"] == 393
    assert body["on_shelf"] is None

    client.post("/api/shelf", json={**CIRCE, "status": "want_to_read"})
    again = client.get("/api/books/isbn/9780316556347").json()
    assert again["on_shelf"] == "want_to_read"
    assert again["shelf_id"] is not None


def test_isbn10_with_x_check_digit_is_accepted(client, ol):
    register(client, "ada")
    assert client.get("/api/books/isbn/080442957X").status_code == 200
