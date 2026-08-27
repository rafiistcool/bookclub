from collections import defaultdict

from fastapi import APIRouter, Depends, Query
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app.deps import get_current_user, get_session
from app.models import ShelfEntry, ShelfStatus, User
from app.schemas import OverlapBookOut, OverlapMember, OverlapOut
from app.serialize import book_out

router = APIRouter(prefix="/api/overlap", tags=["overlap"])


@router.get("", response_model=OverlapOut)
def tbr_overlap(
    include_reading: bool = Query(default=False),
    me: User = Depends(get_current_user),
    session: Session = Depends(get_session),
) -> OverlapOut:
    statuses = [ShelfStatus.want_to_read]
    if include_reading:
        statuses.append(ShelfStatus.currently_reading)
    entries = session.exec(
        select(ShelfEntry)
        .where(col(ShelfEntry.status).in_(statuses))
        .options(selectinload(ShelfEntry.book), selectinload(ShelfEntry.user))
    ).all()
    by_book: dict[int, list[ShelfEntry]] = defaultdict(list)
    for entry in entries:
        if entry.book is None or entry.user is None:
            continue
        by_book[entry.book_id].append(entry)

    items: list[OverlapBookOut] = []
    for rows in by_book.values():
        people: dict[str, ShelfStatus] = {}
        book = None
        for row in rows:
            if row.user is None or row.book is None:
                continue
            people[row.user.username] = row.status
            book = row.book
        if book is None or len(people) < 2:
            continue
        members = [
            OverlapMember(username=name, status=status)
            for name, status in sorted(people.items())
        ]
        items.append(
            OverlapBookOut(book=book_out(book), count=len(members), members=members)
        )
    items.sort(key=lambda row: (-row.count, row.book.title.lower()))
    return OverlapOut(items=items, include_reading=include_reading)
