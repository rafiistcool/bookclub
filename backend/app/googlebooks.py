"""Optional Google Books catalog: search, volume get, and ISBN lookup.

Used when `GOOGLE_BOOKS_API_KEY` is set. Failures are signaled as
`GoogleBooksError` so callers can return a classified error. When the key
is set, search / browse / ISBN / Goodreads stay on Google — they do not
fall back to Open Library on a miss. Never required for a working club —
empty key means this module is not called.
"""

from __future__ import annotations

import asyncio
import logging
import re
from collections import OrderedDict
from dataclasses import dataclass, field
from html import unescape
from time import monotonic, time

import httpx
from fastapi import HTTPException

from app.branding import google_books_ua
from app.config import get_settings
from app.covers import cover_url_from_image_links
from app.openlibrary import extract_isbn
from app.schemas import SearchHit
from app.works import google_catalog_work_id, work_key

logger = logging.getLogger("bookclub.googlebooks")

SEARCH_URL = "https://www.googleapis.com/books/v1/volumes"
VOLUME_URL = "https://www.googleapis.com/books/v1/volumes/{volume_id}"
GB_TIMEOUT = httpx.Timeout(connect=5.0, read=10.0, write=5.0, pool=5.0)
_CACHE_TTL = 18 * 3600.0
_CACHE_MAX_KEYS = 256
_cache: OrderedDict[str, tuple[float, dict]] = OrderedDict()
_inflight: dict[str, asyncio.Future[dict]] = {}
_TAG_RE = re.compile(r"<[^>]+>")
_BR_RE = re.compile(r"<br\s*/?>", re.IGNORECASE)
_P_CLOSE_RE = re.compile(r"</p>", re.IGNORECASE)
_BLANK_LINES_RE = re.compile(r"\n{3,}")
VOLUME_FIELDS = (
    "id,volumeInfo(title,authors,publishedDate,description,pageCount,"
    "categories,averageRating,ratingsCount,industryIdentifiers,imageLinks)"
)
SEARCH_FIELDS = f"totalItems,items({VOLUME_FIELDS})"
RATE_LIMITED = "The library is busy. Wait a few seconds and try again."
_COOLDOWN_SEC = 120.0
_cooldown_until = 0.0


class GoogleBooksError(Exception):
    """Upstream Google Books failed (transport, 429, or 5xx)."""

    def __init__(self, message: str = "google books unavailable", status: int | None = None):
        super().__init__(message)
        self.status = status


@dataclass
class VolumeDetails:
    work_key: str
    volume_id: str
    title: str
    authors: str
    year: int | None
    description: str | None = None
    pages: int | None = None
    subjects: list[str] | None = None
    isbn: str | None = None
    rating: float | None = None
    rating_count: int | None = None
    cover_id: int | None = None
    cover_image_url: str | None = None


def google_books_api_key() -> str:
    return (get_settings().google_books_api_key or "").strip()


def google_books_enabled() -> bool:
    return bool(google_books_api_key())


def clear_google_cache() -> None:
    global _cooldown_until
    _cache.clear()
    _inflight.clear()
    _cooldown_until = 0.0


def google_books_cooling_down() -> bool:
    return time() < _cooldown_until


def _trip_cooldown(status: int | None) -> None:
    global _cooldown_until
    if status in {429, 403}:
        _cooldown_until = time() + _COOLDOWN_SEC


def _cache_get(key: str) -> dict | None:
    hit = _cache.get(key)
    if hit is None:
        return None
    expires, value = hit
    if expires <= time():
        _cache.pop(key, None)
        return None
    _cache.move_to_end(key)
    return value


def _cache_set(key: str, value: dict) -> None:
    now = time()
    expired = [cached_key for cached_key, (expires, _) in _cache.items() if expires <= now]
    for cached_key in expired:
        _cache.pop(cached_key, None)
    _cache[key] = (now + _CACHE_TTL, value)
    _cache.move_to_end(key)
    while len(_cache) > _CACHE_MAX_KEYS:
        _cache.popitem(last=False)


def _year(value: object) -> int | None:
    text = str(value or "").strip()
    if len(text) < 4 or not text[:4].isdigit():
        return None
    year = int(text[:4])
    return year if 1 <= year <= 3000 else None


def _int(value: object) -> int | None:
    try:
        return int(value) if value is not None else None
    except (TypeError, ValueError):
        return None


def _float(value: object) -> float | None:
    try:
        return round(float(value), 2) if value is not None else None
    except (TypeError, ValueError):
        return None


def _volume_isbn(info: dict) -> str | None:
    found_10: str | None = None
    for entry in info.get("industryIdentifiers") or []:
        if not isinstance(entry, dict):
            continue
        kind = str(entry.get("type") or "")
        isbn = extract_isbn(str(entry.get("identifier") or ""))
        if not isbn:
            continue
        if kind == "ISBN_13" or len(isbn) == 13:
            return isbn
        if kind == "ISBN_10" or len(isbn) == 10:
            found_10 = isbn
    return found_10


def _clean_description(value: object) -> str:
    text = unescape(str(value or ""))
    text = _BR_RE.sub("\n", text)
    text = _P_CLOSE_RE.sub("\n\n", text)
    text = _TAG_RE.sub("", text)
    text = _BLANK_LINES_RE.sub("\n\n", text)
    return text.strip()


def _clean_subjects(values: object, limit: int = 12) -> list[str]:
    if not isinstance(values, list):
        return []
    seen: set[str] = set()
    out: list[str] = []
    for item in values:
        name = str(item or "").strip()
        if not name or len(name) > 40:
            continue
        key = name.lower()
        if key in seen:
            continue
        seen.add(key)
        out.append(name)
        if len(out) >= limit:
            break
    return out


def _authors(info: dict) -> str:
    names = info.get("authors") or []
    if not isinstance(names, list):
        return ""
    return ", ".join(str(name) for name in names if name)


def map_volume(volume: dict) -> SearchHit | None:
    if not isinstance(volume, dict):
        return None
    volume_id = str(volume.get("id") or "").strip()
    info = volume.get("volumeInfo")
    if not isinstance(info, dict):
        return None
    title = str(info.get("title") or "").strip()
    isbn = _volume_isbn(info)
    work_id = google_catalog_work_id(isbn=isbn, volume_id=volume_id)
    if not title or work_id is None:
        return None
    cover = cover_url_from_image_links(info.get("imageLinks"))
    return SearchHit(
        ol_work_key=work_key(work_id),
        title=title,
        authors=_authors(info),
        cover_id=None,
        year=_year(info.get("publishedDate")),
        isbn=isbn,
        cover_url=cover,
    )


def map_volumes(items: list[dict]) -> list[SearchHit]:
    hits: list[SearchHit] = []
    seen: set[str] = set()
    for item in items:
        hit = map_volume(item)
        if hit is None or hit.ol_work_key in seen:
            continue
        hits.append(hit)
        seen.add(hit.ol_work_key)
    return hits


def volume_details(volume: dict) -> VolumeDetails | None:
    hit = map_volume(volume)
    if hit is None:
        return None
    info = volume.get("volumeInfo")
    if not isinstance(info, dict):
        return None
    volume_id = str(volume.get("id") or "").strip()
    return VolumeDetails(
        work_key=hit.ol_work_key,
        volume_id=volume_id,
        title=hit.title,
        authors=hit.authors,
        year=hit.year,
        description=_clean_description(info.get("description")),
        pages=_int(info.get("pageCount")),
        subjects=_clean_subjects(info.get("categories")),
        isbn=hit.isbn,
        rating=_float(info.get("averageRating")),
        rating_count=_int(info.get("ratingsCount")),
        cover_image_url=hit.cover_url,
    )


def google_search_query(query: str, subject: str = "") -> str:
    isbn = extract_isbn(query)
    if isbn:
        return f"isbn:{isbn}"
    parts: list[str] = []
    if query:
        parts.append(query)
    if subject:
        label = subject.replace("_", " ")
        parts.append(f'subject:"{label}"')
    return " ".join(parts)


def _google_headers() -> dict[str, str]:
    key = google_books_api_key()
    if not key:
        raise GoogleBooksError("google books key missing")
    return {
        "User-Agent": google_books_ua(get_settings()),
        "X-Goog-Api-Key": key,
    }


async def _fetch_json_uncached(url: str, params: dict[str, str | int]) -> dict:
    if google_books_cooling_down():
        raise GoogleBooksError(status=429)
    started = monotonic()
    try:
        async with httpx.AsyncClient(timeout=GB_TIMEOUT) as client:
            response = await client.get(
                url,
                params=params,
                headers=_google_headers(),
            )
        status = response.status_code
        if status >= 400:
            _trip_cooldown(status)
            elapsed_ms = int((monotonic() - started) * 1000)
            logger.warning(
                "google books request failed url=%s status=%s latency_ms=%s",
                url,
                status,
                elapsed_ms,
            )
            raise GoogleBooksError(status=status)
        payload = response.json()
    except GoogleBooksError:
        raise
    except httpx.HTTPError as exc:
        elapsed_ms = int((monotonic() - started) * 1000)
        status = getattr(getattr(exc, "response", None), "status_code", None)
        _trip_cooldown(status)
        logger.warning(
            "google books request failed url=%s status=%s latency_ms=%s err=%s",
            url,
            status,
            elapsed_ms,
            exc.__class__.__name__,
        )
        raise GoogleBooksError(status=status) from exc
    except ValueError as exc:
        raise GoogleBooksError() from exc

    if not isinstance(payload, dict):
        raise GoogleBooksError()
    logger.info(
        "google books ok url=%s latency_ms=%s",
        url,
        int((monotonic() - started) * 1000),
    )
    return payload


def _empty_volume_items(payload: dict) -> bool:
    items = payload.get("items")
    return isinstance(items, list) and len(items) == 0


async def _fetch_json(
    url: str,
    *,
    cache_key: str,
    params: dict[str, str | int],
    bypass_cache: bool = False,
    cache_empty: bool = True,
) -> dict:
    if not bypass_cache:
        cached = _cache_get(cache_key)
        if cached is not None:
            return cached
        inflight = _inflight.get(cache_key)
        if inflight is not None:
            return await asyncio.shield(inflight)

    loop = asyncio.get_running_loop()
    pending: asyncio.Future[dict] = loop.create_future()
    _inflight[cache_key] = pending
    try:
        payload = await _fetch_json_uncached(url, params)
        if cache_empty or not _empty_volume_items(payload):
            _cache_set(cache_key, payload)
        pending.set_result(payload)
        return payload
    except Exception as exc:
        if not pending.done():
            pending.set_exception(exc)
        raise
    finally:
        if not pending.done():
            pending.set_exception(GoogleBooksError())
        if _inflight.get(cache_key) is pending:
            del _inflight[cache_key]


async def search_volumes(
    query: str,
    *,
    subject: str = "",
    sort: str = "relevance",
    page: int = 1,
    limit: int = 24,
) -> tuple[list[SearchHit], int]:
    q = google_search_query(query, subject)
    if not q:
        return [], 0
    params: dict[str, str | int] = {
        "q": q,
        "maxResults": limit,
        "startIndex": (page - 1) * limit,
        "fields": SEARCH_FIELDS,
    }
    if sort == "new":
        params["orderBy"] = "newest"
    cache_key = f"gb|search|{q.casefold()}|{sort}|{page}|{limit}"
    payload = await _fetch_json(
        SEARCH_URL,
        cache_key=cache_key,
        params=params,
        cache_empty=False,
    )
    items = payload.get("items") or []
    if not isinstance(items, list):
        items = []
    try:
        total = int(payload.get("totalItems") or 0)
    except (TypeError, ValueError):
        total = 0
    return map_volumes([item for item in items if isinstance(item, dict)]), total


async def fetch_volume(volume_id: str, *, bypass_cache: bool = False) -> VolumeDetails:
    params: dict[str, str | int] = {"fields": VOLUME_FIELDS}
    cache_key = f"gb|volume|{volume_id}"
    try:
        payload = await _fetch_json(
            VOLUME_URL.format(volume_id=volume_id),
            cache_key=cache_key,
            params=params,
            bypass_cache=bypass_cache,
        )
    except GoogleBooksError as exc:
        if exc.status == 404:
            raise HTTPException(status_code=404, detail="No such book.") from exc
        if exc.status == 429:
            raise HTTPException(status_code=429, detail=RATE_LIMITED) from exc
        raise HTTPException(
            status_code=502, detail="Could not load that book right now. Try again."
        ) from exc
    details = volume_details(payload)
    if details is None:
        raise HTTPException(status_code=404, detail="No such book.")
    return details


async def lookup_isbn(isbn: str, *, bypass_cache: bool = False) -> VolumeDetails | None:
    params: dict[str, str | int] = {
        "q": f"isbn:{isbn}",
        "maxResults": 1,
        "fields": SEARCH_FIELDS,
    }
    cache_key = f"gb|isbn|{isbn}"
    payload = await _fetch_json(
        SEARCH_URL,
        cache_key=cache_key,
        params=params,
        bypass_cache=bypass_cache,
        cache_empty=False,
    )
    items = payload.get("items") or []
    if not isinstance(items, list) or not items:
        return None
    first = items[0]
    if not isinstance(first, dict):
        return None
    return volume_details(first)
