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
    },
}

NO_ISBN_VOLUME = {
    "id": "abcVolumeId1",
    "volumeInfo": {
        "title": "Local Zine",
        "authors": ["Ada"],
        "publishedDate": "2024",
        "description": "Stapled.",
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
    calls: list[tuple[str, dict | None]] = []
    gb_items: list[dict] = [CIRCE_VOLUME]
    gb_total: int = 1
    gb_status: int | None = None
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
        cls.ol_status = None
        cls.ol_docs = [OL_CIRCE]
        cls.volumes = {
            "zyTCAlFPjgYC": CIRCE_VOLUME,
            "abcVolumeId1": NO_ISBN_VOLUME,
        }

    @classmethod
    def params_for(cls, needle: str) -> list[dict]:
        return [params or {} for url, params in cls.calls if needle in url]

    async def get(self, url, params=None, headers=None):
        _FakeClient.calls.append((url, params))
        assert "Bookclub/1.0" in (headers or {}).get("User-Agent", "")

        if "googleapis.com/books" in url:
            if _FakeClient.gb_status:
                return _FakeResponse(None, url, status_code=_FakeClient.gb_status)
            if url.rstrip("/").endswith("/volumes"):
                q = str((params or {}).get("q") or "")
                items = list(_FakeClient.gb_items)
                if q.startswith("isbn:0000000000") or q == "zzzzempty":
                    items = []
                return _FakeResponse(
                    {"totalItems": 0 if not items else _FakeClient.gb_total, "items": items},
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
    assert google_search_query("circe", "fantasy") == "circe subject:fantasy"


def test_map_volume_prefers_isbn_key():
    hit = map_volume(CIRCE_VOLUME)
    assert hit is not None
    assert hit.ol_work_key == "/works/ISBN9780316769488"
    assert hit.title == "Circe"
    assert hit.authors == "Madeline Miller"
    assert hit.year == 2018
    assert hit.isbn == "9780316769488"
    assert hit.cover_id is None


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


def test_search_prefers_google_books(gb):
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 200
    items = response.json()["items"]
    assert len(items) == 1
    assert items[0]["ol_work_key"] == "/works/ISBN9780316769488"
    assert items[0]["title"] == "Circe"
    assert items[0]["isbn"] == "9780316769488"
    assert items[0]["year"] == 2018
    gb_calls = _FakeClient.params_for("googleapis.com/books")
    assert len(gb_calls) == 1
    assert gb_calls[0]["q"] == "circe"
    assert gb_calls[0]["key"] == "test-gb-key"
    assert _FakeClient.params_for("search.json") == []

    gb.get("/api/books/search", params={"q": "Circe"})
    gb.get("/api/books/search", params={"q": "  CIRCE  "})
    assert len(_FakeClient.params_for("googleapis.com/books")) == 1


def test_search_falls_back_when_google_is_empty(gb):
    _FakeClient.gb_items = []
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 200
    assert response.json()["items"][0]["title"] == "Circe from Open Library"
    assert response.json()["items"][0]["ol_work_key"] == "/works/OL1W"
    assert _FakeClient.params_for("googleapis.com/books")
    assert _FakeClient.params_for("search.json")


def test_search_falls_back_when_google_is_429(gb):
    _FakeClient.gb_status = 429
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 200
    assert response.json()["items"][0]["ol_work_key"] == "/works/OL1W"
    assert _FakeClient.params_for("search.json")


def test_search_falls_back_when_google_is_500(gb):
    _FakeClient.gb_status = 500
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 200
    assert response.json()["items"][0]["ol_work_key"] == "/works/OL1W"


def test_classified_open_library_error_after_google_failure(gb):
    _FakeClient.gb_status = 429
    _FakeClient.ol_status = 429
    response = gb.get("/api/books/search", params={"q": "circe"})
    assert response.status_code == 429
    assert "20 seconds" in response.json()["detail"]


def test_subject_only_and_trending_stay_on_open_library(gb):
    browse = gb.get("/api/books/search", params={"subject": "fantasy"})
    assert browse.status_code == 200
    assert _FakeClient.params_for("googleapis.com/books") == []
    assert _FakeClient.params_for("search.json")

    _FakeClient.calls = []
    trending = gb.get("/api/books/trending")
    assert trending.status_code == 200
    assert _FakeClient.params_for("googleapis.com/books") == []
    assert _FakeClient.params_for("trending")


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
    assert any("googleapis.com/books" in url for url, _ in _FakeClient.calls)
    assert not any("openlibrary.org/works/" in url for url, _ in _FakeClient.calls)

    _FakeClient.calls = []
    again = gb.get("/api/books/works/ISBN9780316769488")
    assert again.status_code == 200
    assert _FakeClient.calls == []


def test_volume_key_detail_and_refresh(gb):
    body = gb.get("/api/books/works/GBabcVolumeId1").json()
    assert body["ol_work_key"] == "/works/GBabcVolumeId1"
    assert body["title"] == "Local Zine"
    assert any(url.endswith("/volumes/abcVolumeId1") for url, _ in _FakeClient.calls)

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
    assert not any("/works/OL" in url for url, _ in _FakeClient.calls)


def test_isbn_lookup_prefers_google(gb):
    response = gb.get("/api/books/isbn/9780316769488")
    assert response.status_code == 200
    body = response.json()
    assert body["ol_work_key"] == "/works/ISBN9780316769488"
    assert body["title"] == "Circe"
    assert _FakeClient.params_for("search.json") == []


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


def test_isbn_cover_url_from_work_key():
    from app.serialize import cover_url

    assert cover_url(None, "/works/ISBN9780316769488") == (
        "https://covers.openlibrary.org/b/isbn/9780316769488-L.jpg?default=false"
    )
    assert cover_url(123, "/works/ISBN9780316769488").endswith("/123-L.jpg?default=false")
    assert cover_url(None, "/works/GBabcVolumeId1") is None
