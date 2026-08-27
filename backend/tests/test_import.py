from pathlib import Path

from app.goodreads import MAX_IMPORT_BYTES, clean_isbn, parse_goodreads_csv
from app.models import ShelfStatus
from tests.conftest import register

FIXTURE = Path(__file__).parent / "fixtures" / "goodreads.csv"


class _FakeResponse:
    def __init__(self, docs: list[dict]):
        self.status_code = 200
        self._docs = docs

    def raise_for_status(self) -> None:
        return None

    def json(self) -> dict:
        return {"numFound": len(self._docs), "docs": self._docs}


class _FakeClient:
    calls: list[dict] = []

    def __init__(self, *args, **kwargs):
        pass

    async def __aenter__(self):
        return self

    async def __aexit__(self, *args):
        return None

    async def get(self, url, params=None, headers=None):
        query = str((params or {}).get("q") or "")
        _FakeClient.calls.append({"q": query})
        if query.startswith("isbn:9780316556347"):
            return _FakeResponse(
                [
                    {
                        "key": "/works/OL1W",
                        "title": "Circe",
                        "author_name": ["Madeline Miller"],
                        "cover_i": 123,
                        "first_publish_year": 2018,
                    }
                ]
            )
        if "Song of Achilles" in query:
            return _FakeResponse(
                [
                    {
                        "key": "/works/OL2W",
                        "title": "The Song of Achilles",
                        "author_name": ["Madeline Miller"],
                        "cover_i": 456,
                        "first_publish_year": 2012,
                    }
                ]
            )
        return _FakeResponse([])


def _import(client, raw: bytes, filename: str = "goodreads.csv"):
    return client.post(
        "/api/shelf/import",
        files={"file": (filename, raw, "text/csv")},
    )


def test_clean_isbn_unwraps_goodreads_formula():
    assert clean_isbn('="9780316556347"') == "9780316556347"
    assert clean_isbn('="0316556343"') == "0316556343"
    assert clean_isbn("") == ""
    assert clean_isbn('=""') == ""


def test_parse_fixture_maps_shelves_and_skips():
    rows, skips = parse_goodreads_csv(FIXTURE.read_bytes())
    assert [(row.title, row.status, row.isbn) for row in rows] == [
        ("Circe", ShelfStatus.want_to_read, "9780316556347"),
        ("The Song of Achilles", ShelfStatus.currently_reading, ""),
        ("Unmatched Novel", ShelfStatus.finished, "9780000000002"),
    ]
    reasons = {title: reason for title, reason in skips}
    assert reasons["Custom Shelf Book"] == "Unknown shelf"
    assert reasons["Untitled"] == "No title"


def test_import_requires_auth(client):
    assert _import(client, FIXTURE.read_bytes()).status_code == 401


def test_import_matches_isbn_and_title(client, monkeypatch):
    from app import goodreads

    register(client, "ada")
    _FakeClient.calls = []
    monkeypatch.setattr(goodreads.httpx, "AsyncClient", _FakeClient)

    response = _import(client, FIXTURE.read_bytes())
    assert response.status_code == 200
    body = response.json()
    assert body["imported"] == 2
    assert body["skipped"] == 3
    skip_titles = {row["title"] for row in body["skips"]}
    assert skip_titles == {"Unmatched Novel", "Custom Shelf Book", "Untitled"}
    assert any(row["reason"] == "No match" for row in body["skips"])

    isbn_queries = [call["q"] for call in _FakeClient.calls if call["q"].startswith("isbn:")]
    assert "isbn:9780316556347" in isbn_queries
    assert any("Song of Achilles" in call["q"] for call in _FakeClient.calls)

    shelf = client.get("/api/shelf").json()["items"]
    by_title = {row["book"]["title"]: row for row in shelf}
    assert set(by_title) == {"Circe", "The Song of Achilles"}
    assert by_title["Circe"]["status"] == "want_to_read"
    assert by_title["Circe"]["book"]["ol_work_key"] == "/works/OL1W"
    assert by_title["The Song of Achilles"]["status"] == "currently_reading"
    assert by_title["The Song of Achilles"]["book"]["ol_work_key"] == "/works/OL2W"


def test_import_skips_books_already_on_shelf(client, monkeypatch):
    from app import goodreads

    register(client, "ada")
    client.post(
        "/api/shelf",
        json={
            "ol_work_key": "/works/OL1W",
            "title": "Circe",
            "authors": "Madeline Miller",
            "cover_id": 123,
            "year": 2018,
            "status": "finished",
        },
    )
    _FakeClient.calls = []
    monkeypatch.setattr(goodreads.httpx, "AsyncClient", _FakeClient)

    body = _import(client, FIXTURE.read_bytes()).json()
    assert body["imported"] == 1
    already = [row for row in body["skips"] if row["title"] == "Circe"]
    assert already == [{"title": "Circe", "reason": "Already on your shelf"}]

    shelf = client.get("/api/shelf").json()["items"]
    circe = next(row for row in shelf if row["book"]["title"] == "Circe")
    assert circe["status"] == "finished"


def test_import_stays_on_signed_in_shelf(client, monkeypatch):
    from app import goodreads

    register(client, "ada")
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    _FakeClient.calls = []
    monkeypatch.setattr(goodreads.httpx, "AsyncClient", _FakeClient)

    assert _import(client, FIXTURE.read_bytes()).json()["imported"] == 2
    assert len(client.get("/api/shelf").json()["items"]) == 2
    ada = client.get("/api/shelf", params={"username": "ada"}).json()
    assert ada["items"] == []


def test_import_rejects_non_goodreads_csv(client):
    register(client, "ada")
    response = _import(client, b"hello,world\n1,2\n")
    assert response.status_code == 400
    assert "Goodreads" in response.json()["detail"]


def test_import_rejects_oversized_file(client):
    register(client, "ada")
    raw = b"Title,Exclusive Shelf\n" + (b"x" * (MAX_IMPORT_BYTES + 1))
    response = _import(client, raw)
    assert response.status_code == 400
    assert "too large" in response.json()["detail"]
