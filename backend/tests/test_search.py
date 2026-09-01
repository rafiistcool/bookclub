from app.routers import books as books_router
from tests.conftest import register


class _FakeResponse:
    status_code = 200

    def raise_for_status(self) -> None:
        return None

    def json(self) -> dict:
        return {
            "numFound": 50,
            "docs": [
                {
                    "key": "/works/OL1W",
                    "title": "Circe",
                    "author_name": ["Madeline Miller"],
                    "cover_i": 123,
                    "first_publish_year": 2018,
                },
                {"key": "/books/OL9M", "title": "An edition, skip"},
                {
                    "key": "/works/OL1W",
                    "title": "Circe duplicate work",
                },
            ],
        }


class _FakeClient:
    calls = 0
    last_params = None
    last_headers = None

    def __init__(self, *args, **kwargs):
        pass

    async def __aenter__(self):
        return self

    async def __aexit__(self, *args):
        return None

    async def get(self, url, params=None, headers=None):
        _FakeClient.calls += 1
        _FakeClient.last_params = params
        _FakeClient.last_headers = headers
        return _FakeResponse()


def test_search_empty_q_browses(client, monkeypatch):
    register(client, "ada")
    _FakeClient.calls = 0
    monkeypatch.setattr(books_router.httpx, "AsyncClient", _FakeClient)

    blank = client.get("/api/books/search", params={"q": "   "})
    assert blank.status_code == 200
    missing = client.get("/api/books/search")
    assert missing.status_code == 200
    body = missing.json()
    assert set(body) == {"items", "page", "has_more"}
    assert body["page"] == 1
    assert body["has_more"] is True
    # A bare "*" is rejected by Open Library (422); the browse must use "*:*".
    assert _FakeClient.last_params["q"] == "*:*"
    assert _FakeClient.last_params["q"] != "*"
    assert _FakeClient.last_params["lang"] == "en"
    assert "language" not in _FakeClient.last_params
    assert _FakeClient.last_params["sort"] == "readinglog"
    assert "Bookclub/1.0" in (_FakeClient.last_headers or {}).get("User-Agent", "")


def test_search_maps_and_caches(client, monkeypatch):
    register(client, "ada")
    _FakeClient.calls = 0
    monkeypatch.setattr(books_router.httpx, "AsyncClient", _FakeClient)
    first = client.get("/api/books/search", params={"q": "circe"})
    assert first.status_code == 200
    body = first.json()
    assert set(body) == {"items", "page", "has_more"}
    assert body["page"] == 1
    assert body["has_more"] is True
    hits = body["items"]
    assert len(hits) == 1
    assert hits[0]["ol_work_key"] == "/works/OL1W"
    assert hits[0]["title"] == "Circe"
    assert hits[0]["on_shelf"] is None
    assert _FakeClient.calls == 1
    assert _FakeClient.last_params["q"] == "circe"
    assert "sort" not in _FakeClient.last_params

    second = client.get("/api/books/search", params={"q": "circe"})
    assert second.status_code == 200
    assert _FakeClient.calls == 1

    client.post(
        "/api/shelf",
        json={
            "ol_work_key": "/works/OL1W",
            "title": "Circe",
            "authors": "Madeline Miller",
            "cover_id": 123,
            "year": 2018,
            "status": "currently_reading",
        },
    )
    marked = client.get("/api/books/search", params={"q": "circe"}).json()
    assert marked["items"][0]["on_shelf"] == "currently_reading"
    assert marked["items"][0]["club_pick"] is False

    client.put(
        "/api/pick",
        json={
            "ol_work_key": "/works/OL1W",
            "title": "Circe",
            "authors": "Madeline Miller",
            "cover_id": 123,
            "year": 2018,
        },
    )
    picked = client.get("/api/books/search", params={"q": "circe"}).json()
    assert picked["items"][0]["club_pick"] is True


def test_search_forwards_page_subject_sort(client, monkeypatch):
    register(client, "ada")
    _FakeClient.calls = 0
    monkeypatch.setattr(books_router.httpx, "AsyncClient", _FakeClient)
    response = client.get(
        "/api/books/search",
        params={
            "q": "circe",
            "subject": "fantasy",
            "sort": "new",
            "page": 2,
            "limit": 12,
        },
    )
    assert response.status_code == 200
    assert _FakeClient.last_params["q"] == "circe subject_key:fantasy"
    assert "subject" not in _FakeClient.last_params
    assert _FakeClient.last_params["sort"] == "new"
    assert _FakeClient.last_params["page"] == 2
    assert _FakeClient.last_params["limit"] == 12
    assert "language" not in _FakeClient.last_params
    assert "lang" not in _FakeClient.last_params

    browse = client.get("/api/books/search", params={"subject": "science_fiction"})
    assert browse.status_code == 200
    assert _FakeClient.last_params["q"] == "subject_key:science_fiction"
    assert _FakeClient.last_params["lang"] == "en"
    assert "subject" not in _FakeClient.last_params


def test_search_cache_keyed_by_page(client, monkeypatch):
    register(client, "ada")
    _FakeClient.calls = 0
    monkeypatch.setattr(books_router.httpx, "AsyncClient", _FakeClient)

    first = client.get("/api/books/search")
    assert first.status_code == 200
    assert _FakeClient.calls == 1

    again = client.get("/api/books/search")
    assert again.status_code == 200
    assert _FakeClient.calls == 1

    page_two = client.get("/api/books/search", params={"page": 2})
    assert page_two.status_code == 200
    assert _FakeClient.calls == 2
    assert _FakeClient.last_params["page"] == 2
