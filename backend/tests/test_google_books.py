import httpx
import pytest
from fastapi.testclient import TestClient

from app.config import get_settings
from app.googlebooks import (
    clear_google_cache,
    google_search_query,
    map_volume,
    map_volumes,
    volume_details,
)
from app.openlibrary import clear_details_cache
from app.routers import books as books_router
from app.security import reset_rate_limits
from app.works import (
    canonical_work_id,
    google_catalog_work_id,
    google_volume_id,
    isbn_from_work_key,
    is_google_catalog_id,
    is_work_id,
)
from tests.conftest import register

CIRCE_VOLUME = {
    "id": "zyTCAlFPjgYC",
    "volumeInfo": {
        "title": "Circe",
        "authors": ["Madeline Miller"],
        "publishedDate": "2018-04-10",
        "description": "<p>A witch on an island.</p>",
        "pageCount": 393,
        "categories": ["Fiction", "Mythology"],
        "averageRating": 4.5,
        "ratingsCount": 88,
        "industryIdentifiers": [
            {"type": "ISBN_13", "identifier": "9780316769488"},
            {"type": "ISBN_10", "identifier": "0316769487"},
        ],
        "imageLinks": {
            "smallThumbnail": (
                "http://books.google.com/books/content?id=zyTCAlFPjgYC"
                "&printsec=frontcover&img=1&zoom=5&source=gbs_api"
            ),
            "thumbnail": (
                "http://books.google.com/books/content?id=zyTCAlFPjgYC"
                "&printsec=frontcover&img=1&zoom=1&source=gbs_api"
            ),
        },
    },
}

CIRCE_COVER = (
    "https://books.google.com/books/content?id=zyTCAlFPjgYC"
    "&printsec=frontcover&img=1&zoom=0&source=gbs_api"
)

NO_ISBN_VOLUME = {
    "id": "abcVolumeId1",
    "volumeInfo": {
        "title": "Local Zine",
        "authors": ["Ada"],
        "publishedDate": "2024",
        "description": "Stapled.",
        "imageLinks": {
            "thumbnail": (
                "http://books.google.com/books/content?id=abcVolumeId1"
                "&printsec=frontcover&img=1&zoom=1&source=gbs_api"
            ),
        },
    },
}

ACHILLES_VOLUME = {
    "id": "achillesVol1",
    "volumeInfo": {
        "title": "The Song of Achilles",
        "authors": ["Madeline Miller"],
        "publishedDate": "2012",
        "industryIdentifiers": [{"type": "ISBN_13", "identifier": "9780062060624"}],
        "imageLinks": {
            "thumbnail": (
                "https://books.google.com/books/content?id=achillesVol1"
                "&printsec=frontcover&img=1&zoom=1&source=gbs_api"
            ),
        },
    },
}

OL_CIRCE = {
    "key": "/works/OL1W",
    "title": "Circe from Open Library",
    "author_name": ["Madeline Miller"],
    "cover_i": 123,
    "first_publish_year": 2018,
}


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
    calls: list[tuple[str, dict | None, dict | None]] = []
    gb_items: list[dict] = [CIRCE_VOLUME]
    gb_total: int = 1
    gb_status: int | None = None
    gb_error: dict | None = None
    ol_status: int | None = None
    ol_docs: list[dict] = [OL_CIRCE]
    volumes: dict[str, dict] = {}

    def __init__(self, *args, **kwargs):
        pass

    async def __aenter__(self):
        return self

    async def __aexit__(self, *args):
        return None

    @classmethod
    def reset(cls) -> None:
        cls.calls = []
        cls.gb_items = [CIRCE_VOLUME]
        cls.gb_total = 1
        cls.gb_status = None
        cls.gb_error = None
        cls.ol_status = None
        cls.ol_docs = [OL_CIRCE]
        cls.volumes = {
            "zyTCAlFPjgYC": CIRCE_VOLUME,
            "abcVolumeId1": NO_ISBN_VOLUME,
        }

    @classmethod
    def params_for(cls, needle: str) -> list[dict]:
        return [params or {} for url, params, *_ in cls.calls if needle in url]

    @classmethod
    def headers_for(cls, needle: str) -> list[dict]:
        return [headers or {} for url, _params, headers in cls.calls if needle in url]

    async def get(self, url, params=None, headers=None):
        _FakeClient.calls.append((url, params, headers))
        assert "Bookclub/1.0" in (headers or {}).get("User-Agent", "")

        if "googleapis.com/books" in url:
            assert "key" not in (params or {})
            assert (headers or {}).get("X-Goog-Api-Key") == "test-gb-key"
            if _FakeClient.gb_status:
                return _FakeResponse(
                    _FakeClient.gb_error, url, status_code=_FakeClient.gb_status
                )
            if url.rstrip("/").endswith("/volumes"):
                q = str((params or {}).get("q") or "")
                items = list(_FakeClient.gb_items)
                if (
                    q.startswith("isbn:0000000000")
                    or q.startswith("isbn:9780000000002")
                    or q == "zzzzempty"
                ):
                    items = []
                elif "Song of Achilles" in q:
                    items = [ACHILLES_VOLUME]
                return _FakeResponse(
                    {"totalItems": _FakeClient.gb_total, "items": items},
                    url,
                )
            volume_id = url.rsplit("/", 1)[-1]
            payload = _FakeClient.volumes.get(volume_id)
            if payload is None:
                return _FakeResponse(None, url, status_code=404)
            return _FakeResponse(payload, url)

        if _FakeClient.ol_status:
            extra = {}
            if _FakeClient.ol_status == 429:
                extra["Retry-After"] = "20"
            return _FakeResponse(None, url, status_code=_FakeClient.ol_status, headers=extra)
        if "search.json" in url:
            return _FakeResponse(
                {"numFound": len(_FakeClient.ol_docs), "docs": _FakeClient.ol_docs},
                url,
            )
        if "trending" in url:
            return _FakeResponse({"works": []}, url)
        if "/subjects/" in url:
            return _FakeResponse({"work_count": 0, "works": []}, url)
        raise AssertionError(f"unexpected url {url}")


def _install_fake(monkeypatch) -> None:
    from app import googlebooks, openlibrary

    monkeypatch.setattr(books_router.httpx, "AsyncClient", _FakeClient)
    monkeypatch.setattr(googlebooks.httpx, "AsyncClient", _FakeClient)
    monkeypatch.setattr(openlibrary.httpx, "AsyncClient", _FakeClient)


@pytest.fixture
def gb(tmp_path, monkeypatch):
    monkeypatch.delenv("BOOKCLUB_ENV_FILE", raising=False)
    monkeypatch.setenv("DATABASE_PATH", str(tmp_path / "bookclub.db"))
    monkeypatch.setenv("SECRET_KEY", "test-secret-key-which-is-long-enough")
    monkeypatch.setenv("BOOKCLUB_BOOTSTRAP_INVITE", "TESTINVITE")
    monkeypatch.setenv("DEBUG", "true")
    monkeypatch.setenv("BOOKCLUB_NAME", "Bookclub")
    monkeypatch.setenv("BOOKCLUB_THEME", "#b44a2a")
    monkeypatch.setenv("BOOKCLUB_PUBLIC_URL", "")
    monkeypatch.setenv("BOOKCLUB_TRUSTED_PROXIES", "*")
    monkeypatch.setenv("BOOKCLUB_TZ", "UTC")
    monkeypatch.setenv("GOOGLE_BOOKS_API_KEY", "test-gb-key")
    get_settings.cache_clear()
    reset_rate_limits()
    books_router.clear_search_cache()
    clear_details_cache()
    clear_google_cache()
    _FakeClient.reset()
    _install_fake(monkeypatch)
    from app.main import create_app

    with TestClient(create_app()) as client:
        register(client, "ada")
        yield client


def test_google_books_disabled_without_key(client):
    from app.googlebooks import google_books_enabled

    assert google_books_enabled() is False


def test_google_catalog_ids():
    assert is_work_id("ISBN9780316769488")
    assert is_work_id("GBzyTCAlFPjgYC")
    assert is_google_catalog_id("ISBN9780316769488")
    assert is_google_catalog_id("GBzyTCAlFPjgYC")
    assert not is_google_catalog_id("OL1W")
    assert not is_google_catalog_id("BCdeadbeef01")
    assert isbn_from_work_key("/works/ISBN9780316769488") == "9780316769488"
    assert google_volume_id("GBzyTCAlFPjgYC") == "zyTCAlFPjgYC"
    assert google_catalog_work_id(isbn="978-0-316-76948-8", volume_id="aa") == (
        "ISBN9780316769488"
    )
    assert google_catalog_work_id(isbn=None, volume_id="zyTCAlFPjgYC") == "GBzyTCAlFPjgYC"
    assert google_search_query("978-0-316-76948-8") == "isbn:9780316769488"
    assert google_search_query("circe", "science_fiction") == (
        'circe subject:"science fiction"'
    )
    assert canonical_work_id("ISBN080442957x") == "ISBN080442957X"


def test_map_volume_prefers_isbn_key():
    hit = map_volume(CIRCE_VOLUME)
    assert hit is not None
    assert hit.ol_work_key == "/works/ISBN9780316769488"
    assert hit.title == "Circe"
    assert hit.authors == "Madeline Miller"
    assert hit.year == 2018
    assert hit.isbn == "9780316769488"
    assert hit.cover_id is None
    assert hit.cover_url == CIRCE_COVER


def test_map_volume_without_isbn_uses_volume_id():
    hit = map_volume(NO_ISBN_VOLUME)
    assert hit is not None
    assert hit.ol_work_key == "/works/GBabcVolumeId1"
    assert hit.isbn is None


def test_map_volumes_skips_junk_and_duplicates():
    dup = {**CIRCE_VOLUME, "id": "other"}
    hits = map_volumes([CIRCE_VOLUME, {"id": "x"}, dup, NO_ISBN_VOLUME])
    assert [hit.ol_work_key for hit in hits] == [
        "/works/ISBN9780316769488",
        "/works/GBabcVolumeId1",
    ]


def test_volume_details_strips_html():
    details = volume_details(CIRCE_VOLUME)
    assert details is not None
    assert details.description == "A witch on an island."
    assert details.pages == 393
    assert details.subjects == ["Fiction", "Mythology"]
    assert details.rating == 4.5
    assert details.cover_image_url == CIRCE_COVER


def _open_library_urls() -> list[str]:
    return [url for url, *_ in _FakeClient.calls if "openlibrary.org" in url]


def test_search_prefers_google_books(gb):
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 200
    items = response.json()["items"]
    assert len(items) == 1
    assert items[0]["ol_work_key"] == "/works/ISBN9780316769488"
    assert items[0]["title"] == "Circe"
    assert items[0]["isbn"] == "9780316769488"
    assert items[0]["year"] == 2018
    assert items[0]["cover_url"] == CIRCE_COVER
    gb_calls = _FakeClient.params_for("googleapis.com/books")
    assert len(gb_calls) == 1
    assert gb_calls[0]["q"] == "circe"
    assert "key" not in gb_calls[0]
    assert "imageLinks" in str(gb_calls[0].get("fields") or "")
    assert _FakeClient.headers_for("googleapis.com/books")[0]["X-Goog-Api-Key"] == (
        "test-gb-key"
    )
    assert _FakeClient.params_for("search.json") == []
    assert _open_library_urls() == []

    gb.get("/api/books/search", params={"q": "Circe"})
    gb.get("/api/books/search", params={"q": "  CIRCE  "})
    assert len(_FakeClient.params_for("googleapis.com/books")) == 1


def test_search_empty_google_does_not_fall_back(gb):
    _FakeClient.gb_items = []
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 200
    assert response.json()["items"] == []
    assert _FakeClient.params_for("googleapis.com/books")
    assert _open_library_urls() == []


def test_search_google_429_does_not_fall_back(gb):
    _FakeClient.gb_status = 429
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 429
    assert "busy" in response.json()["detail"].lower()
    assert _open_library_urls() == []


def test_search_google_500_does_not_fall_back(gb):
    _FakeClient.gb_status = 500
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 502
    assert _open_library_urls() == []


def test_browse_and_empty_search_use_google_not_open_library(gb):
    browse = gb.get("/api/books/search", params={"subject": "fantasy"})
    assert browse.status_code == 200
    assert browse.json()["items"][0]["ol_work_key"] == "/works/ISBN9780316769488"
    qs = _FakeClient.params_for("googleapis.com/books")
    assert qs
    assert 'subject:"fantasy"' in qs[0]["q"]
    assert _open_library_urls() == []

    _FakeClient.calls = []
    trending = gb.get("/api/books/trending")
    assert trending.status_code == 200
    assert trending.json()["items"][0]["cover_url"] == CIRCE_COVER
    tq = _FakeClient.params_for("googleapis.com/books")
    assert tq
    assert tq[0]["q"] == 'subject:"fiction"'
    assert tq[0].get("orderBy") == "newest"
    assert _FakeClient.params_for("trending") == []
    assert _open_library_urls() == []

    _FakeClient.calls = []
    empty = gb.get("/api/books/search")
    assert empty.status_code == 200
    assert empty.json()["items"][0]["title"] == "Circe"
    assert _FakeClient.params_for("trending") == []
    assert _open_library_urls() == []

    _FakeClient.calls = []
    subject = gb.get("/api/books/subjects/science_fiction")
    assert subject.status_code == 200
    sq = _FakeClient.params_for("googleapis.com/books")
    assert sq
    assert 'subject:"science fiction"' in sq[0]["q"]
    assert not any("/subjects/" in url for url, *_ in _FakeClient.calls)
    assert _open_library_urls() == []


def test_isbn_search_uses_google_isbn_field(gb):
    response = gb.get("/api/books/search", params={"q": "978-0-316-76948-8"})
    assert response.status_code == 200
    sent = _FakeClient.params_for("googleapis.com/books")[-1]
    assert sent["q"] == "isbn:9780316769488"
    assert response.json()["items"][0]["ol_work_key"] == "/works/ISBN9780316769488"


def test_detail_imports_from_google_not_open_library(gb):
    body = gb.get("/api/books/works/ISBN9780316769488").json()
    assert body["ol_work_key"] == "/works/ISBN9780316769488"
    assert body["title"] == "Circe"
    assert body["description"] == "A witch on an island."
    assert body["custom"] is False
    assert body["cover_url"] == CIRCE_COVER
    assert any("googleapis.com/books" in url for url, *_ in _FakeClient.calls)
    assert not any("openlibrary.org/works/" in url for url, *_ in _FakeClient.calls)
    assert _open_library_urls() == []

    _FakeClient.calls = []
    again = gb.get("/api/books/works/ISBN9780316769488")
    assert again.status_code == 200
    assert _FakeClient.calls == []


def test_volume_key_detail_and_refresh(gb):
    body = gb.get("/api/books/works/GBabcVolumeId1").json()
    assert body["ol_work_key"] == "/works/GBabcVolumeId1"
    assert body["title"] == "Local Zine"
    assert body["cover_url"] == (
        "https://books.google.com/books/content?id=abcVolumeId1"
        "&printsec=frontcover&img=1&zoom=0&source=gbs_api"
    )
    assert any(url.endswith("/volumes/abcVolumeId1") for url, *_ in _FakeClient.calls)

    _FakeClient.volumes["abcVolumeId1"] = {
        **NO_ISBN_VOLUME,
        "volumeInfo": {
            **NO_ISBN_VOLUME["volumeInfo"],
            "description": "Updated copy.",
            "pageCount": 12,
        },
    }
    refreshed = gb.post("/api/books/works/GBabcVolumeId1/refresh")
    assert refreshed.status_code == 200, refreshed.text
    assert refreshed.json()["description"] == "Updated copy."


def test_known_google_book_skips_upstream(gb):
    gb.post(
        "/api/shelf",
        json={
            "ol_work_key": "/works/ISBN9780316769488",
            "title": "Circe",
            "authors": "Madeline Miller",
            "year": 2018,
            "status": "currently_reading",
        },
    )
    _FakeClient.calls = []
    body = gb.get("/api/books/works/ISBN9780316769488").json()
    assert body["title"] == "Circe"
    assert body["on_shelf"] == "currently_reading"
    assert _FakeClient.calls == []

    pick = gb.put(
        "/api/pick",
        json={
            "ol_work_key": "/works/ISBN9780316769488",
            "title": "Circe",
            "authors": "Madeline Miller",
            "year": 2018,
        },
    )
    assert pick.status_code == 200, pick.text

    diary = gb.get("/api/books/works/ISBN9780316769488/posts")
    assert diary.status_code == 200
    assert diary.json()["ol_work_key"] == "/works/ISBN9780316769488"


def test_refresh_updates_google_isbn_book(gb):
    gb.post(
        "/api/shelf",
        json={
            "ol_work_key": "/works/ISBN9780316769488",
            "title": "Circe",
            "status": "want_to_read",
        },
    )
    local = gb.get("/api/books/works/ISBN9780316769488").json()
    assert local["description"] == ""

    refreshed = gb.post("/api/books/works/ISBN9780316769488/refresh")
    assert refreshed.status_code == 200
    assert refreshed.json()["description"] == "A witch on an island."
    assert not any("/works/OL" in url for url, *_ in _FakeClient.calls)


def test_isbn_lookup_prefers_google(gb):
    response = gb.get("/api/books/isbn/9780316769488")
    assert response.status_code == 200
    body = response.json()
    assert body["ol_work_key"] == "/works/ISBN9780316769488"
    assert body["title"] == "Circe"
    assert body["cover_url"] == CIRCE_COVER
    assert _FakeClient.params_for("search.json") == []
    assert _open_library_urls() == []


def test_club_custom_book_still_skips_catalogs(gb):
    created = gb.post(
        "/api/books/custom",
        json={"title": "My Zine", "authors": "Ada", "year": 2024},
    )
    assert created.status_code == 201
    work_id = created.json()["ol_work_key"].rsplit("/", 1)[-1]
    _FakeClient.calls = []
    local = gb.get("/api/books/works/" + work_id)
    assert local.status_code == 200
    assert local.json()["custom"] is True
    assert _FakeClient.calls == []
    refresh = gb.post("/api/books/works/" + work_id + "/refresh")
    assert refresh.status_code == 400


def test_cover_url_from_image_links_https_and_zoom():
    from app.covers import cover_url_from_image_links, normalize_cover_image_url

    assert cover_url_from_image_links(CIRCE_VOLUME["volumeInfo"]["imageLinks"]) == (
        CIRCE_COVER
    )
    assert cover_url_from_image_links(None) is None
    assert cover_url_from_image_links({"thumbnail": "javascript:alert(1)"}) is None
    assert normalize_cover_image_url("http://books.google.com/books/content?id=x") == (
        "https://books.google.com/books/content?id=x"
    )
    from app.covers import storable_cover_image_url

    assert storable_cover_image_url(CIRCE_COVER) == CIRCE_COVER
    assert (
        storable_cover_image_url(
            "https://covers.openlibrary.org/b/id/123-L.jpg?default=false"
        )
        is None
    )
    assert storable_cover_image_url("https://attacker.example/pixel.gif") is None


def test_isbn_cover_url_from_work_key(monkeypatch):
    from app.config import get_settings
    from app.serialize import cover_url

    monkeypatch.delenv("GOOGLE_BOOKS_API_KEY", raising=False)
    get_settings.cache_clear()

    assert cover_url(None, "/works/ISBN9780316769488") == (
        "https://covers.openlibrary.org/b/isbn/9780316769488-L.jpg?default=false"
    )
    assert cover_url(123, "/works/ISBN9780316769488").endswith("/123-L.jpg?default=false")
    assert cover_url(None, "/works/GBabcVolumeId1") is None
    assert cover_url(
        None,
        "/works/GBabcVolumeId1",
        "http://books.google.com/books/content?id=abcVolumeId1",
    ) == "https://books.google.com/books/content?id=abcVolumeId1"


def test_google_cover_wins_over_open_library_isbn_cdn():
    from app.serialize import cover_url

    assert cover_url(123, "/works/ISBN9780316769488", CIRCE_COVER) == CIRCE_COVER


def test_isbn_cdn_skipped_when_google_key_is_set(gb):
    from app.serialize import cover_url

    assert cover_url(None, "/works/ISBN9780316769488") is None
    assert cover_url(555, "/works/OL1W") == (
        "https://covers.openlibrary.org/b/id/555-L.jpg?default=false"
    )


def test_later_google_page_does_not_mix_open_library(gb):
    _FakeClient.gb_total = 50
    page1 = gb.get("/api/books/search", params={"q": "circe", "page": 1})
    assert page1.status_code == 200
    assert page1.json()["items"][0]["ol_work_key"] == "/works/ISBN9780316769488"
    assert page1.json()["has_more"] is True

    _FakeClient.gb_items = []
    page2 = gb.get("/api/books/search", params={"q": "circe", "page": 2})
    assert page2.status_code == 200
    assert page2.json()["items"] == []
    assert page2.json()["has_more"] is False
    assert _FakeClient.params_for("search.json") == []


def test_title_and_popular_sorts_stay_on_google(gb):
    title = gb.get("/api/books/search", params={"q": "circe", "sort": "title"})
    assert title.status_code == 200
    assert title.json()["items"][0]["ol_work_key"] == "/works/ISBN9780316769488"
    assert "orderBy" not in _FakeClient.params_for("googleapis.com/books")[-1]
    assert _open_library_urls() == []

    popular = gb.get("/api/books/search", params={"q": "circe", "sort": "readinglog"})
    assert popular.status_code == 200
    assert popular.json()["items"][0]["cover_url"] == CIRCE_COVER
    assert popular.json()["items"][0]["ol_work_key"] == "/works/ISBN9780316769488"
    assert _open_library_urls() == []


def test_isbn_refresh_keeps_description_when_google_misses(gb):
    imported = gb.get("/api/books/works/ISBN9780316769488").json()
    assert imported["description"] == "A witch on an island."

    _FakeClient.gb_items = []
    refreshed = gb.post("/api/books/works/ISBN9780316769488/refresh")
    assert refreshed.status_code == 200
    assert refreshed.json()["description"] == "A witch on an island."
    assert refreshed.json()["subjects"] == ["Fiction", "Mythology"]
    assert refreshed.json()["cover_url"] == CIRCE_COVER
    assert _open_library_urls() == []


def test_isbn_refresh_without_key_keeps_description(gb, monkeypatch):
    imported = gb.get("/api/books/works/ISBN9780316769488").json()
    assert imported["description"] == "A witch on an island."

    monkeypatch.setenv("GOOGLE_BOOKS_API_KEY", "")
    get_settings.cache_clear()
    _FakeClient.calls = []
    refreshed = gb.post("/api/books/works/ISBN9780316769488/refresh")
    assert refreshed.status_code == 200
    assert refreshed.json()["description"] == "A witch on an island."
    assert _FakeClient.params_for("googleapis.com/books") == []


def test_isbn_x_and_X_are_the_same_row(gb):
    first = gb.get("/api/books/works/ISBN080442957X")
    assert first.status_code == 200
    second = gb.get("/api/books/works/ISBN080442957x")
    assert second.status_code == 200
    assert first.json()["ol_work_key"] == "/works/ISBN080442957X"
    assert second.json()["ol_work_key"] == "/works/ISBN080442957X"


def test_isbn_lookup_miss_does_not_use_open_library(gb):
    _FakeClient.gb_items = []
    response = gb.get("/api/books/isbn/9780316769488")
    assert response.status_code == 404
    assert _open_library_urls() == []


def test_shelf_add_persists_google_cover_url(gb):
    added = gb.post(
        "/api/shelf",
        json={
            "ol_work_key": "/works/ISBN9780316769488",
            "title": "Circe",
            "authors": "Madeline Miller",
            "cover_url": CIRCE_COVER,
            "status": "want_to_read",
        },
    )
    assert added.status_code == 201, added.text
    assert added.json()["book"]["cover_url"] == CIRCE_COVER
    _FakeClient.calls = []
    detail = gb.get("/api/books/works/ISBN9780316769488").json()
    assert detail["cover_url"] == CIRCE_COVER
    assert _FakeClient.calls == []


def test_volume_key_without_google_key(client):
    register(client, "ada")
    assert client.get("/api/books/works/GBabcVolumeId1").status_code == 404
    refresh = client.post("/api/books/works/GBabcVolumeId1/refresh")
    assert refresh.status_code == 400
    assert "GOOGLE_BOOKS_API_KEY" in refresh.json()["detail"]


def test_volume_refresh_429_is_classified(gb):
    gb.get("/api/books/works/GBabcVolumeId1")
    _FakeClient.gb_status = 429
    response = gb.post("/api/books/works/GBabcVolumeId1/refresh")
    assert response.status_code == 429
    assert "busy" in response.json()["detail"].lower()


def test_goodreads_import_prefers_google(gb):
    from pathlib import Path

    raw = (Path(__file__).parent / "fixtures" / "goodreads.csv").read_bytes()
    response = gb.post(
        "/api/shelf/import",
        files={"file": ("goodreads.csv", raw, "text/csv")},
    )
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["imported"] == 2
    shelf = gb.get("/api/shelf").json()["items"]
    keys = {row["book"]["ol_work_key"] for row in shelf}
    assert "/works/ISBN9780316769488" in keys
    assert "/works/ISBN9780062060624" in keys
    assert _FakeClient.params_for("search.json") == []
    assert _open_library_urls() == []
    covers = {row["book"]["ol_work_key"]: row["book"]["cover_url"] for row in shelf}
    assert covers["/works/ISBN9780316769488"] == CIRCE_COVER


def _book_row(client, key: str):
    from sqlmodel import Session, select

    from app.models import Book

    with Session(client.app.state.engine) as session:
        return session.exec(select(Book).where(Book.ol_work_key == key)).first()


def test_pick_does_not_persist_ol_cdn_cover_url(client):
    register(client, "ada")
    ol_cdn = "https://covers.openlibrary.org/b/id/123-L.jpg?default=false"
    response = client.put(
        "/api/pick",
        json={
            "ol_work_key": "/works/OL1W",
            "title": "Circe",
            "cover_id": 123,
            "cover_url": ol_cdn,
        },
    )
    assert response.status_code == 200, response.text
    book = _book_row(client, "/works/OL1W")
    assert book is not None
    assert book.cover_image_url is None
    assert book.cover_id == 123


def test_shelf_rejects_non_google_cover_url(client):
    register(client, "ada")
    response = client.post(
        "/api/shelf",
        json={
            "ol_work_key": "/works/OL1W",
            "title": "Circe",
            "cover_id": 123,
            "cover_url": "https://attacker.example/pixel.gif?u=1",
            "status": "want_to_read",
        },
    )
    assert response.status_code == 201, response.text
    book = _book_row(client, "/works/OL1W")
    assert book is not None
    assert book.cover_image_url is None
    assert book.cover_id == 123


def test_shelf_isbn_x_and_X_are_the_same_row(gb):
    first = gb.post(
        "/api/shelf",
        json={
            "ol_work_key": "/works/ISBN080442957X",
            "title": "Circe",
            "status": "want_to_read",
        },
    )
    assert first.status_code == 201, first.text
    second = gb.post(
        "/api/shelf",
        json={
            "ol_work_key": "/works/ISBN080442957x",
            "title": "Circe",
            "status": "want_to_read",
        },
    )
    assert second.status_code == 409
    keys = {row["book"]["ol_work_key"] for row in gb.get("/api/shelf").json()["items"]}
    assert keys == {"/works/ISBN080442957X"}


def test_google_403_access_not_configured_is_502(gb):
    _FakeClient.gb_status = 403
    _FakeClient.gb_error = {
        "error": {
            "message": "Books API has not been used in project 123 before or it is disabled.",
            "errors": [{"reason": "accessNotConfigured"}],
        }
    }
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 502
    assert "busy" not in response.json()["detail"].lower()

    _FakeClient.gb_status = None
    _FakeClient.gb_error = None
    _FakeClient.calls = []
    again = gb.get("/api/books/search", params={"q": "other"})
    assert again.status_code == 200
    assert _FakeClient.params_for("googleapis.com/books")


def test_google_403_quota_cools_down(gb):
    _FakeClient.gb_status = 403
    _FakeClient.gb_error = {
        "error": {
            "message": "Quota exceeded",
            "errors": [{"reason": "rateLimitExceeded"}],
        }
    }
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 429

    _FakeClient.gb_status = None
    _FakeClient.gb_error = None
    _FakeClient.calls = []
    again = gb.get("/api/books/search", params={"q": "other"})
    assert again.status_code == 429
    assert _FakeClient.params_for("googleapis.com/books") == []


def test_goodreads_import_while_cooling_down(gb):
    from pathlib import Path

    _FakeClient.gb_status = 429
    gb.get("/api/books/search", params={"q": "circe"})
    _FakeClient.gb_status = None
    _FakeClient.calls = []
    raw = (Path(__file__).parent / "fixtures" / "goodreads.csv").read_bytes()
    response = gb.post(
        "/api/shelf/import",
        files={"file": ("goodreads.csv", raw, "text/csv")},
    )
    assert response.status_code == 429
    assert "busy" in response.json()["detail"].lower()
    assert _FakeClient.params_for("googleapis.com/books") == []


def test_later_empty_google_page_has_more_false_when_total_stays_high(gb):
    _FakeClient.gb_total = 50
    page1 = gb.get("/api/books/search", params={"q": "circe", "page": 1})
    assert page1.json()["has_more"] is True
    _FakeClient.gb_items = []
    page2 = gb.get("/api/books/search", params={"q": "circe", "page": 2})
    assert page2.status_code == 200
    assert page2.json()["items"] == []
    assert page2.json()["has_more"] is False
    assert _open_library_urls() == []

