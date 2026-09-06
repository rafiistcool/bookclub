from sqlmodel import Session, col, select

from app.models import Book, ShelfEntry, ShelfStatus, utcnow


def upsert_book(
    session: Session,
    *,
    ol_work_key: str,
    title: str,
    authors: str,
    cover_id: int | None,
    year: int | None,
) -> Book:
    book = session.exec(select(Book).where(Book.ol_work_key == ol_work_key)).first()
    if book is None:
        book = Book(
            ol_work_key=ol_work_key,
            title=title,
            authors=authors,
            cover_id=cover_id,
            year=year,
        )
        session.add(book)
        session.flush()
        return book
    book.title = title
    book.authors = authors
    book.cover_id = cover_id
    book.year = year
    return book


def place_item(
    session: Session,
    item: ShelfEntry,
    status: ShelfStatus,
    position: int,
) -> None:
    source_status = item.status
    source_changed = source_status != status

    siblings = session.exec(
        select(ShelfEntry)
        .where(
            ShelfEntry.user_id == item.user_id,
            ShelfEntry.status == status,
            col(ShelfEntry.id) != item.id,
        )
        .order_by(ShelfEntry.position, ShelfEntry.id)
    ).all()
    insert_at = max(0, min(position, len(siblings)))
    siblings.insert(insert_at, item)
    item.status = status
    item.updated_at = utcnow()
    apply_reading_dates(item, status, changed=source_changed)
    for index, row in enumerate(siblings):
        row.position = index

    if source_changed:
        leftovers = session.exec(
            select(ShelfEntry)
            .where(
                ShelfEntry.user_id == item.user_id,
                ShelfEntry.status == source_status,
                col(ShelfEntry.id) != item.id,
            )
            .order_by(ShelfEntry.position, ShelfEntry.id)
        ).all()
        for index, row in enumerate(leftovers):
            row.position = index


def apply_finish_fields(
    entry: ShelfEntry,
    status: ShelfStatus,
    *,
    rating: int | None = None,
    take: str | None = None,
    dnf_reason: str | None = None,
) -> None:
    if status == ShelfStatus.finished:
        entry.rating = rating
        entry.take = (take or "").strip()
        entry.dnf_reason = ""
        return
    if status == ShelfStatus.did_not_finish:
        entry.rating = None
        entry.take = ""
        entry.dnf_reason = (dnf_reason or "").strip()
        return
    entry.rating = None
    entry.take = ""
    entry.dnf_reason = ""


def apply_progress(entry: ShelfEntry, status: ShelfStatus, progress: int | None) -> None:
    if status == ShelfStatus.currently_reading:
        entry.progress = progress
        return
    entry.progress = None


def apply_reading_dates(entry: ShelfEntry, status: ShelfStatus, *, changed: bool) -> None:
    """Stamp started_at / finished_at on status transitions.

    started_at is set the first time a book enters Reading (or lands directly
    in Finished / DNF without passing through Reading) and survives later
    moves so a re-read keeps its original start. finished_at only exists
    while the book sits in Finished. Moving back to Want to read clears both
    so the book reads as fresh again.
    """
    now = utcnow()
    if status == ShelfStatus.want_to_read:
        if changed:
            entry.started_at = None
            entry.finished_at = None
        return
    if entry.started_at is None:
        entry.started_at = now
    if status == ShelfStatus.finished:
        if changed or entry.finished_at is None:
            entry.finished_at = now
        return
    entry.finished_at = None
