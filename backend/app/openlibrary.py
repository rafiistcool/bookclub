"""Open Library lookups beyond search: work details and ISBN resolution.

Everything here is best-effort — a failed upstream call raises HTTPException
502 so the UI can say "try again" instead of rendering half a page.
"""

import asyncio
import json
import logging
import re
from collections import OrderedDict
from dataclasses import dataclass, field
from time import time

import httpx
from fastapi import HTTPException

from app.branding import open_library_ua
from app.config import get_settings

logger = logging.getLogger("bookclub.openlibrary")

WORK_URL = "https://openlibrary.org{key}.json"
SEARCH_URL = "https://openlibrary.org/search.json"
_TIMEOUT = 8.0
_DETAILS_TTL = 18 * 3600.0
_DETAILS_MAX_KEYS = 64
_details_cache: OrderedDict[str, tuple[float, "WorkDetails"]] = OrderedDict()
_details_inflight: dict[str, asyncio.Future["WorkDetails"]] = {}
_ISBN_RE = re.compile(r"^(?:\d{9}[\dXx]|\d{13})$")
MAX_SUBJECTS = 12


def extract_isbn(raw: str) -> str | None:
    """Return a normalized ISBN-10/13, or None if `raw` is not ISBN-shaped."""
    cleaned = re.sub(r"[\s-]", "", raw or "").upper()
    if not _ISBN_RE.fullmatch(cleaned):
        return None
    return cleaned


def positive_cover_id(value: object) -> int | None:
    """Open Library uses -1 / 0 for 'no cover'. Only a positive id is usable."""
    try:
        cover = int(value) if value is not None else None
    except (TypeError, ValueError):
        return None
    return cover if cover is not None and cover > 0 else None


def first_positive_cover(*values: object) -> int | None:
    """First usable cover id, walking lists (work `covers[]`) and scalars."""
    for value in values:
        if isinstance(value, list):
            for item in value:
                cover = positive_cover_id(item)
                if cover is not None:
                    return cover
            continue
        cover = positive_cover_id(value)
        if cover is not None:
            return cover
    return None


@dataclass
class WorkDetails:
    ol_work_key: str
    title: str
    authors: str
    cover_id: int | None
    year: int | None
    description: str
    pages: int | None
    subjects: list[str] = field(default_factory=list)
    ol_rating: float | None = None
    ol_rating_count: int | None = None


@dataclass
class IsbnHit:
    isbn: str
    ol_work_key: str
    title: str
    authors: str
    cover_id: int | None
    year: int | None
    pages: int | None


def clear_details_cache() -> None:
    _details_cache.clear()
    _details_inflight.clear()


def _details_cache_get(key: str) -> WorkDetails | None:
    hit = _details_cache.get(key)
    if hit is None:
        return None
    expires, value = hit
    if expires <= time():
        _details_cache.pop(key, None)
        return None
    _details_cache.move_to_end(key)
    return value


def _details_cache_set(key: str, value: WorkDetails) -> None:
    _details_cache[key] = (time() + _DETAILS_TTL, value)
    _details_cache.move_to_end(key)
    while len(_details_cache) > _DETAILS_MAX_KEYS:
        _details_cache.popitem(last=False)


def normalize_isbn(raw: str) -> str:
    cleaned = re.sub(r"[\s-]", "", raw or "").upper()
    if not _ISBN_RE.fullmatch(cleaned):
        raise HTTPException(status_code=400, detail="That is not a valid ISBN-10 or ISBN-13")
    return cleaned


def _clean_description(value: object) -> str:
    if isinstance(value, dict):
        value = value.get("value", "")
    text = str(value or "").strip()
    # Open Library descriptions often trail with source links in markdown.
    text = re.sub(r"\n+-{3,}[\s\S]*$", "", text)
    text = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", text)
    return text.strip()


def _clean_subjects(values: object) -> list[str]:
    if not isinstance(values, list):
        return []
    seen: set[str] = set()
    out: list[str] = []
    for item in values:
        name = str(item or "").strip()
        if not name or len(name) > 40 or ":" in name or "," in name:
            continue
        key = name.lower()
        if key in seen:
            continue
        seen.add(key)
        out.append(name)
        if len(out) >= MAX_SUBJECTS:
            break
    return out


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


async def _get(client: httpx.AsyncClient, url: str, params: dict | None = None) -> dict:
    response = await client.get(
        url, params=params, headers={"User-Agent": open_library_ua(get_settings())}
    )
    response.raise_for_status()
    payload = response.json()
    return payload if isinstance(payload, dict) else {}


async def _load_work_details(ol_work_key: str) -> WorkDetails:
    search_params = {
        "q": f"key:{ol_work_key}",
        "fields": "key,title,author_name,cover_i,first_publish_year,number_of_pages_median,ratings_average,ratings_count,subject",
        "limit": 1,
    }
    try:
        async with httpx.AsyncClient(timeout=_TIMEOUT) as client:
            work, search = await asyncio.gather(
                _get(client, WORK_URL.format(key=ol_work_key)),
                _get(client, SEARCH_URL, search_params),
            )
    except httpx.HTTPError as exc:
        logger.warning("open library work lookup failed key=%s err=%s", ol_work_key, exc.__class__.__name__)
        raise HTTPException(
            status_code=502, detail="Could not load that book right now. Try again."
        ) from exc

    doc = (search.get("docs") or [{}])[0] if isinstance(search.get("docs"), list) else {}
    subjects = _clean_subjects(work.get("subjects")) or _clean_subjects(doc.get("subject"))
    return WorkDetails(
        ol_work_key=ol_work_key,
        title=str(work.get("title") or doc.get("title") or "").strip(),
        authors=", ".join(str(name) for name in (doc.get("author_name") or []) if name),
        cover_id=first_positive_cover(doc.get("cover_i"), work.get("covers")),
        year=_int(doc.get("first_publish_year")),
        description=_clean_description(work.get("description")),
        pages=_int(doc.get("number_of_pages_median")),
        subjects=subjects,
        ol_rating=_float(doc.get("ratings_average")),
        ol_rating_count=_int(doc.get("ratings_count")),
    )


async def fetch_work_details(ol_work_key: str, *, bypass_cache: bool = False) -> WorkDetails:
    if not bypass_cache:
        cached = _details_cache_get(ol_work_key)
        if cached is not None:
            return cached
        inflight = _details_inflight.get(ol_work_key)
        if inflight is not None:
            return await asyncio.shield(inflight)

    loop = asyncio.get_running_loop()
    pending: asyncio.Future[WorkDetails] = loop.create_future()
    _details_inflight[ol_work_key] = pending
    try:
        details = await _load_work_details(ol_work_key)
        _details_cache_set(ol_work_key, details)
        pending.set_result(details)
        return details
    except Exception as exc:
        if not pending.done():
            pending.set_exception(exc)
        raise
    finally:
        # CancelledError is a BaseException, so the except above misses it.
        # Resolve waiters parked on shield() or they hang after we drop _inflight.
        if not pending.done():
            pending.set_exception(
                HTTPException(
                    status_code=502, detail="Could not load that book right now. Try again."
                )
            )
        if _details_inflight.get(ol_work_key) is pending:
            del _details_inflight[ol_work_key]


async def lookup_isbn(isbn: str) -> IsbnHit | None:
    params = {
        "q": f"isbn:{isbn}",
        "fields": "key,title,author_name,cover_i,first_publish_year,number_of_pages_median",
        "limit": 1,
    }
    try:
        async with httpx.AsyncClient(timeout=_TIMEOUT) as client:
            payload = await _get(client, SEARCH_URL, params)
    except httpx.HTTPError as exc:
        logger.warning("open library isbn lookup failed err=%s", exc.__class__.__name__)
        raise HTTPException(
            status_code=502, detail="Could not look that ISBN up right now. Try again."
        ) from exc
    docs = payload.get("docs") or []
    for doc in docs:
        key = str(doc.get("key") or "")
        title = str(doc.get("title") or "").strip()
        if not key.startswith("/works/") or not title:
            continue
        return IsbnHit(
            isbn=isbn,
            ol_work_key=key,
            title=title,
            authors=", ".join(str(name) for name in (doc.get("author_name") or []) if name),
            cover_id=positive_cover_id(doc.get("cover_i")),
            year=_int(doc.get("first_publish_year")),
            pages=_int(doc.get("number_of_pages_median")),
        )
    return None


def subjects_to_json(subjects: list[str]) -> str:
    return json.dumps(subjects, ensure_ascii=False)


def subjects_from_json(raw: str) -> list[str]:
    try:
        data = json.loads(raw or "[]")
    except json.JSONDecodeError:
        return []
    return [str(item) for item in data] if isinstance(data, list) else []
