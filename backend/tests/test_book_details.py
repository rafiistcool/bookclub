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
    assert body["cover_url"].endswith("/123-L.jpg?default=false")
    assert len(ol.calls) == 2

    # First view imports the work; later reads are local even after the RAM cache is cleared.
    openlibrary.clear_details_cache()
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

    calls_before = len(ol.calls)
    body = client.get("/api/books/work/OL1W").json()
    assert len(ol.calls) == calls_before
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
    assert body["pages"] is None

    refreshed = client.post("/api/books/work/OL1W/refresh")
    assert refreshed.status_code == 200, refreshed.text
    assert refreshed.json()["pages"] == 393
    shelf = client.get("/api/shelf").json()["items"][0]
    assert shelf["book"]["pages"] == 393

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
    calls = len(ol.calls)
    again = client.get("/api/books/isbn/9780316556347").json()
    assert again["on_shelf"] == "want_to_read"
    assert again["shelf_id"] is not None
    assert len(ol.calls) == calls


def test_isbn10_with_x_check_digit_is_accepted(client, ol):
    register(client, "ada")
    assert client.get("/api/books/isbn/080442957X").status_code == 200


def test_work_details_skips_leading_negative_cover(client, monkeypatch):
    register(client, "ada")
    search = {
        "numFound": 1,
        "docs": [
            {
                "key": "/works/OL1W",
                "title": "Circe",
                "author_name": ["Madeline Miller"],
                "cover_i": -1,
                "first_publish_year": 2018,
            }
        ],
    }
    work = {**WORK_JSON, "covers": [-1, 0, 555]}

    class Client(_Client):
        async def get(self, url, params=None, headers=None):
            _Client.calls.append(url)
            if url.endswith("/search.json"):
                return _Resp(search)
            return _Resp(work)

    monkeypatch.setattr(openlibrary.httpx, "AsyncClient", Client)
    body = client.get("/api/books/work/OL1W").json()
    assert body["cover_id"] == 555
    assert body["cover_url"].endswith("/555-L.jpg?default=false")


def test_positive_cover_id_rejects_placeholders():
    assert openlibrary.positive_cover_id(-1) is None
    assert openlibrary.positive_cover_id(0) is None
    assert openlibrary.positive_cover_id(None) is None
    assert openlibrary.first_positive_cover(-1, [0, -1, 99], 12) == 99
    assert openlibrary.extract_isbn("978-0-316-76948-8") == "9780316769488"
    assert openlibrary.extract_isbn("it") is None


def test_cancelled_work_details_unblocks_coalesced_waiters(monkeypatch):
    import asyncio

    from fastapi import HTTPException

    openlibrary.clear_details_cache()
    entered = asyncio.Event()
    release = asyncio.Event()

    async def slow_get(client, url, params=None):
        entered.set()
        await release.wait()
        return {}

    monkeypatch.setattr(openlibrary, "_get", slow_get)

    async def run():
        owner = asyncio.create_task(openlibrary.fetch_work_details("/works/OL1W"))
        await entered.wait()
        follower = asyncio.create_task(openlibrary.fetch_work_details("/works/OL1W"))
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
    assert openlibrary._details_inflight == {}
