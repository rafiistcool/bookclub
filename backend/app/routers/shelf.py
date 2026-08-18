from fastapi import APIRouter, Depends, HTTPException, Query
from fastapi.responses import JSONResponse
from sqlalchemy.orm import selectinload
from sqlmodel import Session, select

from app.deps import get_current_user, get_session
from app.models import ShelfEntry, User
from app.schemas import ShelfAddIn, ShelfItemOut, ShelfListOut, ShelfPatchIn, UserOut
from app.serialize import shelf_item_out
from app.shelf_ops import place_item, upsert_book

router = APIRouter(prefix="/api/shelf", tags=["shelf"])


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
        user=UserOut(id=owner.id or 0, username=owner.username),
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
    session.add(entry)
    session.flush()
    place_item(session, entry, payload.status, 0)
    session.commit()
    loaded = _load_entry(session, entry.id or 0)
    if loaded is None:
        raise HTTPException(status_code=500, detail="Could not save that book")
    return shelf_item_out(loaded)


@router.patch("/{entry_id}", response_model=ShelfItemOut)
def update_shelf_item(
    entry_id: int,
    payload: ShelfPatchIn,
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> ShelfItemOut:
    if payload.status is None and payload.position is None:
        raise HTTPException(status_code=400, detail="Nothing to update")
    entry = _load_entry(session, entry_id)
    if entry is None or entry.user_id != me.id:
        raise HTTPException(status_code=404, detail="Book not on your shelf")
    status = payload.status or entry.status
    position = payload.position if payload.position is not None else entry.position
    place_item(session, entry, status, position)
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
    entry = session.get(ShelfEntry, entry_id)
    if entry is None or entry.user_id != me.id:
        raise HTTPException(status_code=404, detail="Book not on your shelf")
    status = entry.status
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
