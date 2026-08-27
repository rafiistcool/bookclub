import csv
import io
import re

import httpx

from app.branding import open_library_ua
from app.config import get_settings
from app.models import ShelfStatus
from app.routers.books import OPEN_LIBRARY_URL, map_open_library_docs
from app.schemas import SearchHit

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


async def _search_first(query: str) -> SearchHit | None:
    try:
        async with httpx.AsyncClient(timeout=8.0) as client:
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
    hits = map_open_library_docs(payload.get("docs") or [])
    return hits[0] if hits else None


async def lookup_work(isbn: str, title: str, authors: str) -> SearchHit | None:
    queries: list[str] = []
    if isbn:
        queries.append(f"isbn:{isbn}")
    title_author = " ".join(part for part in (title, authors) if part).strip()
    if title_author and title_author not in queries:
        queries.append(title_author)
    for query in queries:
        hit = await _search_first(query)
        if hit is not None:
            return hit
    return None
