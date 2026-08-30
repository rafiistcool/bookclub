import asyncio
import logging
import re
from time import monotonic, time
from typing import Any, Literal

import httpx
from fastapi import APIRouter, Depends, HTTPException, Path, Query
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app.branding import open_library_ua
from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import Book, ClubPick, ShelfEntry, User
from app.schemas import BookDetailOut, BookReader, SearchHit, SearchPage

router = APIRouter(prefix="/api/books", tags=["books"])
logger = logging.getLogger("bookclub.search")

_SUBJECT_KEY_RE = re.compile(r"^[a-z0-9_]+$")
_WORK_ID_RE = re.compile(r"^OL\d+W$")
_AUTHOR_ID_RE = re.compile(r"^OL\d+A$")
# Markdown leftovers in Open Library descriptions, which we render as text.
_REF_DEFINITION_RE = re.compile(r"^[ \t]*\[[^\]]+\]:[ \t]*\S+.*$", re.MULTILINE)
_INLINE_LINK_RE = re.compile(r"\[([^\]]+)\]\([^)]*\)")
_REF_LINK_RE = re.compile(r"\[([^\]]+)\]\[[^\]]*\]")
_BLOCKQUOTE_RE = re.compile(r"^[ \t]*>[ \t]?", re.MULTILINE)
_BLANK_LINES_RE = re.compile(r"\n{3,}")
# Open Library mixes machine tags like "award:hugo_award=1970" into subjects.
_MACHINE_TAG_RE = re.compile(r"^[a-z_]+:\S")
_CACHE_TTL = 300.0
_cache: dict[str, tuple[float, Any]] = {}
# Open Library's trending and subject endpoints regularly take well over 8s to
# respond, so the read budget is generous while connect stays short.
_TIMEOUT = httpx.Timeout(connect=5.0, read=15.0, write=5.0, pool=5.0)

OPEN_LIBRARY_URL = "https://openlibrary.org/search.json"
TRENDING_URL = "https://openlibrary.org/trending/daily.json"
SUBJECT_URL = "https://openlibrary.org/subjects/{subject}.json"
WORK_URL = "https://openlibrary.org/works/{work_id}.json"
AUTHOR_URL = "https://openlibrary.org/authors/{author_id}.json"

SEARCH_FIELDS = "key,title,author_name,cover_i,first_publish_year"
SEARCH_UNAVAILABLE = "Could not search the library right now. Try again."
BROWSE_UNAVAILABLE = "Could not load the library right now. Try again."
# Open Library rejects any q shorter than 3 characters, so an empty search box
# cannot be expressed as a search query at all. Browse falls back to trending.
MAX_DETAIL_AUTHORS = 3

Sort = Literal["readinglog", "new", "title", "relevance"]


def clear_search_cache() -> None:
    _cache.clear()


def _cache_get(key: str) -> Any | None:
    hit = _cache.get(key)
    if hit is None:
        return None
    expires, value = hit
    if expires <= time():
        _cache.pop(key, None)
        return None
    return value


def _cache_set(key: str, value: Any) -> None:
    _cache[key] = (time() + _CACHE_TTL, value)


async def _fetch_json(
    url: str,
    *,
    cache_key: str,
    detail: str,
    params: dict[str, str | int] | None = None,
    missing_detail: str | None = None,
) -> dict:
    """GET JSON from Open Library, cached by `cache_key`.

    Raises 502 on transport or status errors, or 404 when `missing_detail` is
    set and Open Library reports the resource does not exist.
    """
    cached = _cache_get(cache_key)
    if cached is not None:
        return cached

    started = monotonic()
    try:
        async with httpx.AsyncClient(timeout=_TIMEOUT) as client:
            response = await client.get(
                url,
                params=params,
                headers={"User-Agent": open_library_ua(get_settings())},
            )
            response.raise_for_status()
            payload = response.json()
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
        if status == 404 and missing_detail is not None:
            raise HTTPException(status_code=404, detail=missing_detail) from exc
        raise HTTPException(status_code=502, detail=detail) from exc

    if not isinstance(payload, dict):
        logger.warning("open library returned a non-object payload url=%s", url)
        raise HTTPException(status_code=502, detail=detail)

    logger.info(
        "open library ok url=%s latency_ms=%s",
        url,
        int((monotonic() - started) * 1000),
    )
    _cache_set(cache_key, payload)
    return payload


def _cover_id(value: Any) -> int | None:
    # Open Library uses -1 for "no cover" in some payloads.
    try:
        cover = int(value)
    except (TypeError, ValueError):
        return None
    return cover if cover > 0 else None


def _year(value: Any) -> int | None:
    try:
        return int(value)
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


def _num_found(payload: dict) -> int:
    raw = payload.get("num_found")
    if raw is None:
        raw = payload.get("numFound")
    try:
        return int(raw or 0)
    except (TypeError, ValueError):
        return 0


async def fetch_open_library(
    query: str,
    *,
    subject: str,
    sort: Sort,
    page: int,
    limit: int,
) -> SearchPage:
    params: dict[str, str | int] = {
        "q": _open_library_q(query, subject),
        "page": page,
        "limit": limit,
        "fields": SEARCH_FIELDS,
    }
    if not query:
        params["lang"] = "en"
    if sort != "relevance":
        params["sort"] = sort

    cache_key = f"search|{params['q']}|{sort}|{page}|{limit}"
    payload = await _fetch_json(
        OPEN_LIBRARY_URL,
        cache_key=cache_key,
        detail=SEARCH_UNAVAILABLE,
        params=params,
    )
    items = map_open_library_docs(payload.get("docs") or [])
    return SearchPage(
        items=items,
        page=page,
        has_more=page * limit < _num_found(payload),
    )


async def fetch_trending(limit: int) -> SearchPage:
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


async def _author_name(author_id: str) -> str:
    try:
        author = await _fetch_json(
            AUTHOR_URL.format(author_id=author_id),
            cache_key=f"author|{author_id}",
            detail=BROWSE_UNAVAILABLE,
        )
    except HTTPException:
        return ""
    name = author.get("name")
    return name.strip() if isinstance(name, str) else ""


async def _author_names(payload: dict) -> str:
    """Resolve work author refs to names. Best effort: refs carry no names."""
    author_ids = _author_ids(payload)
    if not author_ids:
        return ""
    names = await asyncio.gather(*(_author_name(one) for one in author_ids))
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

    resolved_sort: Sort = sort or ("relevance" if query else "readinglog")
    result = await fetch_open_library(
        query,
        subject=subject,
        sort=resolved_sort,
        page=page,
        limit=limit,
    )
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


@router.get("/works/{work_id}", response_model=BookDetailOut)
async def book_detail(
    work_id: str = Path(...),
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> BookDetailOut:
    if not _WORK_ID_RE.fullmatch(work_id):
        raise HTTPException(status_code=404, detail="No such book.")
    work_key = f"/works/{work_id}"

    payload = await _fetch_json(
        WORK_URL.format(work_id=work_id),
        cache_key=f"work|{work_id}",
        detail=BROWSE_UNAVAILABLE,
        missing_detail="No such book.",
    )

    book = session.exec(select(Book).where(Book.ol_work_key == work_key)).first()
    covers = [
        cover for cover in (_cover_id(raw) for raw in payload.get("covers") or []) if cover
    ]

    title = str(payload.get("title") or "").strip()
    detail = BookDetailOut(
        ol_work_key=work_key,
        title=title or (book.title if book else ""),
        authors=await _author_names(payload) or (book.authors if book else ""),
        cover_id=covers[0] if covers else (book.cover_id if book else None),
        year=book.year if book else None,
        description=_work_description(payload),
        subjects=_work_subjects(payload),
        club_pick=_current_pick_key(session) == work_key,
    )

    if book is None or book.id is None:
        return detail

    entries = session.exec(
        select(ShelfEntry)
        .where(ShelfEntry.book_id == book.id)
        .options(selectinload(ShelfEntry.user))
    ).all()
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
