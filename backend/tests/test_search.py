import httpx
import pytest

from app.routers import books as books_router
from tests.conftest import register

SEARCH_DOCS = [
    {
        "key": "/works/OL1W",
        "title": "Circe",
        "author_name": ["Madeline Miller"],
        "cover_i": 123,
        "first_publish_year": 2018,
    },
    {"key": "/books/OL9M", "title": "An edition, skip"},
    {"key": "/works/OL1W", "title": "Circe duplicate work"},
]

TRENDING_WORKS = [
    {
        "key": "/works/OL7W",
        "title": "Atomic Habits",
        "author_name": ["James Clear"],
        "cover_i": 999,
        "first_publish_year": 2016,
    },
    {"key": "/works/OL8W", "title": "No cover", "cover_i": -1},
]

# subjects/*.json uses cover_id and authors[].name instead of cover_i/author_name.
SUBJECT_WORKS = [
    {
        "key": "/works/OL138052W",
        "title": "Alice's Adventures in Wonderland",
        "authors": [{"key": "/authors/OL22098A", "name": "Lewis Carroll"}],
        "cover_id": 10527843,
        "first_publish_year": 1865,
    },
    {"key": "/authors/OL1A", "title": "Not a work, skip"},
]


class _FakeResponse:
    def __init__(self, payload, url, status_code=200):
        self._payload = payload
        self._url = url
        self.status_code = status_code

    def raise_for_status(self) -> None:
        if self.status_code < 400:
            return
        request = httpx.Request("GET", self._url)
        raise httpx.HTTPStatusError(
            f"{self.status_code}",
            request=request,
            response=httpx.Response(self.status_code, request=request),
        )

    def json(self):
        return self._payload


class _FakeClient:
    """Routes Open Library URLs to canned payloads and records every call."""

    calls: list[tuple[str, dict | None]] = []
    work_payload: dict = {}
    missing: set[str] = set()

    def __init__(self, *args, **kwargs):
        pass

    async def __aenter__(self):
        return self

    async def __aexit__(self, *args):
        return None

    @classmethod
    def reset(cls) -> None:
        cls.calls = []
        cls.missing = set()
        cls.work_payload = {
            "title": "Circe",
            "description": "A plain string description.",
            "subjects": [
                "Mythology",
                "mythology",
                "Greek literature",
                "",
                "award:hugo_award=1970",
            ],
            "covers": [-1, 555],
            "authors": [{"author": {"key": "/authors/OL2A"}}],
        }

    @classmethod
    def params_for(cls, needle: str) -> list[dict]:
        return [params or {} for url, params in cls.calls if needle in url]

    async def get(self, url, params=None, headers=None):
        _FakeClient.calls.append((url, params))
        assert "Bookclub/1.0" in (headers or {}).get("User-Agent", "")

        for marker in _FakeClient.missing:
            if marker in url:
                return _FakeResponse(None, url, status_code=404)

        if "search.json" in url:
            return _FakeResponse({"numFound": 50, "docs": SEARCH_DOCS}, url)
        if "trending" in url:
            return _FakeResponse({"works": TRENDING_WORKS}, url)
        if "/subjects/" in url:
            return _FakeResponse({"work_count": 40, "works": SUBJECT_WORKS}, url)
        if "/authors/" in url:
            return _FakeResponse({"name": "Madeline Miller"}, url)
        if "/works/" in url:
            return _FakeResponse(_FakeClient.work_payload, url)
        raise AssertionError(f"unexpected url {url}")


@pytest.fixture
def ol(client, monkeypatch):
    register(client, "ada")
    _FakeClient.reset()
    monkeypatch.setattr(books_router.httpx, "AsyncClient", _FakeClient)
    return client


def _shelve(client, work_key="/works/OL1W", status="currently_reading"):
    return client.post(
        "/api/shelf",
        json={
            "ol_work_key": work_key,
            "title": "Circe",
            "authors": "Madeline Miller",
            "cover_id": 123,
            "year": 2018,
            "status": status,
        },
    )


def test_empty_search_serves_trending(ol):
    blank = ol.get("/api/books/search", params={"q": "   "})
    assert blank.status_code == 200
    missing = ol.get("/api/books/search")
    assert missing.status_code == 200

    body = missing.json()
    assert set(body) == {"items", "page", "has_more"}
    assert body["page"] == 1
    assert body["has_more"] is False
    assert [hit["title"] for hit in body["items"]] == ["Atomic Habits", "No cover"]
    # -1 means "no cover" in Open Library payloads.
    assert body["items"][1]["cover_id"] is None
    assert _FakeClient.params_for("search.json") == []
    assert _FakeClient.params_for("trending")


def test_open_library_never_receives_a_too_short_query(ol):
    """Regression: q=* returned 422 "Query too short, must be at least 3 characters"."""
    ol.get("/api/books/search")
    ol.get("/api/books/search", params={"q": ""})
    ol.get("/api/books/search", params={"q": "  "})
    ol.get("/api/books/search", params={"subject": "fantasy"})
    ol.get("/api/books/search", params={"q": "circe"})
    ol.get("/api/books/trending")
    ol.get("/api/books/subjects/fantasy")

    for params in _FakeClient.params_for("search.json"):
        assert params["q"] != "*"
        assert len(params["q"]) >= 3


def test_search_maps_and_caches(ol):
    first = ol.get("/api/books/search", params={"q": "circe"})
    assert first.status_code == 200
    body = first.json()
    assert body["page"] == 1
    assert body["has_more"] is True
    hits = body["items"]
    assert len(hits) == 1
    assert hits[0]["ol_work_key"] == "/works/OL1W"
    assert hits[0]["title"] == "Circe"
    assert hits[0]["on_shelf"] is None
    assert len(_FakeClient.params_for("search.json")) == 1
    assert _FakeClient.params_for("search.json")[0]["q"] == "circe"
    assert "sort" not in _FakeClient.params_for("search.json")[0]

    second = ol.get("/api/books/search", params={"q": "circe"})
    assert second.status_code == 200
    assert len(_FakeClient.params_for("search.json")) == 1

    # Case and extra spaces share the cache key.
    ol.get("/api/books/search", params={"q": "Circe"})
    ol.get("/api/books/search", params={"q": "  CIRCE  "})
    assert len(_FakeClient.params_for("search.json")) == 1

    _shelve(ol)
    marked = ol.get("/api/books/search", params={"q": "circe"}).json()
    assert marked["items"][0]["on_shelf"] == "currently_reading"
    assert marked["items"][0]["club_pick"] is False

    ol.put(
        "/api/pick",
        json={
            "ol_work_key": "/works/OL1W",
            "title": "Circe",
            "authors": "Madeline Miller",
            "cover_id": 123,
            "year": 2018,
        },
    )
    picked = ol.get("/api/books/search", params={"q": "circe"}).json()
    assert picked["items"][0]["club_pick"] is True


def test_search_forwards_page_subject_sort(ol):
    response = ol.get(
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
    sent = _FakeClient.params_for("search.json")[-1]
    assert sent["q"] == "circe subject_key:fantasy"
    assert "subject" not in sent
    assert sent["sort"] == "new"
    assert sent["page"] == 2
    assert sent["limit"] == 12
    assert "lang" not in sent

    browse = ol.get("/api/books/search", params={"subject": "science_fiction"})
    assert browse.status_code == 200
    sent = _FakeClient.params_for("search.json")[-1]
    assert sent["q"] == "subject_key:science_fiction"
    assert sent["lang"] == "en"
    assert sent["sort"] == "readinglog"


def test_search_cache_keyed_by_page(ol):
    assert ol.get("/api/books/search", params={"q": "circe"}).status_code == 200
    assert len(_FakeClient.params_for("search.json")) == 1

    assert ol.get("/api/books/search", params={"q": "circe"}).status_code == 200
    assert len(_FakeClient.params_for("search.json")) == 1

    page_two = ol.get("/api/books/search", params={"q": "circe", "page": 2})
    assert page_two.status_code == 200
    assert len(_FakeClient.params_for("search.json")) == 2
    assert _FakeClient.params_for("search.json")[-1]["page"] == 2


def test_trending_annotates_and_caches(ol):
    _shelve(ol, work_key="/works/OL7W", status="want_to_read")

    body = ol.get("/api/books/trending", params={"limit": 5}).json()
    assert body["items"][0]["on_shelf"] == "want_to_read"
    assert _FakeClient.params_for("trending")[0]["limit"] == 5

    ol.get("/api/books/trending", params={"limit": 5})
    assert len(_FakeClient.params_for("trending")) == 1


def test_subject_maps_its_own_payload_shape(ol):
    body = ol.get("/api/books/subjects/Science Fiction", params={"limit": 10}).json()
    assert len(body["items"]) == 1
    hit = body["items"][0]
    assert hit["title"] == "Alice's Adventures in Wonderland"
    assert hit["authors"] == "Lewis Carroll"
    assert hit["cover_id"] == 10527843
    assert hit["year"] == 1865

    url = [u for u, _ in _FakeClient.calls if "/subjects/" in u][0]
    assert url.endswith("/subjects/science_fiction.json")
    assert body["has_more"] is True

    page_four = ol.get("/api/books/subjects/fantasy", params={"page": 4, "limit": 10})
    assert page_four.json()["has_more"] is False
    assert _FakeClient.params_for("/subjects/")[-1]["offset"] == 30


def test_subject_rejects_invalid_key(ol):
    response = ol.get("/api/books/subjects/%21%21%21")
    assert response.status_code == 400
    assert _FakeClient.params_for("/subjects/") == []


def test_book_detail_resolves_description_subjects_and_authors(ol):
    body = ol.get("/api/books/works/OL1W").json()
    assert body["ol_work_key"] == "/works/OL1W"
    assert body["title"] == "Circe"
    assert body["description"] == "A plain string description."
    # Deduplicated case-insensitively; blanks and machine tags dropped.
    assert body["subjects"] == ["Mythology", "Greek literature"]
    # First positive cover wins; -1 is skipped.
    assert body["cover_id"] == 555
    assert body["authors"] == "Madeline Miller"
    assert body["on_shelf"] is None
    assert body["readers"] == []

    # First view imports the work; later reads are local even after the RAM cache is cleared.
    books_router.clear_search_cache()
    calls = len(_FakeClient.calls)
    again = ol.get("/api/books/works/OL1W").json()
    assert again["description"] == "A plain string description."
    assert again["subjects"] == ["Mythology", "Greek literature"]
    assert len(_FakeClient.calls) == calls


def test_book_detail_accepts_dict_description(ol):
    _FakeClient.work_payload = {
        "title": "Circe",
        "description": {"type": "/type/text", "value": "  A dict description.  "},
    }
    body = ol.get("/api/books/works/OL1W").json()
    assert body["description"] == "A dict description."
    assert body["subjects"] == []
    assert body["cover_id"] is None


def test_book_detail_flattens_markdown_in_description(ol):
    _FakeClient.work_payload = {
        "title": "Circe",
        "description": (
            "[Comment by Kim Stanley Robinson][1] and [a note](http://example.com):\n"
            "\n"
            "> One of my favorite novels.\n"
            "> Truly.\n"
            "\n"
            "\n"
            "\n"
            "The end.\n"
            "\n"
            "[1]: http://example.com/robinson\n"
        ),
    }
    body = ol.get("/api/books/works/OL1W").json()
    assert body["description"] == (
        "Comment by Kim Stanley Robinson and a note:\n"
        "\n"
        "One of my favorite novels.\n"
        "Truly.\n"
        "\n"
        "The end."
    )


def test_book_detail_includes_own_shelf_state_and_other_readers(ol):
    _shelve(ol)
    ol.patch("/api/shelf/1", json={"progress": 40})
    invite = ol.post("/api/invites").json()["code"]
    ol.post("/api/auth/logout")
    register(ol, "grace", invite=invite)
    _shelve(ol, status="finished")
    ol.patch("/api/shelf/2", json={"rating": 5, "take": "Loved it"})

    _FakeClient.calls = []
    body = ol.get("/api/books/works/OL1W").json()
    assert _FakeClient.calls == []
    assert body["on_shelf"] == "finished"
    assert body["rating"] == 5
    assert body["take"] == "Loved it"
    assert [reader["username"] for reader in body["readers"]] == ["ada"]
    assert body["readers"][0]["status"] == "currently_reading"
    assert body["readers"][0]["progress"] == 40
    # Known book: local fields only (search metadata from the shelf add).
    assert body["year"] == 2018
    assert body["cover_id"] == 123
    assert body["title"] == "Circe"
    assert body["description"] == ""


def test_book_detail_rejects_malformed_work_id(ol):
    assert ol.get("/api/books/works/not-a-key").status_code == 404
    assert ol.get("/api/books/works/OL1M").status_code == 404
    assert _FakeClient.calls == []


def test_book_detail_propagates_not_found(ol):
    _FakeClient.missing = {"/works/"}
    response = ol.get("/api/books/works/OL404W")
    assert response.status_code == 404
    assert response.json()["detail"] == "No such book."


def test_upstream_failure_is_a_502(ol):
    _FakeClient.missing = {"search.json"}
    response = ol.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 502
    assert response.json()["detail"] == books_router.SEARCH_UNAVAILABLE


def test_detail_survives_author_lookup_failure(ol):
    _FakeClient.missing = {"/authors/"}
    body = ol.get("/api/books/works/OL1W").json()
    assert body["title"] == "Circe"
    assert body["authors"] == ""


def test_book_detail_refresh_updates_local_row(ol):
    _shelve(ol)
    _FakeClient.calls = []
    local = ol.get("/api/books/works/OL1W").json()
    assert local["description"] == ""
    assert _FakeClient.calls == []

    refreshed = ol.post("/api/books/works/OL1W/refresh")
    assert refreshed.status_code == 200, refreshed.text
    body = refreshed.json()
    assert body["description"] == "A plain string description."
    assert body["subjects"] == ["Mythology", "Greek literature"]
    assert body["cover_id"] == 555
    assert any("/works/" in url for url, _ in _FakeClient.calls)

    _FakeClient.calls = []
    books_router.clear_search_cache()
    again = ol.get("/api/books/works/OL1W").json()
    assert again["description"] == "A plain string description."
    assert _FakeClient.calls == []


def test_search_cache_is_bounded(monkeypatch):
    books_router.clear_search_cache()
    monkeypatch.setattr(books_router, "_CACHE_MAX_KEYS", 2)
    books_router._cache_set("a", {"n": 1})
    books_router._cache_set("b", {"n": 2})
    books_router._cache_set("c", {"n": 3})
    assert books_router._cache_get("a") is None
    assert books_router._cache_get("b") == {"n": 2}
    assert books_router._cache_get("c") == {"n": 3}
    books_router.clear_search_cache()


def test_identical_inflight_searches_share_one_request(monkeypatch):
    import asyncio

    books_router.clear_search_cache()
    _FakeClient.reset()
    release = asyncio.Event()
    entered = asyncio.Event()
    original_get = _FakeClient.get

    async def slow_get(self, url, params=None, headers=None):
        entered.set()
        await release.wait()
        return await original_get(self, url, params, headers)

    monkeypatch.setattr(_FakeClient, "get", slow_get)
    monkeypatch.setattr(books_router.httpx, "AsyncClient", _FakeClient)

    async def run():
        first = asyncio.create_task(
            books_router.fetch_open_library(
                "circe", subject="", sort="relevance", page=1, limit=24
            )
        )
        await entered.wait()
        second = asyncio.create_task(
            books_router.fetch_open_library(
                "CIRCE", subject="", sort="relevance", page=1, limit=24
            )
        )
        await asyncio.sleep(0)
        release.set()
        return await asyncio.gather(first, second)

    pages = asyncio.run(run())
    assert pages[0].items[0].title == "Circe"
    assert pages[1].items[0].title == "Circe"
    assert len(_FakeClient.params_for("search.json")) == 1


def test_search_cache_ttl_is_at_least_half_a_day():
    assert books_router._CACHE_TTL >= 12 * 3600
