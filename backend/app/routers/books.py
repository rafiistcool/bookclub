import asyncio
import logging
import re
from collections import OrderedDict
from time import monotonic, time
from typing import Any, Literal

import httpx
from fastapi import APIRouter, Depends, HTTPException, Path, Query
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app.branding import open_library_ua
from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import Book, ClubPick, Quote, ShelfEntry, ShelfStatus, User
from app.googlebooks import (
    GoogleBooksError,
    VolumeDetails,
    fetch_volume,
    google_books_enabled,
    lookup_isbn as lookup_isbn_google,
    search_volumes,
)
from app.openlibrary import (
    OL_TIMEOUT,
    WorkDetails,
    extract_isbn,
    fetch_work_details,
    first_positive_cover,
    lookup_isbn,
    normalize_isbn,
    positive_cover_id,
    subjects_from_json,
)
from app.pick_ops import current_pick
from app.schemas import (
    BookDetailOut,
    BookDetailsOut,
    BookMember,
    BookReader,
    CustomBookIn,
    IsbnHitOut,
    SearchHit,
    SearchPage,
)
from app.serialize import book_cover_url
from app.shelf_ops import apply_book_details, upsert_book
from app.works import (
    canonical_work_id,
    google_volume_id,
    isbn_from_work_id,
    isbn_from_work_key,
    is_club_work_id,
    is_club_work_key,
    is_google_catalog_id,
    is_google_work_id,
    is_open_library_work_id,
    is_work_id,
    new_club_work_id,
    work_key,
)

router = APIRouter(prefix="/api/books", tags=["books"])
logger = logging.getLogger("bookclub.search")

_SUBJECT_KEY_RE = re.compile(r"^[a-z0-9_]+$")
_AUTHOR_ID_RE = re.compile(r"^OL\d+A$")
# Markdown leftovers in Open Library descriptions, which we render as text.
_REF_DEFINITION_RE = re.compile(r"^[ \t]*\[[^\]]+\]:[ \t]*\S+.*$", re.MULTILINE)
_INLINE_LINK_RE = re.compile(r"\[([^\]]+)\]\([^)]*\)")
_REF_LINK_RE = re.compile(r"\[([^\]]+)\]\[[^\]]*\]")
_BLOCKQUOTE_RE = re.compile(r"^[ \t]*>[ \t]?", re.MULTILINE)
_BLANK_LINES_RE = re.compile(r"\n{3,}")
# Open Library mixes machine tags like "award:hugo_award=1970" into subjects.
_MACHINE_TAG_RE = re.compile(r"^[a-z_]+:\S")
# Club-scale RAM cache: identical searches stay warm for a day, and a key cap
# keeps the process bounded on a shared 16GB host.
_CACHE_TTL = 18 * 3600.0
_CACHE_MAX_KEYS = 256
_cache: OrderedDict[str, tuple[float, Any]] = OrderedDict()
_inflight: dict[str, asyncio.Future[dict]] = {}
# Connect is shorter than the old 10s so a dead hop fails faster, but long
# enough for a slow TLS handshake to Open Library (~4s measured). Read stays
# long because trending/subjects regularly take several seconds.
_TIMEOUT = OL_TIMEOUT

OPEN_LIBRARY_URL = "https://openlibrary.org/search.json"
TRENDING_URL = "https://openlibrary.org/trending/daily.json"
SUBJECT_URL = "https://openlibrary.org/subjects/{subject}.json"
WORK_URL = "https://openlibrary.org/works/{work_id}.json"
AUTHOR_URL = "https://openlibrary.org/authors/{author_id}.json"

# isbn is omitted: OL returns every edition ISBN and ~10× the payload.
# cover_edition_key already covers the /b/olid/ fallback.
SEARCH_FIELDS = "key,title,author_name,cover_i,first_publish_year,cover_edition_key"
WORK_SEARCH_FIELDS = (
    "key,title,author_name,cover_i,first_publish_year,"
    "number_of_pages_median,ratings_average,ratings_count,subject"
)
SEARCH_UNAVAILABLE = "Could not search the library right now. Try again."
BROWSE_UNAVAILABLE = "Could not load the library right now. Try again."
QUERY_TOO_SHORT = (
    "Need at least 3 characters. Add the author, or paste an ISBN."
)
RATE_LIMITED = "The library is busy. Wait a few seconds and try again."
CUSTOM_NO_REFRESH = "This book was added by the club, not Open Library."
GOOGLE_NO_REFRESH = (
    "This book was added from Google Books. Set GOOGLE_BOOKS_API_KEY to refresh."
)
# Open Library rejects any q shorter than 3 characters, so an empty search box
# cannot be expressed as a search query at all. Browse falls back to trending.
MAX_DETAIL_AUTHORS = 3
_TITLE_BY_AUTHOR_RE = re.compile(r"^(.+?)\s+by\s+(.+)$", re.IGNORECASE)

Sort = Literal["readinglog", "new", "title", "relevance"]


def clear_search_cache() -> None:
    _cache.clear()
    _inflight.clear()


def normalize_search_query(query: str) -> str:
    """Collapse whitespace and case so identical searches share a cache key."""
    return " ".join(query.casefold().split())


def _cache_get(key: str) -> Any | None:
    hit = _cache.get(key)
    if hit is None:
        return None
    expires, value = hit
    if expires <= time():
        _cache.pop(key, None)
        return None
    _cache.move_to_end(key)
    return value


def _cache_set(key: str, value: Any) -> None:
    now = time()
    expired = [cached_key for cached_key, (expires, _) in _cache.items() if expires <= now]
    for cached_key in expired:
        _cache.pop(cached_key, None)
    _cache[key] = (now + _CACHE_TTL, value)
    _cache.move_to_end(key)
    while len(_cache) > _CACHE_MAX_KEYS:
        _cache.popitem(last=False)


def _retry_after_detail(response: httpx.Response) -> str:
    raw = response.headers.get("Retry-After")
    try:
        seconds = int(raw) if raw is not None else 0
    except (TypeError, ValueError):
        seconds = 0
    if seconds > 0:
        return f"The library is busy. Try again in {seconds} seconds."
    return RATE_LIMITED


def _classify_open_library_status(
    status: int,
    *,
    detail: str,
    missing_detail: str | None,
    response: httpx.Response | None = None,
) -> HTTPException:
    if status == 422:
        return HTTPException(status_code=400, detail=QUERY_TOO_SHORT)
    if status == 429:
        hint = _retry_after_detail(response) if response is not None else RATE_LIMITED
        return HTTPException(status_code=429, detail=hint)
    if status == 404 and missing_detail is not None:
        return HTTPException(status_code=404, detail=missing_detail)
    return HTTPException(status_code=502, detail=detail)


async def _fetch_json_uncached(
    url: str,
    *,
    detail: str,
    params: dict[str, str | int] | None = None,
    missing_detail: str | None = None,
) -> dict:
    started = monotonic()
    try:
        async with httpx.AsyncClient(timeout=_TIMEOUT) as client:
            response = await client.get(
                url,
                params=params,
                headers={"User-Agent": open_library_ua(get_settings())},
            )
        status = response.status_code
        if status >= 400:
            elapsed_ms = int((monotonic() - started) * 1000)
            logger.warning(
                "open library request failed url=%s status=%s latency_ms=%s err=HTTPStatus",
                url,
                status,
                elapsed_ms,
            )
            raise _classify_open_library_status(
                status,
                detail=detail,
                missing_detail=missing_detail,
                response=response,
            )
        payload = response.json()
    except HTTPException:
        raise
    except httpx.HTTPError as exc:
        elapsed_ms = int((monotonic() - started) * 1000)
        status = getattr(getattr(exc, "response", None), "status_code", None)
        logger.warning(
            "open library request failed url=%s status=%s latency_ms=%s err=%s",
            url,
            status,
            elapsed_ms,
            exc.__class__.__name__,
        )
        raise HTTPException(status_code=502, detail=detail) from exc

    if not isinstance(payload, dict):
        logger.warning("open library returned a non-object payload url=%s", url)
        raise HTTPException(status_code=502, detail=detail)

    logger.info(
        "open library ok url=%s latency_ms=%s",
        url,
        int((monotonic() - started) * 1000),
    )
    return payload


def _empty_search_docs(payload: dict) -> bool:
    docs = payload.get("docs")
    return isinstance(docs, list) and len(docs) == 0


async def _fetch_json(
    url: str,
    *,
    cache_key: str,
    detail: str,
    params: dict[str, str | int] | None = None,
    missing_detail: str | None = None,
    bypass_cache: bool = False,
    cache_empty: bool = True,
) -> dict:
    """GET JSON from Open Library, cached by `cache_key`.

    Identical in-flight requests share one upstream call. Raises 502 on
    transport or status errors, or 404 when `missing_detail` is set and Open
    Library reports the resource does not exist. Successful empty `search.json`
    pages are not cached when `cache_empty` is false (a lucky-empty 200 must
    not freeze for 18 hours).
    """
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
        payload = await _fetch_json_uncached(
            url, detail=detail, params=params, missing_detail=missing_detail
        )
        if cache_empty or not _empty_search_docs(payload):
            _cache_set(cache_key, payload)
        pending.set_result(payload)
        return payload
    except Exception as exc:
        if not pending.done():
            pending.set_exception(exc)
        raise
    finally:
        # CancelledError is a BaseException, so the except above misses it.
        # Resolve waiters parked on shield() or they hang after we drop _inflight.
        if not pending.done():
            pending.set_exception(HTTPException(status_code=502, detail=detail))
        if _inflight.get(cache_key) is pending:
            del _inflight[cache_key]


def _cover_id(value: Any) -> int | None:
    return positive_cover_id(value)


def _doc_isbn(doc: dict) -> str | None:
    raw = doc.get("isbn")
    if isinstance(raw, str):
        return extract_isbn(raw)
    if not isinstance(raw, list):
        return None
    for item in raw:
        isbn = extract_isbn(str(item or ""))
        if isbn:
            return isbn
    return None


def _doc_edition_key(doc: dict) -> str | None:
    raw = str(doc.get("cover_edition_key") or "").strip()
    return raw or None


def _year(value: Any) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _rating(value: Any) -> float | None:
    try:
        return round(float(value), 2) if value is not None else None
    except (TypeError, ValueError):
        return None


def map_open_library_docs(docs: list[dict]) -> list[SearchHit]:
    """Map `search.json` docs, and `trending/*.json` works which share the shape."""
    hits: list[SearchHit] = []
    seen: set[str] = set()
    for doc in docs:
        if not isinstance(doc, dict):
            continue
        key = str(doc.get("key") or "")
        if not key.startswith("/works/") or key in seen:
            continue
        title = str(doc.get("title") or "").strip()
        if not title:
            continue
        authors = ", ".join(str(name) for name in (doc.get("author_name") or []) if name)
        hits.append(
            SearchHit(
                ol_work_key=key,
                title=title,
                authors=authors,
                cover_id=_cover_id(doc.get("cover_i")),
                year=_year(doc.get("first_publish_year")),
                on_shelf=None,
                cover_edition_key=_doc_edition_key(doc),
                isbn=_doc_isbn(doc),
            )
        )
        seen.add(key)
    return hits


def map_subject_works(works: list[dict]) -> list[SearchHit]:
    """Map `subjects/*.json` works, which use cover_id and authors[].name."""
    hits: list[SearchHit] = []
    seen: set[str] = set()
    for work in works:
        if not isinstance(work, dict):
            continue
        key = str(work.get("key") or "")
        if not key.startswith("/works/") or key in seen:
            continue
        title = str(work.get("title") or "").strip()
        if not title:
            continue
        authors = ", ".join(
            str(entry.get("name"))
            for entry in (work.get("authors") or [])
            if isinstance(entry, dict) and entry.get("name")
        )
        hits.append(
            SearchHit(
                ol_work_key=key,
                title=title,
                authors=authors,
                cover_id=_cover_id(work.get("cover_id")),
                year=_year(work.get("first_publish_year")),
                on_shelf=None,
            )
        )
        seen.add(key)
    return hits


def _normalize_subject(subject: str) -> str:
    key = "_".join(subject.lower().split())
    if not key or not _SUBJECT_KEY_RE.fullmatch(key):
        return ""
    return key


def _open_library_q(query: str, subject: str) -> str:
    if query and subject:
        return f"{query} subject_key:{subject}"
    if subject:
        return f"subject_key:{subject}"
    return query


def _search_candidates(query: str, subject: str) -> list[str]:
    """Primary OL `q` plus title/author fallbacks for a true miss."""
    isbn = extract_isbn(query)
    if isbn:
        return [f"isbn:{isbn}"]
    if not query:
        return [_open_library_q(query, subject)]
    primary = _open_library_q(query, subject)
    by_match = _TITLE_BY_AUTHOR_RE.fullmatch(query)
    if by_match:
        title, author = by_match.group(1).strip(), by_match.group(2).strip()
        fielded = f'title:"{title}" author:"{author}"'
        if subject:
            fielded = f"{fielded} subject_key:{subject}"
        # Raw query first. "Stand by Me" must not become title:"Stand" author:"Me"
        # before the plain q, or junk fielded hits short-circuit the real title.
        return [primary, fielded]
    title_q = f"title:{query}"
    author_q = f"author:{query}"
    if subject:
        title_q = f"{title_q} subject_key:{subject}"
        author_q = f"{author_q} subject_key:{subject}"
    return [primary, title_q, author_q]


def _num_found(payload: dict) -> int:
    raw = payload.get("num_found")
    if raw is None:
        raw = payload.get("numFound")
    try:
        return int(raw or 0)
    except (TypeError, ValueError):
        return 0


async def _search_ol_page(
    q: str,
    *,
    sort: Sort,
    page: int,
    limit: int,
    lang_en: bool,
) -> tuple[list[SearchHit], int]:
    params: dict[str, str | int] = {
        "q": q,
        "page": page,
        "limit": limit,
        "fields": SEARCH_FIELDS,
    }
    if lang_en:
        params["lang"] = "en"
    if sort != "relevance":
        params["sort"] = sort
    cache_key = f"ol|search|{normalize_search_query(q)}|{sort}|{page}|{limit}"
    payload = await _fetch_json(
        OPEN_LIBRARY_URL,
        cache_key=cache_key,
        detail=SEARCH_UNAVAILABLE,
        params=params,
        cache_empty=False,
    )
    return map_open_library_docs(payload.get("docs") or []), _num_found(payload)


async def fetch_open_library(
    query: str,
    *,
    subject: str,
    sort: Sort,
    page: int,
    limit: int,
) -> SearchPage:
    candidates = _search_candidates(query, subject)
    lang_en = not query
    primary, *fallbacks = candidates
    items, num_found = await _search_ol_page(
        primary, sort=sort, page=page, limit=limit, lang_en=lang_en
    )
    if not items and page == 1 and fallbacks:
        # Title and author retries are independent; run them together after a
        # true miss so an obscure query does not pay two serial OL waits.
        results = await asyncio.gather(
            *[
                _search_ol_page(q, sort=sort, page=1, limit=limit, lang_en=lang_en)
                for q in fallbacks
            ]
        )
        for batch, found in results:
            if batch:
                items = batch
                num_found = found
                break
    return SearchPage(
        items=items,
        page=page,
        has_more=page * limit < num_found,
    )


def _google_http_error(exc: GoogleBooksError, *, detail: str) -> HTTPException:
    if exc.status in {429, 403}:
        return HTTPException(status_code=429, detail=RATE_LIMITED)
    return HTTPException(status_code=502, detail=detail)


def _google_sort(sort: Sort) -> str:
    return "new" if sort == "new" else "relevance"


async def fetch_google_catalog(
    query: str,
    *,
    subject: str,
    sort: Sort,
    page: int,
    limit: int,
    detail: str = SEARCH_UNAVAILABLE,
) -> SearchPage:
    """Google Books search/browse. Empty miss stays empty — no Open Library."""
    try:
        items, total = await search_volumes(
            query, subject=subject, sort=_google_sort(sort), page=page, limit=limit
        )
    except GoogleBooksError as exc:
        raise _google_http_error(exc, detail=detail) from exc
    return SearchPage(items=items, page=page, has_more=page * limit < total)


async def fetch_catalog(
    query: str,
    *,
    subject: str,
    sort: Sort,
    page: int,
    limit: int,
) -> SearchPage:
    """Google Books when a key is set; Open Library otherwise.

    With a key: empty Google results stay empty (add-your-own). Hard Google
    failures (5xx / timeout / 429) return a classified error. Title and
    Popular map to Google relevance; New maps to newest. No Open Library
    fallback.
    """
    if google_books_enabled():
        return await fetch_google_catalog(
            query, subject=subject, sort=sort, page=page, limit=limit
        )
    return await fetch_open_library(
        query, subject=subject, sort=sort, page=page, limit=limit
    )


async def fetch_trending(limit: int) -> SearchPage:
    if google_books_enabled():
        return await fetch_google_catalog(
            "",
            subject="fiction",
            sort="new",
            page=1,
            limit=limit,
            detail=BROWSE_UNAVAILABLE,
        )
    payload = await _fetch_json(
        TRENDING_URL,
        cache_key=f"trending|{limit}",
        detail=BROWSE_UNAVAILABLE,
        params={"limit": limit},
    )
    return SearchPage(
        items=map_open_library_docs(payload.get("works") or []),
        page=1,
        has_more=False,
    )


async def fetch_subject(subject: str, *, page: int, limit: int) -> SearchPage:
    if google_books_enabled():
        return await fetch_google_catalog(
            "",
            subject=subject,
            sort="relevance",
            page=page,
            limit=limit,
            detail=BROWSE_UNAVAILABLE,
        )
    offset = (page - 1) * limit
    payload = await _fetch_json(
        SUBJECT_URL.format(subject=subject),
        cache_key=f"subject|{subject}|{page}|{limit}",
        detail=BROWSE_UNAVAILABLE,
        params={"limit": limit, "offset": offset, "details": "false"},
        missing_detail="No such subject.",
    )
    try:
        work_count = int(payload.get("work_count") or 0)
    except (TypeError, ValueError):
        work_count = 0
    return SearchPage(
        items=map_subject_works(payload.get("works") or []),
        page=page,
        has_more=offset + limit < work_count,
    )


def _clean_description(text: str) -> str:
    """Flatten Open Library's Markdown-ish descriptions to plain paragraphs.

    Descriptions are author-entered Markdown that we render as text, so link
    syntax, blockquote markers, and trailing reference definitions would
    otherwise show up verbatim.
    """
    text = _REF_DEFINITION_RE.sub("", text)
    text = _INLINE_LINK_RE.sub(r"\1", text)
    text = _REF_LINK_RE.sub(r"\1", text)
    text = _BLOCKQUOTE_RE.sub("", text)
    text = _BLANK_LINES_RE.sub("\n\n", text)
    return text.strip()


def _work_description(payload: dict) -> str:
    raw = payload.get("description")
    if isinstance(raw, dict):
        raw = raw.get("value")
    if not isinstance(raw, str):
        return ""
    return _clean_description(raw)


def _work_subjects(payload: dict, limit: int = 12) -> list[str]:
    subjects: list[str] = []
    seen: set[str] = set()
    for entry in payload.get("subjects") or []:
        if not isinstance(entry, str):
            continue
        label = entry.strip()
        folded = label.lower()
        if not label or folded in seen or _MACHINE_TAG_RE.match(label):
            continue
        subjects.append(label)
        seen.add(folded)
        if len(subjects) >= limit:
            break
    return subjects


def _author_ids(payload: dict) -> list[str]:
    ids: list[str] = []
    for entry in payload.get("authors") or []:
        if not isinstance(entry, dict):
            continue
        ref = entry.get("author")
        key = ref.get("key") if isinstance(ref, dict) else entry.get("key")
        if not isinstance(key, str):
            continue
        author_id = key.rsplit("/", 1)[-1]
        if _AUTHOR_ID_RE.fullmatch(author_id) and author_id not in ids:
            ids.append(author_id)
        if len(ids) >= MAX_DETAIL_AUTHORS:
            break
    return ids


async def _author_name(author_id: str, *, bypass_cache: bool = False) -> str:
    try:
        author = await _fetch_json(
            AUTHOR_URL.format(author_id=author_id),
            cache_key=f"author|{author_id}",
            detail=BROWSE_UNAVAILABLE,
            bypass_cache=bypass_cache,
        )
    except HTTPException:
        return ""
    name = author.get("name")
    return name.strip() if isinstance(name, str) else ""


async def _author_names(payload: dict, *, bypass_cache: bool = False) -> str:
    """Resolve work author refs to names. Best effort: refs carry no names."""
    author_ids = _author_ids(payload)
    if not author_ids:
        return ""
    names = await asyncio.gather(
        *(_author_name(one, bypass_cache=bypass_cache) for one in author_ids)
    )
    return ", ".join(name for name in names if name)


def _annotate_shelf(hits: list[SearchHit], user: User, session: Session) -> None:
    if not hits:
        return
    keys = [hit.ol_work_key for hit in hits]
    books = session.exec(select(Book).where(col(Book.ol_work_key).in_(keys))).all()
    book_ids = {book.ol_work_key: book.id for book in books if book.id is not None}
    if not book_ids:
        return
    entries = session.exec(
        select(ShelfEntry).where(
            ShelfEntry.user_id == user.id,
            col(ShelfEntry.book_id).in_(list(book_ids.values())),
        )
    ).all()
    entry_by_book_id = {entry.book_id: entry for entry in entries}
    for hit in hits:
        book_id = book_ids.get(hit.ol_work_key)
        if book_id is None:
            continue
        entry = entry_by_book_id.get(book_id)
        if entry is not None:
            hit.on_shelf = entry.status
            hit.shelf_id = entry.id


def _current_pick_key(session: Session) -> str | None:
    current = session.exec(
        select(ClubPick)
        .where(ClubPick.ended_at.is_(None))
        .options(selectinload(ClubPick.book))
        .order_by(ClubPick.id.desc())
    ).first()
    if current is None or current.book is None:
        return None
    return current.book.ol_work_key


def _annotate_club_pick(hits: list[SearchHit], session: Session) -> None:
    if not hits:
        return
    key = _current_pick_key(session)
    if key is None:
        return
    for hit in hits:
        hit.club_pick = hit.ol_work_key == key


def _annotate(page: SearchPage, user: User, session: Session) -> SearchPage:
    _annotate_shelf(page.items, user, session)
    _annotate_club_pick(page.items, session)
    return page


def _local_catalog_hits(session: Session, query: str, limit: int) -> list[SearchHit]:
    """Club-local title/author matches, including books members added themselves."""
    needle = query.casefold()
    if len(needle) < 2:
        return []
    hits: list[SearchHit] = []
    books = session.exec(select(Book).order_by(Book.id)).all()
    for book in books:
        haystack = f"{book.title} {book.authors}".casefold()
        if needle not in haystack:
            continue
        hits.append(
            SearchHit(
                ol_work_key=book.ol_work_key,
                title=book.title,
                authors=book.authors,
                cover_id=_cover_id(book.cover_id),
                year=book.year,
                isbn=isbn_from_work_key(book.ol_work_key),
                custom=is_club_work_key(book.ol_work_key),
                cover_url=book_cover_url(book),
            )
        )
        if len(hits) >= limit:
            break
    return hits


def _merge_local_hits(page: SearchPage, local: list[SearchHit], limit: int) -> SearchPage:
    if not local:
        return page
    seen = {hit.ol_work_key for hit in local}
    merged = list(local)
    for hit in page.items:
        if hit.ol_work_key in seen:
            continue
        merged.append(hit)
        seen.add(hit.ol_work_key)
        if len(merged) >= limit:
            break
    page.items = merged
    return page


@router.get("/search", response_model=SearchPage)
async def search_books(
    q: str = Query(default=""),
    subject: str = Query(default=""),
    sort: Sort | None = Query(default=None),
    page: int = Query(default=1, ge=1),
    limit: int = Query(default=24, ge=1, le=40),
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> SearchPage:
    query = " ".join(q.split())
    if len(query) > 200:
        raise HTTPException(status_code=400, detail="Search is too long")
    subject = _normalize_subject(subject)

    # An empty box with no subject has no valid search query to send, so serve
    # trending instead of asking Open Library to match everything.
    if not query and not subject:
        return _annotate(await fetch_trending(limit), user, session)

    isbn = extract_isbn(query)
    if (
        query
        and len(query) < 3
        and not isbn
        and not subject
        and not google_books_enabled()
    ):
        raise HTTPException(status_code=400, detail=QUERY_TOO_SHORT)

    resolved_sort: Sort = sort or ("relevance" if query else "readinglog")
    local = _local_catalog_hits(session, query, limit) if (query and page == 1) else []
    try:
        result = await fetch_catalog(
            query,
            subject=subject,
            sort=resolved_sort,
            page=page,
            limit=limit,
        )
    except HTTPException:
        if local:
            return _annotate(
                SearchPage(items=local, page=page, has_more=False),
                user,
                session,
            )
        raise
    result = _merge_local_hits(result, local, limit)
    return _annotate(result, user, session)


@router.get("/trending", response_model=SearchPage)
async def trending_books(
    limit: int = Query(default=12, ge=1, le=40),
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> SearchPage:
    return _annotate(await fetch_trending(limit), user, session)


@router.get("/subjects/{subject}", response_model=SearchPage)
async def subject_books(
    subject: str = Path(...),
    page: int = Query(default=1, ge=1),
    limit: int = Query(default=12, ge=1, le=40),
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> SearchPage:
    key = _normalize_subject(subject)
    if not key:
        raise HTTPException(status_code=400, detail="That is not a valid subject.")
    result = await fetch_subject(key, page=page, limit=limit)
    return _annotate(result, user, session)


def _book_by_key(session: Session, work_key: str) -> Book | None:
    return session.exec(select(Book).where(Book.ol_work_key == work_key)).first()


def _annotate_book_detail(detail: BookDetailOut, book: Book, user: User, session: Session) -> BookDetailOut:
    if book.id is None:
        return detail
    entries = session.exec(
        select(ShelfEntry)
        .where(ShelfEntry.book_id == book.id)
        .options(selectinload(ShelfEntry.user))
    ).all()
    ratings = [
        entry.rating
        for entry in entries
        if entry.status == ShelfStatus.finished and entry.rating is not None
    ]
    if ratings:
        detail.club_rating = round(sum(ratings) / len(ratings), 1)
        detail.rating_count = len(ratings)
    for entry in entries:
        if entry.user_id == user.id:
            detail.on_shelf = entry.status
            detail.shelf_id = entry.id
            detail.rating = entry.rating
            detail.take = entry.take
            detail.dnf_reason = entry.dnf_reason
            detail.progress = entry.progress
        elif entry.user is not None:
            detail.readers.append(
                BookReader(
                    username=entry.user.username,
                    status=entry.status,
                    rating=entry.rating,
                    progress=entry.progress,
                )
            )
    detail.readers.sort(key=lambda reader: reader.username)
    return detail


def _book_detail_from_local(book: Book, user: User, session: Session) -> BookDetailOut:
    detail = BookDetailOut(
        ol_work_key=book.ol_work_key,
        title=book.title,
        authors=book.authors,
        cover_id=book.cover_id,
        cover_url=book_cover_url(book),
        year=book.year,
        description=book.description,
        subjects=subjects_from_json(book.subjects),
        club_pick=_current_pick_key(session) == book.ol_work_key,
        custom=is_club_work_key(book.ol_work_key),
    )
    return _annotate_book_detail(detail, book, user, session)


def _work_details_from_local(
    book: Book,
    user: User,
    session: Session,
    *,
    ol_rating_count: int | None = None,
) -> BookDetailsOut:
    members: list[BookMember] = []
    on_shelf = None
    shelf_id = None
    quote_count = 0
    if book.id is not None:
        entries = session.exec(
            select(ShelfEntry)
            .where(ShelfEntry.book_id == book.id)
            .options(selectinload(ShelfEntry.user))
            .order_by(ShelfEntry.status, ShelfEntry.id)
        ).all()
        for entry in entries:
            if entry.user is None:
                continue
            members.append(
                BookMember(
                    username=entry.user.username,
                    status=entry.status,
                    rating=entry.rating,
                    take=entry.take,
                    progress=entry.progress,
                    finished_at=entry.finished_at,
                )
            )
            if entry.user_id == user.id:
                on_shelf = entry.status
                shelf_id = entry.id
        members.sort(key=lambda row: (row.username != user.username, row.username))
        quote_count = len(session.exec(select(Quote).where(Quote.book_id == book.id)).all())
    pick = current_pick(session)
    return BookDetailsOut(
        ol_work_key=book.ol_work_key,
        title=book.title,
        authors=book.authors,
        cover_id=book.cover_id,
        year=book.year,
        cover_url=book_cover_url(book),
        description=book.description,
        pages=book.pages,
        subjects=subjects_from_json(book.subjects),
        ol_rating=book.ol_rating,
        ol_rating_count=ol_rating_count,
        on_shelf=on_shelf,
        shelf_id=shelf_id,
        club_pick=bool(pick and pick.book_id == book.id),
        members=members,
        quote_count=quote_count,
    )


async def _work_search_doc(work_id: str, work_key: str, *, bypass_cache: bool) -> dict:
    """Best-effort search.json hit for year / pages / rating / author names."""
    try:
        payload = await _fetch_json(
            OPEN_LIBRARY_URL,
            cache_key=f"work-search|{work_id}",
            detail=BROWSE_UNAVAILABLE,
            params={
                "q": f"key:{work_key}",
                "fields": WORK_SEARCH_FIELDS,
                "limit": 1,
            },
            bypass_cache=bypass_cache,
        )
    except HTTPException:
        return {}
    docs = payload.get("docs") or []
    doc = docs[0] if docs and isinstance(docs[0], dict) else {}
    return doc


async def _import_work_payload(session: Session, work_id: str, *, bypass_cache: bool = False) -> Book:
    """Fetch a work from Open Library and upsert it into the local Book row.

    Merges `works/{id}.json` (description, subjects, covers) with the
    `search.json key:` doc (year, pages, rating, author names), same fields
    `_load_work_details` uses for the `/work/` path.
    """
    work_key = f"/works/{work_id}"
    payload, doc = await asyncio.gather(
        _fetch_json(
            WORK_URL.format(work_id=work_id),
            cache_key=f"work|{work_id}",
            detail=BROWSE_UNAVAILABLE,
            missing_detail="No such book.",
            bypass_cache=bypass_cache,
        ),
        _work_search_doc(work_id, work_key, bypass_cache=bypass_cache),
    )
    title = str(payload.get("title") or doc.get("title") or "").strip()
    search_authors = ", ".join(str(name) for name in (doc.get("author_name") or []) if name)
    if bypass_cache:
        authors = await _author_names(payload, bypass_cache=True) or search_authors
    else:
        authors = search_authors or await _author_names(payload, bypass_cache=False)
    cover_id = first_positive_cover(payload.get("covers"), doc.get("cover_i"))
    year = _year(doc.get("first_publish_year"))
    pages = _year(doc.get("number_of_pages_median"))
    ol_rating = _rating(doc.get("ratings_average"))
    subjects = _work_subjects(payload)
    if not subjects:
        subjects = _work_subjects({"subjects": doc.get("subject") or []})
    book = _book_by_key(session, work_key)
    if book is None:
        book = upsert_book(
            session,
            ol_work_key=work_key,
            title=title or work_id,
            authors=authors,
            cover_id=cover_id,
            year=year,
        )
    apply_book_details(
        book,
        title=title,
        authors=authors,
        cover_id=cover_id,
        year=year,
        description=_work_description(payload),
        pages=pages,
        subjects=subjects,
        ol_rating=ol_rating,
    )
    session.add(book)
    session.commit()
    loaded = _book_by_key(session, work_key)
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save that book")
    return loaded


def _import_google_details(
    session: Session, work_id: str, details: VolumeDetails
) -> Book:
    """Upsert a Google Books volume under its ISBN or volume work key."""
    key = work_key(work_id)
    book = _book_by_key(session, key)
    if book is None:
        book = upsert_book(
            session,
            ol_work_key=key,
            title=details.title or work_id,
            authors=details.authors,
            cover_id=details.cover_id,
            cover_image_url=details.cover_image_url,
            year=details.year,
        )
    apply_book_details(
        book,
        title=details.title,
        authors=details.authors,
        cover_id=details.cover_id,
        year=details.year,
        description=details.description,
        pages=details.pages,
        subjects=details.subjects,
        ol_rating=details.rating,
        cover_image_url=details.cover_image_url,
    )
    session.add(book)
    session.commit()
    loaded = _book_by_key(session, key)
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save that book")
    return loaded


async def _load_google_catalog(
    work_id: str, *, bypass_cache: bool = False
) -> VolumeDetails:
    """Resolve an ISBN- or volume-keyed work from Google Books.

    When the Google key is set, a miss is a 404 — no Open Library ISBN
    fallback. Without a key, ISBN rows can still refresh from Open Library.
    """
    if is_google_work_id(work_id):
        volume_id = google_volume_id(work_id)
        if volume_id is None:
            raise HTTPException(status_code=404, detail="No such book.")
        if not google_books_enabled():
            raise HTTPException(
                status_code=400 if bypass_cache else 404,
                detail=GOOGLE_NO_REFRESH if bypass_cache else "No such book.",
            )
        return await fetch_volume(volume_id, bypass_cache=bypass_cache)

    isbn = isbn_from_work_id(work_id)
    if isbn is None:
        raise HTTPException(status_code=404, detail="No such book.")
    if google_books_enabled():
        try:
            details = await lookup_isbn_google(isbn, bypass_cache=bypass_cache)
        except GoogleBooksError as exc:
            raise _google_http_error(
                exc, detail="Could not load that book right now. Try again."
            ) from exc
        if details is None:
            raise HTTPException(status_code=404, detail="No such book.")
        return details
    hit = await lookup_isbn(isbn, bypass_cache=bypass_cache)
    if hit is None:
        raise HTTPException(status_code=404, detail="No such book.")
    return VolumeDetails(
        work_key=work_key(work_id),
        volume_id="",
        title=hit.title,
        authors=hit.authors,
        year=hit.year,
        description=None,
        pages=hit.pages,
        subjects=None,
        isbn=isbn,
        cover_id=hit.cover_id,
    )


async def _refresh_google_catalog(session: Session, work_id: str) -> Book:
    """Re-fetch Google metadata. A miss keeps the local row instead of wiping it."""
    existing = _book_by_key(session, work_key(work_id))
    try:
        details = await _load_google_catalog(work_id, bypass_cache=True)
    except HTTPException as exc:
        if exc.status_code == 404 and existing is not None:
            return existing
        raise
    return _import_google_details(session, work_id, details)


def _import_work_details(session: Session, work_id: str, details: WorkDetails) -> Book:
    work_key = f"/works/{work_id}"
    book = _book_by_key(session, work_key)
    if book is None:
        book = upsert_book(
            session,
            ol_work_key=work_key,
            title=details.title or work_id,
            authors=details.authors,
            cover_id=details.cover_id,
            year=details.year,
        )
    apply_book_details(
        book,
        title=details.title,
        authors=details.authors,
        cover_id=details.cover_id,
        year=details.year,
        description=details.description,
        pages=details.pages,
        subjects=details.subjects,
        ol_rating=details.ol_rating,
    )
    session.add(book)
    session.commit()
    loaded = _book_by_key(session, work_key)
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save that book")
    return loaded


@router.post("/custom", response_model=BookDetailOut, status_code=201)
def create_custom_book(
    payload: CustomBookIn,
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> BookDetailOut:
    """Add a book that Open Library does not have (#7). Never calls OL."""
    created_id = new_club_work_id()
    book = Book(
        ol_work_key=work_key(created_id),
        title=payload.title,
        authors=payload.authors,
        year=payload.year,
        description=payload.description,
    )
    session.add(book)
    session.commit()
    session.refresh(book)
    return _book_detail_from_local(book, user, session)


@router.get("/works/{work_id}", response_model=BookDetailOut)
async def book_detail(
    work_id: str = Path(...),
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> BookDetailOut:
    work_id = canonical_work_id(work_id)
    if not is_work_id(work_id):
        raise HTTPException(status_code=404, detail="No such book.")
    key = work_key(work_id)
    book = _book_by_key(session, key)
    if book is not None:
        return _book_detail_from_local(book, user, session)
    if is_club_work_id(work_id):
        raise HTTPException(status_code=404, detail="No such book.")
    if is_google_catalog_id(work_id):
        details = await _load_google_catalog(work_id)
        book = _import_google_details(session, work_id, details)
        return _book_detail_from_local(book, user, session)
    if not is_open_library_work_id(work_id):
        raise HTTPException(status_code=404, detail="No such book.")
    book = await _import_work_payload(session, work_id)
    return _book_detail_from_local(book, user, session)


@router.post("/works/{work_id}/refresh", response_model=BookDetailOut)
async def refresh_book_detail(
    work_id: str = Path(...),
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> BookDetailOut:
    work_id = canonical_work_id(work_id)
    if is_club_work_id(work_id):
        raise HTTPException(status_code=400, detail=CUSTOM_NO_REFRESH)
    if is_google_catalog_id(work_id):
        book = await _refresh_google_catalog(session, work_id)
        return _book_detail_from_local(book, user, session)
    if not is_open_library_work_id(work_id):
        raise HTTPException(status_code=404, detail="No such book.")
    book = await _import_work_payload(session, work_id, bypass_cache=True)
    return _book_detail_from_local(book, user, session)


@router.get("/work/{work_id}", response_model=BookDetailsOut)
async def work_details(
    work_id: str,
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> BookDetailsOut:
    work_id = canonical_work_id(work_id)
    if is_club_work_id(work_id):
        key = work_key(work_id)
        book = _book_by_key(session, key)
        if book is None:
            raise HTTPException(status_code=404, detail="No such book.")
        return _work_details_from_local(book, user, session)
    if is_google_catalog_id(work_id):
        key = work_key(work_id)
        book = _book_by_key(session, key)
        if book is not None:
            return _work_details_from_local(book, user, session)
        details = await _load_google_catalog(work_id)
        book = _import_google_details(session, work_id, details)
        return _work_details_from_local(
            book, user, session, ol_rating_count=details.rating_count
        )
    if not is_open_library_work_id(work_id):
        raise HTTPException(status_code=400, detail="That is not an Open Library work id")
    key = work_key(work_id)
    book = _book_by_key(session, key)
    if book is not None:
        return _work_details_from_local(book, user, session)
    details = await fetch_work_details(key)
    book = _import_work_details(session, work_id, details)
    return _work_details_from_local(book, user, session, ol_rating_count=details.ol_rating_count)


@router.post("/work/{work_id}/refresh", response_model=BookDetailsOut)
async def refresh_work_details(
    work_id: str,
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> BookDetailsOut:
    work_id = canonical_work_id(work_id)
    if is_club_work_id(work_id):
        raise HTTPException(status_code=400, detail=CUSTOM_NO_REFRESH)
    if is_google_catalog_id(work_id):
        book = await _refresh_google_catalog(session, work_id)
        return _work_details_from_local(book, user, session)
    if not is_open_library_work_id(work_id):
        raise HTTPException(status_code=400, detail="That is not an Open Library work id")
    key = work_key(work_id)
    details = await fetch_work_details(key, bypass_cache=True)
    book = _import_work_details(session, work_id, details)
    return _work_details_from_local(book, user, session, ol_rating_count=details.ol_rating_count)


@router.get("/isbn/{isbn}", response_model=IsbnHitOut)
async def isbn_lookup(
    isbn: str,
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> IsbnHitOut:
    clean = normalize_isbn(isbn)
    if google_books_enabled():
        try:
            gb_hit = await lookup_isbn_google(clean)
        except GoogleBooksError as exc:
            raise _google_http_error(
                exc, detail="Could not look that ISBN up right now. Try again."
            ) from exc
        if gb_hit is None:
            raise HTTPException(status_code=404, detail="No book found for that ISBN")
        out = IsbnHitOut(
            isbn=clean,
            ol_work_key=gb_hit.work_key,
            title=gb_hit.title,
            authors=gb_hit.authors,
            cover_id=None,
            year=gb_hit.year,
            pages=gb_hit.pages,
            cover_url=gb_hit.cover_image_url,
        )
        book = session.exec(
            select(Book).where(Book.ol_work_key == gb_hit.work_key)
        ).first()
        if book is not None:
            entry = session.exec(
                select(ShelfEntry).where(
                    ShelfEntry.user_id == user.id, ShelfEntry.book_id == book.id
                )
            ).first()
            if entry is not None:
                out.on_shelf = entry.status
                out.shelf_id = entry.id
        return out
    hit = await lookup_isbn(clean)
    if hit is None:
        raise HTTPException(status_code=404, detail="No book found for that ISBN")
    out = IsbnHitOut(
        isbn=hit.isbn,
        ol_work_key=hit.ol_work_key,
        title=hit.title,
        authors=hit.authors,
        cover_id=hit.cover_id,
        year=hit.year,
        pages=hit.pages,
    )
    book = session.exec(select(Book).where(Book.ol_work_key == hit.ol_work_key)).first()
    if book is not None:
        entry = session.exec(
            select(ShelfEntry).where(
                ShelfEntry.user_id == user.id, ShelfEntry.book_id == book.id
            )
        ).first()
        if entry is not None:
            out.on_shelf = entry.status
            out.shelf_id = entry.id
    return out
