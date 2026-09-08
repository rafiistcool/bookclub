"""The reading diary: club-wide entries on any book, with one level of replies.

Entries live on the book, not on a club pick, so members can write about
whatever they are reading. The current pick's discussion on Home is simply
that book's diary.
"""

from fastapi import APIRouter, Depends, HTTPException, Path, Query
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app.config import get_settings
from app.deps import get_current_user, get_session
from app.models import (
    Book,
    BookPost,
    BookPostReaction,
    ShelfEntry,
    ShelfStatus,
    User,
    utcnow,
)
from app.pick_ops import current_pick
from app.schemas import (
    DiaryEntryIn,
    DiaryEntryOut,
    DiaryEntryPatchIn,
    DiaryFeedItemOut,
    DiaryFeedOut,
    DiaryOut,
    ReactionIn,
    ReactionOut,
)
from app.serialize import book_out
from app.shelf_ops import upsert_book
from app.timezone import meeting_label, resolved_timezone
from app.works import is_work_id, work_key

router = APIRouter(tags=["diary"])

FEED_DEFAULT = 10
FEED_MAX = 50


def _timezone() -> str:
    return resolved_timezone(get_settings().bookclub_tz)


def _work_key(work_id: str) -> str:
    if not is_work_id(work_id):
        raise HTTPException(status_code=404, detail="No such book.")
    return work_key(work_id)


def _position(entry: ShelfEntry | None) -> tuple[int | None, ShelfStatus | None]:
    """Where a member is in a book: progress percent (100 when finished) and status."""
    if entry is None:
        return None, None
    if entry.status == ShelfStatus.finished:
        return 100, entry.status
    if entry.status == ShelfStatus.currently_reading:
        return entry.progress, entry.status
    return None, entry.status


def _shelf_by_user(session: Session, book_id: int) -> dict[int, ShelfEntry]:
    rows = session.exec(select(ShelfEntry).where(ShelfEntry.book_id == book_id)).all()
    return {row.user_id: row for row in rows}


def _reactions_out(post: BookPost, viewer: User) -> list[ReactionOut]:
    grouped: dict[str, list[str]] = {}
    for reaction in post.reactions:
        name = reaction.user.username if reaction.user is not None else "unknown"
        grouped.setdefault(reaction.emoji, []).append(name)
    out = [
        ReactionOut(
            emoji=emoji,
            count=len(users),
            mine=viewer.username in users,
            users=sorted(users),
        )
        for emoji, users in grouped.items()
    ]
    out.sort(key=lambda row: (-row.count, row.emoji))
    return out


def _entry_out(
    post: BookPost,
    viewer: User,
    tz_name: str,
    shelf: dict[int, ShelfEntry],
    *,
    replies: list[BookPost] | None = None,
) -> DiaryEntryOut:
    author_entry = shelf.get(post.author_id)
    author_rating = (
        author_entry.rating
        if author_entry is not None and author_entry.status == ShelfStatus.finished
        else None
    )
    deleted = post.deleted_at is not None
    return DiaryEntryOut(
        id=post.id or 0,
        author=post.author.username if post.author is not None else "unknown",
        mine=post.author_id == viewer.id,
        body="" if deleted else post.body,
        deleted=deleted,
        spoiler_upto=None if deleted else post.spoiler_upto,
        progress_at=post.progress_at,
        status_at=post.status_at,
        author_rating=author_rating,
        parent_id=post.parent_id,
        created_at=post.created_at,
        created_label=meeting_label(post.created_at, tz_name) or "",
        edited=post.updated_at is not None and not deleted,
        reactions=[] if deleted else _reactions_out(post, viewer),
        replies=[_entry_out(row, viewer, tz_name, shelf) for row in (replies or [])],
    )


def _post_query():
    return select(BookPost).options(
        selectinload(BookPost.author),
        selectinload(BookPost.reactions).selectinload(BookPostReaction.user),
    )


def _load_post(session: Session, post_id: int) -> BookPost | None:
    return session.exec(_post_query().where(BookPost.id == post_id)).first()


def _book_by_key(session: Session, work_key: str) -> Book | None:
    return session.exec(select(Book).where(Book.ol_work_key == work_key)).first()


def _diary_for(session: Session, work_key: str, book: Book | None, viewer: User) -> DiaryOut:
    tz_name = _timezone()
    if book is None or book.id is None:
        return DiaryOut(ol_work_key=work_key, items=[], timezone=tz_name)
    rows = session.exec(
        _post_query()
        .where(BookPost.book_id == book.id)
        .order_by(BookPost.created_at, BookPost.id)
    ).all()
    shelf = _shelf_by_user(session, book.id)
    replies_by_parent: dict[int, list[BookPost]] = {}
    for row in rows:
        if row.parent_id is not None:
            replies_by_parent.setdefault(row.parent_id, []).append(row)
    items = [
        _entry_out(row, viewer, tz_name, shelf, replies=replies_by_parent.get(row.id or 0))
        for row in rows
        if row.parent_id is None
    ]
    my_progress, my_status = _position(shelf.get(viewer.id or 0))
    return DiaryOut(
        ol_work_key=work_key,
        book_id=book.id,
        my_progress=my_progress,
        my_status=my_status,
        items=items,
        timezone=tz_name,
    )


@router.get("/api/books/works/{work_id}/posts", response_model=DiaryOut)
def book_diary(
    work_id: str = Path(...),
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> DiaryOut:
    work_key = _work_key(work_id)
    return _diary_for(session, work_key, _book_by_key(session, work_key), me)


@router.post("/api/books/works/{work_id}/posts", response_model=DiaryEntryOut, status_code=201)
def add_entry(
    payload: DiaryEntryIn,
    work_id: str = Path(...),
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> DiaryEntryOut:
    work_key = _work_key(work_id)
    book = _book_by_key(session, work_key)
    if book is None:
        if payload.book is None:
            raise HTTPException(
                status_code=400, detail="Nobody has this book yet; send its title to start the diary"
            )
        book = upsert_book(
            session,
            ol_work_key=work_key,
            title=payload.book.title,
            authors=payload.book.authors,
            cover_id=payload.book.cover_id,
            year=payload.book.year,
        )
    book_id = book.id or 0

    if payload.parent_id is not None:
        parent = session.get(BookPost, payload.parent_id)
        if parent is None or parent.book_id != book_id:
            raise HTTPException(status_code=400, detail="That entry is not on this book")
        if parent.parent_id is not None:
            raise HTTPException(status_code=400, detail="Reply to the entry itself, not to a reply")
        if parent.deleted_at is not None:
            raise HTTPException(status_code=400, detail="That entry was deleted")

    mine = session.exec(
        select(ShelfEntry).where(ShelfEntry.user_id == me.id, ShelfEntry.book_id == book_id)
    ).first()
    progress_at, status_at = _position(mine)
    pick = current_pick(session)
    post = BookPost(
        book_id=book_id,
        author_id=me.id or 0,
        parent_id=payload.parent_id,
        body=payload.body,
        spoiler_upto=payload.spoiler_upto,
        progress_at=progress_at,
        status_at=status_at,
        pick_id=pick.id if pick is not None and pick.book_id == book_id else None,
    )
    session.add(post)
    session.commit()
    loaded = _load_post(session, post.id or 0)
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save that entry")
    return _entry_out(loaded, me, _timezone(), _shelf_by_user(session, book_id))


@router.patch("/api/posts/{post_id}", response_model=DiaryEntryOut)
def edit_entry(
    post_id: int,
    payload: DiaryEntryPatchIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> DiaryEntryOut:
    post = _load_post(session, post_id)
    if post is None or post.deleted_at is not None:
        raise HTTPException(status_code=404, detail="No entry with that id")
    if post.author_id != me.id:
        raise HTTPException(status_code=403, detail="You can only edit your own entries")
    if payload.body is None and "spoiler_upto" not in payload.model_fields_set:
        raise HTTPException(status_code=400, detail="Nothing to update")
    if payload.body is not None:
        post.body = payload.body
    if "spoiler_upto" in payload.model_fields_set:
        post.spoiler_upto = payload.spoiler_upto
    post.updated_at = utcnow()
    session.add(post)
    session.commit()
    loaded = _load_post(session, post_id)
    if loaded is None:
        raise HTTPException(status_code=404, detail="No entry with that id")
    replies = _replies_of(session, loaded) if loaded.parent_id is None else None
    return _entry_out(
        loaded, me, _timezone(), _shelf_by_user(session, loaded.book_id), replies=replies
    )


def _replies_of(session: Session, post: BookPost) -> list[BookPost]:
    return session.exec(
        _post_query()
        .where(BookPost.parent_id == post.id)
        .order_by(BookPost.created_at, BookPost.id)
    ).all()


def _delete_reactions(session: Session, post_id: int) -> None:
    for reaction in session.exec(
        select(BookPostReaction).where(BookPostReaction.post_id == post_id)
    ).all():
        session.delete(reaction)


@router.delete("/api/posts/{post_id}", status_code=204)
def delete_entry(
    post_id: int,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> None:
    post = session.get(BookPost, post_id)
    if post is None or post.deleted_at is not None:
        raise HTTPException(status_code=404, detail="No entry with that id")
    if post.author_id != me.id:
        raise HTTPException(status_code=403, detail="You can only delete your own entries")
    _delete_reactions(session, post_id)
    has_replies = (
        session.exec(select(BookPost.id).where(BookPost.parent_id == post_id).limit(1)).first()
        is not None
    )
    if has_replies:
        # Keep the anchor so the replies still make sense.
        post.body = ""
        post.spoiler_upto = None
        post.deleted_at = utcnow()
        session.add(post)
    else:
        session.delete(post)
    session.commit()


@router.post("/api/posts/{post_id}/reactions", response_model=DiaryEntryOut)
def toggle_reaction(
    post_id: int,
    payload: ReactionIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> DiaryEntryOut:
    post = _load_post(session, post_id)
    if post is None or post.deleted_at is not None:
        raise HTTPException(status_code=404, detail="No entry with that id")
    existing = session.exec(
        select(BookPostReaction).where(
            BookPostReaction.post_id == post_id,
            BookPostReaction.user_id == me.id,
            BookPostReaction.emoji == payload.emoji,
        )
    ).first()
    if existing is not None:
        session.delete(existing)
    else:
        session.add(BookPostReaction(post_id=post_id, user_id=me.id or 0, emoji=payload.emoji))
    session.commit()
    session.expire_all()
    loaded = _load_post(session, post_id)
    if loaded is None:
        raise HTTPException(status_code=404, detail="No entry with that id")
    replies = _replies_of(session, loaded) if loaded.parent_id is None else None
    return _entry_out(
        loaded, me, _timezone(), _shelf_by_user(session, loaded.book_id), replies=replies
    )


@router.get("/api/diary", response_model=DiaryFeedOut)
def diary_feed(
    before: int | None = Query(default=None, ge=1),
    limit: int = Query(default=FEED_DEFAULT, ge=1, le=FEED_MAX),
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> DiaryFeedOut:
    """Newest entries across every book, for the Club page. `before` is an entry id."""
    tz_name = _timezone()
    query = (
        _post_query()
        .options(selectinload(BookPost.book))
        .where(BookPost.deleted_at.is_(None))
        .order_by(col(BookPost.id).desc())
        .limit(limit + 1)
    )
    if before is not None:
        query = query.where(BookPost.id < before)
    rows = session.exec(query).all()
    has_more = len(rows) > limit
    rows = rows[:limit]

    shelf_cache: dict[int, dict[int, ShelfEntry]] = {}
    parent_ids = [row.parent_id for row in rows if row.parent_id is not None]
    parents: dict[int, BookPost] = {}
    if parent_ids:
        for parent in session.exec(
            select(BookPost)
            .options(selectinload(BookPost.author))
            .where(col(BookPost.id).in_(parent_ids))
        ).all():
            parents[parent.id or 0] = parent

    items: list[DiaryFeedItemOut] = []
    for row in rows:
        if row.book is None:
            continue
        shelf = shelf_cache.get(row.book_id)
        if shelf is None:
            shelf = _shelf_by_user(session, row.book_id)
            shelf_cache[row.book_id] = shelf
        parent = parents.get(row.parent_id or 0) if row.parent_id is not None else None
        my_progress, my_status = _position(shelf.get(me.id or 0))
        items.append(
            DiaryFeedItemOut(
                entry=_entry_out(row, me, tz_name, shelf),
                book=book_out(row.book),
                parent_author=(
                    parent.author.username if parent is not None and parent.author else None
                ),
                my_progress=my_progress,
                my_status=my_status,
            )
        )
    return DiaryFeedOut(items=items, has_more=has_more, timezone=tz_name)
