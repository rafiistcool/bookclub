from fastapi import APIRouter, Depends, File, HTTPException, Query, UploadFile
from fastapi.responses import JSONResponse
from sqlalchemy.orm import selectinload
from sqlmodel import Session, select

from app import activity
from app.deps import get_current_user, get_session
from app.goodreads import (
    MAX_IMPORT_BYTES,
    SKIP_LIST_LIMIT,
    lookup_catalog,
    parse_goodreads_csv,
)
from app.models import ShelfEntry, ShelfStatus, User
from app.schemas import (
    GoodreadsImportOut,
    GoodreadsSkipOut,
    ShelfAddIn,
    ShelfItemOut,
    ShelfListOut,
    ShelfPatchIn,
)
from app.serialize import shelf_item_out, user_out
from app.shelf_ops import apply_finish_fields, apply_progress, place_item, upsert_book

router = APIRouter(prefix="/api/shelf", tags=["shelf"])


def _status_event(entry: ShelfEntry, me: User, session: Session, *, added: bool) -> None:
    """Record the activity row that matches a shelf status change."""
    status = entry.status
    book = entry.book
    if status == ShelfStatus.finished:
        activity.record(
            session, me, "shelf_finished", book=book, rating=entry.rating, take=entry.take or None
        )
    elif status == ShelfStatus.did_not_finish:
        activity.record(session, me, "shelf_dnf", book=book, reason=entry.dnf_reason or None)
    elif added:
        activity.record(session, me, "shelf_added", book=book, status=status.value)
    else:
        activity.record(session, me, "shelf_moved", book=book, status=status.value)


def _load_entry(session: Session, entry_id: int) -> ShelfEntry | None:
    return session.exec(
        select(ShelfEntry)
        .where(ShelfEntry.id == entry_id)
        .options(selectinload(ShelfEntry.book))
    ).first()


def _shelf_for_user(session: Session, user: User) -> list[ShelfEntry]:
    return session.exec(
        select(ShelfEntry)
        .where(ShelfEntry.user_id == user.id)
        .options(selectinload(ShelfEntry.book))
        .order_by(ShelfEntry.status, ShelfEntry.position, ShelfEntry.id)
    ).all()


@router.get("", response_model=ShelfListOut)
def get_shelf(
    username: str | None = Query(default=None),
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ShelfListOut:
    if username:
        owner = session.exec(select(User).where(User.username == username.lower())).first()
        if owner is None:
            raise HTTPException(status_code=404, detail="No member with that name")
    else:
        owner = me
    items = [_shelf_item_or_skip(entry) for entry in _shelf_for_user(session, owner)]
    return ShelfListOut(
        user=user_out(owner),
        items=[item for item in items if item is not None],
    )


def _shelf_item_or_skip(entry: ShelfEntry) -> ShelfItemOut | None:
    if entry.book is None:
        return None
    return shelf_item_out(entry)


@router.post("", response_model=ShelfItemOut, status_code=201)
def add_to_shelf(
    payload: ShelfAddIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ShelfItemOut | JSONResponse:
    book = upsert_book(
        session,
        ol_work_key=payload.ol_work_key,
        title=payload.title,
        authors=payload.authors,
        cover_id=payload.cover_id,
        year=payload.year,
        cover_image_url=payload.cover_url,
    )
    existing = session.exec(
        select(ShelfEntry)
        .where(ShelfEntry.user_id == me.id, ShelfEntry.book_id == book.id)
        .options(selectinload(ShelfEntry.book))
    ).first()
    if existing is not None:
        return JSONResponse(
            status_code=409,
            content={
                "detail": "Already on your shelf",
                "item": shelf_item_out(existing).model_dump(mode="json"),
            },
        )

    entry = ShelfEntry(
        user_id=me.id or 0,
        book_id=book.id or 0,
        status=payload.status,
        position=0,
    )
    apply_finish_fields(
        entry,
        payload.status,
        rating=payload.rating,
        take=payload.take,
        dnf_reason=payload.dnf_reason,
    )
    apply_progress(entry, payload.status, payload.progress)
    session.add(entry)
    session.flush()
    place_item(session, entry, payload.status, 0)
    entry.book = book
    _status_event(entry, me, session, added=True)
    session.commit()
    loaded = _load_entry(session, entry.id or 0)
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save that book")
    return shelf_item_out(loaded)


@router.post("/import", response_model=GoodreadsImportOut)
async def import_goodreads(
    file: UploadFile = File(...),
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> GoodreadsImportOut:
    raw = await file.read(MAX_IMPORT_BYTES + 1)
    try:
        rows, skips = parse_goodreads_csv(raw)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc

    imported = 0
    looked_up = await lookup_catalog(rows)
    for row, hit, reason in looked_up:
        if hit is None:
            skips.append((row.title, reason or "No match"))
            continue
        book = upsert_book(
            session,
            ol_work_key=hit.ol_work_key,
            title=hit.title,
            authors=hit.authors,
            cover_id=hit.cover_id,
            year=hit.year,
            cover_image_url=hit.cover_url,
        )
        existing = session.exec(
            select(ShelfEntry).where(
                ShelfEntry.user_id == me.id, ShelfEntry.book_id == book.id
            )
        ).first()
        if existing is not None:
            skips.append((row.title, "Already on your shelf"))
            continue
        entry = ShelfEntry(
            user_id=me.id or 0,
            book_id=book.id or 0,
            status=row.status,
            position=0,
        )
        session.add(entry)
        session.flush()
        place_item(session, entry, row.status, 0)
        imported += 1

    session.commit()
    return GoodreadsImportOut(
        imported=imported,
        skipped=len(skips),
        skips=[
            GoodreadsSkipOut(title=title, reason=reason)
            for title, reason in skips[:SKIP_LIST_LIMIT]
        ],
    )


@router.patch("/{entry_id}", response_model=ShelfItemOut)
def update_shelf_item(
    entry_id: int,
    payload: ShelfPatchIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ShelfItemOut:
    if (
        payload.status is None
        and payload.position is None
        and "rating" not in payload.model_fields_set
        and "take" not in payload.model_fields_set
        and "dnf_reason" not in payload.model_fields_set
        and "progress" not in payload.model_fields_set
    ):
        raise HTTPException(status_code=400, detail="Nothing to update")
    entry = _load_entry(session, entry_id)
    if entry is None or entry.user_id != me.id:
        raise HTTPException(status_code=404, detail="Book not on your shelf")
    previous_status = entry.status
    previous_progress = entry.progress
    status = payload.status or entry.status
    position = payload.position if payload.position is not None else entry.position
    if payload.status is not None or any(
        field in payload.model_fields_set for field in ("rating", "take", "dnf_reason")
    ):
        apply_finish_fields(
            entry,
            status,
            rating=payload.rating if "rating" in payload.model_fields_set else entry.rating,
            take=payload.take if "take" in payload.model_fields_set else entry.take,
            dnf_reason=(
                payload.dnf_reason
                if "dnf_reason" in payload.model_fields_set
                else entry.dnf_reason
            ),
        )
    if payload.status is not None or "progress" in payload.model_fields_set:
        apply_progress(
            entry,
            status,
            payload.progress if "progress" in payload.model_fields_set else entry.progress,
        )
    place_item(session, entry, status, position)
    if status != previous_status:
        _status_event(entry, me, session, added=False)
    elif (
        status == ShelfStatus.currently_reading
        and "progress" in payload.model_fields_set
        and entry.progress is not None
        and entry.progress != previous_progress
    ):
        activity.record(session, me, "progress", book=entry.book, progress=entry.progress)
    session.commit()
    loaded = _load_entry(session, entry_id)
    if loaded is None:
        raise HTTPException(status_code=404, detail="Book not on your shelf")
    return shelf_item_out(loaded)


@router.delete("/{entry_id}", status_code=204)
def remove_shelf_item(
    entry_id: int,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> None:
    entry = _load_entry(session, entry_id)
    if entry is None or entry.user_id != me.id:
        raise HTTPException(status_code=404, detail="Book not on your shelf")
    status = entry.status
    activity.record(session, me, "shelf_removed", book=entry.book, status=status.value)
    session.delete(entry)
    session.flush()
    leftovers = session.exec(
        select(ShelfEntry)
        .where(ShelfEntry.user_id == me.id, ShelfEntry.status == status)
        .order_by(ShelfEntry.position, ShelfEntry.id)
    ).all()
    for index, row in enumerate(leftovers):
        row.position = index
    session.commit()
