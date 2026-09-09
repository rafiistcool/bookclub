import asyncio
import csv
import io
import re

import httpx
from fastapi import HTTPException

from app.branding import open_library_ua
from app.config import get_settings
from app.googlebooks import (
    GoogleBooksError,
    RATE_LIMITED,
    google_books_cooling_down,
    google_books_enabled,
    search_volumes,
)
from app.models import ShelfStatus
from app.openlibrary import OL_TIMEOUT
from app.routers.books import OPEN_LIBRARY_URL, map_open_library_docs
from app.schemas import SearchHit

LOOKUP_CONCURRENCY = 2

MAX_IMPORT_BYTES = 1_500_000
MAX_IMPORT_ROWS = 400
SKIP_LIST_LIMIT = 25

SHELF_MAP = {
    "to-read": ShelfStatus.want_to_read,
    "currently-reading": ShelfStatus.currently_reading,
    "read": ShelfStatus.finished,
}

_ISBN_CHARS = re.compile(r"[^0-9Xx]")


def clean_isbn(value: str) -> str:
    text = (value or "").strip().strip('"').strip("'")
    if text.startswith("="):
        text = text[1:].strip().strip('"').strip("'")
    digits = _ISBN_CHARS.sub("", text)
    if len(digits) in {10, 13}:
        return digits.upper()
    return ""


def _cell(row: dict[str, str], *names: str) -> str:
    for name in names:
        if name in row and row[name] is not None:
            return str(row[name]).strip()
        lowered = {key.lower(): key for key in row}
        key = lowered.get(name.lower())
        if key is not None and row[key] is not None:
            return str(row[key]).strip()
    return ""


class GoodreadsRow:
    def __init__(self, title: str, authors: str, isbn: str, status: ShelfStatus) -> None:
        self.title = title
        self.authors = authors
        self.isbn = isbn
        self.status = status


def parse_goodreads_csv(raw: bytes) -> tuple[list[GoodreadsRow], list[tuple[str, str]]]:
    if len(raw) > MAX_IMPORT_BYTES:
        raise ValueError("That file is too large. Keep the export under 1.5 MB.")
    try:
        text = raw.decode("utf-8-sig")
    except UnicodeDecodeError as exc:
        raise ValueError("The file must be a UTF-8 Goodreads CSV export") from exc
    reader = csv.DictReader(io.StringIO(text))
    if reader.fieldnames is None:
        raise ValueError("That does not look like a Goodreads export")
    headers = {name.strip().lower() for name in reader.fieldnames if name}
    if "exclusive shelf" not in headers or "title" not in headers:
        raise ValueError("That does not look like a Goodreads export")

    rows: list[GoodreadsRow] = []
    skips: list[tuple[str, str]] = []
    for index, raw_row in enumerate(reader):
        if index >= MAX_IMPORT_ROWS:
            skips.append(("", "Stopped after 400 rows"))
            break
        title = _cell(raw_row, "Title")
        authors = _cell(raw_row, "Author")
        shelf = _cell(raw_row, "Exclusive Shelf").lower()
        isbn = clean_isbn(_cell(raw_row, "ISBN13")) or clean_isbn(_cell(raw_row, "ISBN"))
        label = title or "Untitled"
        if not title:
            skips.append((label, "No title"))
            continue
        status = SHELF_MAP.get(shelf)
        if status is None:
            skips.append((label, "Unknown shelf"))
            continue
        rows.append(GoodreadsRow(title=title, authors=authors, isbn=isbn, status=status))
    return rows, skips


async def _search_first(
    query: str, client: httpx.AsyncClient | None = None
) -> SearchHit | None:
    own = False
    if client is None:
        client = httpx.AsyncClient(timeout=OL_TIMEOUT)
        own = True
    try:
        response = await client.get(
            OPEN_LIBRARY_URL,
            params={
                "q": query,
                "limit": 5,
                "fields": "key,title,author_name,cover_i,first_publish_year",
            },
            headers={"User-Agent": open_library_ua(get_settings())},
        )
        response.raise_for_status()
        payload = response.json()
    except (httpx.HTTPError, ValueError, TypeError):
        return None
    finally:
        if own:
            await client.aclose()
    hits = map_open_library_docs(payload.get("docs") or [])
    return hits[0] if hits else None


async def _search_google(query: str) -> SearchHit | None:
    items, _total = await search_volumes(query, page=1, limit=5)
    return items[0] if items else None


async def lookup_work(
    isbn: str,
    title: str,
    authors: str,
    client: httpx.AsyncClient | None = None,
) -> SearchHit | None:
    queries: list[str] = []
    if isbn:
        queries.append(f"isbn:{isbn}")
    title_author = " ".join(part for part in (title, authors) if part).strip()
    if title_author and title_author not in queries:
        queries.append(title_author)
    if google_books_enabled():
        for query in queries:
            hit = await _search_google(query)
            if hit is not None:
                return hit
        return None
    for query in queries:
        hit = await _search_first(query, client)
        if hit is not None:
            return hit
    return None


async def lookup_catalog(
    rows: list[GoodreadsRow],
) -> list[tuple[GoodreadsRow, SearchHit | None, str | None]]:
    """Resolve each CSV row against the configured catalog, two lookups at a time."""
    if not rows:
        return []
    if google_books_enabled() and google_books_cooling_down():
        raise HTTPException(status_code=429, detail=RATE_LIMITED)
    sem = asyncio.Semaphore(LOOKUP_CONCURRENCY)

    async def one(
        row: GoodreadsRow, client: httpx.AsyncClient | None
    ) -> tuple[GoodreadsRow, SearchHit | None, str | None]:
        async with sem:
            try:
                hit = await lookup_work(row.isbn, row.title, row.authors, client)
            except GoogleBooksError:
                return row, None, "Catalog unavailable"
            return row, hit, None if hit else "No match"

    if google_books_enabled():
        return list(await asyncio.gather(*(one(row, None) for row in rows)))

    async with httpx.AsyncClient(timeout=OL_TIMEOUT) as client:
        return list(await asyncio.gather(*(one(row, client) for row in rows)))
