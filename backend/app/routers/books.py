import logging
import re
from time import monotonic, time
from typing import Literal

import httpx
from fastapi import APIRouter, Depends, HTTPException, Query
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app.branding import open_library_ua
from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import Book, ClubPick, Quote, ShelfEntry, User, utcnow
from app.openlibrary import (
    fetch_work_details,
    lookup_isbn,
    normalize_isbn,
    subjects_from_json,
    subjects_to_json,
)
from app.pick_ops import current_pick
from app.schemas import BookDetailsOut, BookMember, IsbnHitOut, SearchHit, SearchPage
from app.serialize import cover_url

DETAILS_REFRESH_DAYS = 7

router = APIRouter(prefix="/api/books", tags=["books"])
logger = logging.getLogger("bookclub.search")

_SUBJECT_KEY_RE = re.compile(r"^[a-z0-9_]+$")
_CACHE_TTL = 300.0
_search_cache: dict[str, tuple[float, SearchPage]] = {}
OPEN_LIBRARY_URL = "https://openlibrary.org/search.json"
Sort = Literal["readinglog", "new", "title", "relevance"]


def clear_search_cache() -> None:
    _search_cache.clear()


def _cache_get(key: str) -> SearchPage | None:
    hit = _search_cache.get(key)
    if hit is None:
        return None
    expires, value = hit
    if expires <= time():
        _search_cache.pop(key, None)
        return None
    return value


def _cache_set(key: str, value: SearchPage) -> None:
    _search_cache[key] = (time() + _CACHE_TTL, value)


def _copy_page(page: SearchPage) -> SearchPage:
    return SearchPage(
        items=[hit.model_copy() for hit in page.items],
        page=page.page,
        has_more=page.has_more,
    )


def map_open_library_docs(docs: list[dict]) -> list[SearchHit]:
    hits: list[SearchHit] = []
    seen: set[str] = set()
    for doc in docs:
        key = str(doc.get("key") or "")
        if not key.startswith("/works/") or key in seen:
            continue
        title = str(doc.get("title") or "").strip()
        if not title:
            continue
        authors = ", ".join(str(name) for name in (doc.get("author_name") or []) if name)
        cover = doc.get("cover_i")
        year = doc.get("first_publish_year")
        hits.append(
            SearchHit(
                ol_work_key=key,
                title=title,
                authors=authors,
                cover_id=int(cover) if cover is not None else None,
                year=int(year) if year is not None else None,
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


# Open Library rejects a bare "*" wildcard with HTTP 422; "*:*" is the
# Solr match-all it accepts, which keeps the default "Popular" browse alive.
MATCH_ALL = "*:*"


def _open_library_q(query: str, subject: str) -> str:
    if query and subject:
        return f"{query} subject_key:{subject}"
    if subject:
        return f"subject_key:{subject}"
    return query or MATCH_ALL


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
    cache_key = f"{query}|{subject}|{sort}|{page}|{limit}"
    cached = _cache_get(cache_key)
    if cached is not None:
        return _copy_page(cached)

    params: dict[str, str | int] = {
        "q": _open_library_q(query, subject),
        "page": page,
        "limit": limit,
        "fields": "key,title,author_name,cover_i,first_publish_year",
    }
    if not query:
        params["lang"] = "en"
    if sort != "relevance":
        params["sort"] = sort

    started = monotonic()
    try:
        async with httpx.AsyncClient(timeout=8.0) as client:
            response = await client.get(
                OPEN_LIBRARY_URL,
                params=params,
                headers={"User-Agent": open_library_ua(get_settings())},
            )
            response.raise_for_status()
            payload = response.json()
    except httpx.HTTPError as exc:
        elapsed_ms = int((monotonic() - started) * 1000)
        status = getattr(getattr(exc, "response", None), "status_code", None)
        logger.warning(
            "open library search failed query_len=%s status=%s latency_ms=%s err=%s",
            len(query),
            status,
            elapsed_ms,
            exc.__class__.__name__,
        )
        raise HTTPException(
            status_code=502,
            detail="Could not search the library right now. Try again.",
        ) from exc

    elapsed_ms = int((monotonic() - started) * 1000)
    logger.info("open library search ok latency_ms=%s", elapsed_ms)
    items = map_open_library_docs(payload.get("docs") or [])
    result = SearchPage(
        items=items,
        page=page,
        has_more=page * limit < _num_found(payload),
    )
    _cache_set(cache_key, result)
    return _copy_page(result)


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


def _annotate_club_pick(hits: list[SearchHit], session: Session) -> None:
    if not hits:
        return
    current = session.exec(
        select(ClubPick)
        .where(ClubPick.ended_at.is_(None))
        .options(selectinload(ClubPick.book))
        .order_by(ClubPick.id.desc())
    ).first()
    if current is None or current.book is None:
        return
    key = current.book.ol_work_key
    for hit in hits:
        hit.club_pick = hit.ol_work_key == key


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
    resolved_sort: Sort = sort or ("relevance" if query else "readinglog")

    result = await fetch_open_library(
        query,
        subject=subject,
        sort=resolved_sort,
        page=page,
        limit=limit,
    )
    _annotate_shelf(result.items, user, session)
    _annotate_club_pick(result.items, session)
    return result


def _stale(book: Book) -> bool:
    if book.details_fetched_at is None:
        return True
    fetched = book.details_fetched_at
    if fetched.tzinfo is None:
        from datetime import timezone

        fetched = fetched.replace(tzinfo=timezone.utc)
    return (utcnow() - fetched).days >= DETAILS_REFRESH_DAYS


@router.get("/work/{work_id}", response_model=BookDetailsOut)
async def work_details(
    work_id: str,
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> BookDetailsOut:
    if not re.fullmatch(r"OL\d+W", work_id):
        raise HTTPException(status_code=400, detail="That is not an Open Library work id")
    key = f"/works/{work_id}"
    book = session.exec(select(Book).where(Book.ol_work_key == key)).first()

    if book is not None and not _stale(book):
        title, authors, cover_id, year = book.title, book.authors, book.cover_id, book.year
        description, pages, subjects, ol_rating, rating_count = (
            book.description,
            book.pages,
            subjects_from_json(book.subjects),
            book.ol_rating,
            None,
        )
    else:
        details = await fetch_work_details(key)
        title = details.title or (book.title if book else "")
        authors = details.authors or (book.authors if book else "")
        cover_id = details.cover_id if details.cover_id is not None else (book.cover_id if book else None)
        year = details.year if details.year is not None else (book.year if book else None)
        description, pages, subjects = details.description, details.pages, details.subjects
        ol_rating, rating_count = details.ol_rating, details.ol_rating_count
        if book is not None:
            book.description = description
            book.pages = pages
            book.subjects = subjects_to_json(subjects)
            book.ol_rating = ol_rating
            if cover_id is not None:
                book.cover_id = cover_id
            book.details_fetched_at = utcnow()
            session.add(book)
            session.commit()

    members: list[BookMember] = []
    on_shelf = None
    shelf_id = None
    quote_count = 0
    if book is not None:
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
    club_pick = bool(pick and book and pick.book_id == book.id)

    return BookDetailsOut(
        ol_work_key=key,
        title=title,
        authors=authors,
        cover_id=cover_id,
        year=year,
        cover_url=cover_url(cover_id),
        description=description,
        pages=pages,
        subjects=subjects,
        ol_rating=ol_rating,
        ol_rating_count=rating_count,
        on_shelf=on_shelf,
        shelf_id=shelf_id,
        club_pick=club_pick,
        members=members,
        quote_count=quote_count,
    )


@router.get("/isbn/{isbn}", response_model=IsbnHitOut)
async def isbn_lookup(
    isbn: str,
    user: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> IsbnHitOut:
    clean = normalize_isbn(isbn)
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
