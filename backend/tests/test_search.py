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
        "number_of_pages_median": 393,
        "ratings_average": 4.27,
        "ratings_count": 812,
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
    def __init__(self, payload, url, status_code=200, headers=None):
        self._payload = payload
        self._url = url
        self.status_code = status_code
        self.headers = httpx.Headers(headers or {})

    def raise_for_status(self) -> None:
        if self.status_code < 400:
            return
        request = httpx.Request("GET", self._url)
        raise httpx.HTTPStatusError(
            f"{self.status_code}",
            request=request,
            response=httpx.Response(
                self.status_code, request=request, headers=self.headers
            ),
        )

    def json(self):
        return self._payload


class _FakeClient:
    """Routes Open Library URLs to canned payloads and records every call."""

    calls: list[tuple[str, dict | None]] = []
    work_payload: dict = {}
    missing: set[str] = set()
    status_for: dict[str, int] = {}
    retry_after: str | None = None
    author_name: str = "Madeline Miller"

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
        cls.status_for = {}
        cls.retry_after = None
        cls.author_name = "Madeline Miller"
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

        for marker, code in _FakeClient.status_for.items():
            if marker in url:
                extra = {}
                if _FakeClient.retry_after:
                    extra["Retry-After"] = _FakeClient.retry_after
                return _FakeResponse(None, url, status_code=code, headers=extra)

        for marker in _FakeClient.missing:
            if marker in url:
                return _FakeResponse(None, url, status_code=404)

        if "search.json" in url:
            q = str((params or {}).get("q") or "")
            if q == "zzzzempty" or q.startswith("zzzzempty ") or q.endswith(":zzzzempty"):
                return _FakeResponse({"numFound": 0, "docs": []}, url)
            if q.startswith("title:obscurexyz") or q.startswith("author:obscurexyz"):
                return _FakeResponse({"numFound": 1, "docs": [SEARCH_DOCS[0]]}, url)
            if q == "obscurexyz" or q.startswith("obscurexyz "):
                return _FakeResponse({"numFound": 0, "docs": []}, url)
            if "isbn:" in q:
                isbn_doc = {
                    **SEARCH_DOCS[0],
                    "isbn": ["9780316769488"],
                    "cover_edition_key": "OL1M",
                }
                return _FakeResponse({"numFound": 1, "docs": [isbn_doc]}, url)
            if 'title:"Stand"' in q and 'author:"Me"' in q:
                junk = {
                    "key": "/works/OLJUNKW",
                    "title": "Nothing Stands Between a Girl and Her Vegas",
                    "author_name": ["Raised Me"],
                    "cover_i": 1,
                    "first_publish_year": 2020,
                }
                return _FakeResponse({"numFound": 6, "docs": [junk]}, url)
            if q == "Stand by Me" or q.startswith("Stand by Me "):
                stand = {
                    "key": "/works/OLSTANDW",
                    "title": "Stand by Me",
                    "author_name": ["Ben E. King"],
                    "cover_i": 2,
                    "first_publish_year": 1986,
                }
                return _FakeResponse({"numFound": 1, "docs": [stand]}, url)
            return _FakeResponse({"numFound": 50, "docs": SEARCH_DOCS}, url)
        if "trending" in url:
            return _FakeResponse({"works": TRENDING_WORKS}, url)
        if "/subjects/" in url:
            return _FakeResponse({"work_count": 40, "works": SUBJECT_WORKS}, url)
        if "/authors/" in url:
            return _FakeResponse({"name": _FakeClient.author_name}, url)
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
    sent = _FakeClient.params_for("search.json")[0]
    assert sent["q"] == "circe"
    assert "sort" not in sent
    fields = str(sent["fields"]).split(",")
    assert "cover_edition_key" in fields
    assert "isbn" not in fields

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


def test_first_import_skips_author_api_when_search_has_names(ol):
    body = ol.get("/api/books/works/OL1W").json()
    assert body["authors"] == "Madeline Miller"
    assert [url for url, _ in _FakeClient.calls if "/authors/" in url] == []


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
    # Work JSON has no covers; the search.json doc still has cover_i.
    assert body["cover_id"] == 123


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
    # Author API failed; the search.json doc still has the name.
    assert body["authors"] == "Madeline Miller"


def test_book_detail_refresh_updates_local_row(ol):
    created = ol.post(
        "/api/shelf",
        json={
            "ol_work_key": "/works/OL1W",
            "title": "Circe",
            "status": "currently_reading",
        },
    )
    assert created.status_code == 201
    _FakeClient.calls = []
    local = ol.get("/api/books/works/OL1W").json()
    assert local["description"] == ""
    assert local["year"] is None
    assert _FakeClient.calls == []

    refreshed = ol.post("/api/books/works/OL1W/refresh")
    assert refreshed.status_code == 200, refreshed.text
    body = refreshed.json()
    assert body["description"] == "A plain string description."
    assert body["subjects"] == ["Mythology", "Greek literature"]
    assert body["cover_id"] == 555
    assert body["year"] == 2018
    assert any("/works/" in url for url, _ in _FakeClient.calls)
    assert any("search.json" in url for url, _ in _FakeClient.calls)

    shelf = ol.get("/api/shelf").json()["items"][0]
    assert shelf["book"]["pages"] == 393
    rich = ol.get("/api/books/work/OL1W").json()
    assert rich["pages"] == 393
    assert rich["ol_rating"] == 4.27
    assert rich["year"] == 2018

    _FakeClient.calls = []
    books_router.clear_search_cache()
    again = ol.get("/api/books/works/OL1W").json()
    assert again["description"] == "A plain string description."
    assert again["year"] == 2018
    assert _FakeClient.calls == []


def test_refresh_bypasses_author_cache(ol):
    first = ol.get("/api/books/works/OL1W").json()
    assert first["authors"] == "Madeline Miller"
    _FakeClient.author_name = "M. Miller"
    refreshed = ol.post("/api/books/works/OL1W/refresh").json()
    assert refreshed["authors"] == "M. Miller"


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


def test_open_library_connect_timeout_is_short():
    assert books_router._TIMEOUT.connect == 8.0
    assert books_router._TIMEOUT.read == 15.0
    assert books_router.OL_TIMEOUT.connect == 8.0
    assert books_router._TIMEOUT.connect < 10.0


def test_cancelled_fetch_unblocks_coalesced_waiters(monkeypatch):
    import asyncio

    from fastapi import HTTPException

    books_router.clear_search_cache()
    _FakeClient.reset()
    entered = asyncio.Event()
    release = asyncio.Event()
    original_get = _FakeClient.get

    async def slow_get(self, url, params=None, headers=None):
        entered.set()
        await release.wait()
        return await original_get(self, url, params, headers)

    monkeypatch.setattr(_FakeClient, "get", slow_get)
    monkeypatch.setattr(books_router.httpx, "AsyncClient", _FakeClient)

    async def run():
        owner = asyncio.create_task(
            books_router.fetch_open_library(
                "circe", subject="", sort="relevance", page=1, limit=24
            )
        )
        await entered.wait()
        follower = asyncio.create_task(
            books_router.fetch_open_library(
                "CIRCE", subject="", sort="relevance", page=1, limit=24
            )
        )
        await asyncio.sleep(0)
        owner.cancel()
        owner_exc = None
        try:
            await owner
        except asyncio.CancelledError as exc:
            owner_exc = exc
        follower_exc = None
        try:
            await asyncio.wait_for(follower, timeout=1)
        except HTTPException as exc:
            follower_exc = exc
        return owner_exc, follower_exc

    owner_exc, follower_exc = asyncio.run(run())
    assert isinstance(owner_exc, asyncio.CancelledError)
    assert follower_exc is not None
    assert follower_exc.status_code == 502
    assert books_router._inflight == {}


def test_short_query_is_a_400_and_never_hits_open_library(ol):
    response = ol.get("/api/books/search", params={"q": "it"})
    assert response.status_code == 400
    assert "3 characters" in response.json()["detail"]
    assert _FakeClient.params_for("search.json") == []

    two = ol.get("/api/books/search", params={"q": "ab"})
    assert two.status_code == 400
    assert _FakeClient.params_for("search.json") == []


def test_upstream_422_is_a_helpful_400(ol):
    _FakeClient.status_for = {"search.json": 422}
    response = ol.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 400
    assert "3 characters" in response.json()["detail"]


def test_upstream_429_is_a_429_with_retry_hint(ol):
    _FakeClient.status_for = {"search.json": 429}
    _FakeClient.retry_after = "20"
    response = ol.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 429
    assert "20 seconds" in response.json()["detail"]


def test_isbn_query_uses_isbn_field(ol):
    response = ol.get("/api/books/search", params={"q": "978-0-316-76948-8"})
    assert response.status_code == 200
    sent = _FakeClient.params_for("search.json")[-1]
    assert sent["q"] == "isbn:9780316769488"
    hit = response.json()["items"][0]
    assert hit["title"] == "Circe"
    assert hit["isbn"] == "9780316769488"
    assert hit["cover_edition_key"] == "OL1M"


def test_empty_search_pages_are_not_cached(ol):
    first = ol.get("/api/books/search", params={"q": "zzzzempty"})
    assert first.status_code == 200
    assert first.json()["items"] == []
    first_calls = len(_FakeClient.params_for("search.json"))
    assert first_calls >= 1

    second = ol.get("/api/books/search", params={"q": "zzzzempty"})
    assert second.status_code == 200
    assert len(_FakeClient.params_for("search.json")) > first_calls


def test_title_fallback_when_raw_query_misses(ol):
    response = ol.get("/api/books/search", params={"q": "obscurexyz"})
    assert response.status_code == 200
    qs = [params["q"] for params in _FakeClient.params_for("search.json")]
    assert "obscurexyz" in qs
    assert any(q.startswith("title:obscurexyz") for q in qs)
    assert any(q.startswith("author:obscurexyz") for q in qs)
    assert response.json()["items"][0]["title"] == "Circe"


def test_search_candidates_keep_raw_query_before_by_split():
    assert books_router._search_candidates("Stand by Me", "") == [
        "Stand by Me",
        'title:"Stand" author:"Me"',
    ]
    assert books_router._search_candidates("Circe by Madeline Miller", "") == [
        "Circe by Madeline Miller",
        'title:"Circe" author:"Madeline Miller"',
    ]


def test_title_containing_by_uses_raw_query_first(ol):
    """Regression: fielded-first turned Stand by Me into junk OL hits."""
    response = ol.get("/api/books/search", params={"q": "Stand by Me"})
    assert response.status_code == 200
    qs = [params["q"] for params in _FakeClient.params_for("search.json")]
    assert qs[0] == "Stand by Me"
    assert not any('title:"Stand"' in q for q in qs)
    assert response.json()["items"][0]["title"] == "Stand by Me"
    assert response.json()["items"][0]["ol_work_key"] == "/works/OLSTANDW"


def test_local_catalog_is_searched_without_open_library_for_custom_books(ol):
    created = ol.post(
        "/api/books/custom",
        json={"title": "My Zine", "authors": "Ada", "year": 2024},
    )
    assert created.status_code == 201, created.text
    body = created.json()
    assert body["custom"] is True
    assert body["title"] == "My Zine"
    assert body["ol_work_key"].startswith("/works/BC")
    work_id = body["ol_work_key"].rsplit("/", 1)[-1]

    _FakeClient.calls = []
    local = ol.get("/api/books/works/" + work_id)
    assert local.status_code == 200
    assert local.json()["custom"] is True
    assert _FakeClient.calls == []

    _FakeClient.calls = []
    search = ol.get("/api/books/search", params={"q": "zine"}).json()
    keys = [hit["ol_work_key"] for hit in search["items"]]
    assert body["ol_work_key"] in keys
    assert search["items"][0]["custom"] is True

    refresh = ol.post("/api/books/works/" + work_id + "/refresh")
    assert refresh.status_code == 400
    assert "Open Library" in refresh.json()["detail"]


def test_local_catalog_hits_survive_open_library_outage(ol):
    created = ol.post(
        "/api/books/custom",
        json={"title": "My Zine", "authors": "Ada", "year": 2024},
    )
    assert created.status_code == 201, created.text
    work_key = created.json()["ol_work_key"]

    _FakeClient.status_for = {"search.json": 503}
    response = ol.get("/api/books/search", params={"q": "zine"})
    assert response.status_code == 200, response.text
    items = response.json()["items"]
    assert items
    assert items[0]["title"] == "My Zine"
    assert items[0]["custom"] is True
    assert items[0]["ol_work_key"] == work_key
    assert response.json()["has_more"] is False


def test_custom_book_requires_a_title(ol):
    response = ol.post("/api/books/custom", json={"title": "   ", "authors": "Ada"})
    assert response.status_code in {400, 422}
