from sqlmodel import Session, col, select

from app.covers import storable_cover_image_url
from app.models import Book, ShelfEntry, ShelfStatus, utcnow
from app.openlibrary import subjects_to_json
from app.works import work_id_from_key, work_key


def _usable_cover(cover_id: int | None) -> int | None:
    return cover_id if cover_id is not None and cover_id > 0 else None


def upsert_book(
    session: Session,
    *,
    ol_work_key: str,
    title: str,
    authors: str,
    cover_id: int | None,
    year: int | None,
    cover_image_url: str | None = None,
) -> Book:
    ol_work_key = work_key(work_id_from_key(ol_work_key))
    cover_id = _usable_cover(cover_id)
    cover_image_url = storable_cover_image_url(cover_image_url)
    book = session.exec(select(Book).where(Book.ol_work_key == ol_work_key)).first()
    if book is None:
        book = Book(
            ol_work_key=ol_work_key,
            title=title,
            authors=authors,
            cover_id=cover_id,
            cover_image_url=cover_image_url,
            year=year,
        )
        session.add(book)
        session.flush()
        return book
    book.title = title
    if authors:
        book.authors = authors
    if cover_id is not None:
        book.cover_id = cover_id
    if cover_image_url is not None:
        book.cover_image_url = cover_image_url
    if year is not None:
        book.year = year
    return book


def apply_book_details(
    book: Book,
    *,
    title: str = "",
    authors: str = "",
    cover_id: int | None = None,
    year: int | None = None,
    description: str | None = None,
    pages: int | None = None,
    subjects: list[str] | None = None,
    ol_rating: float | None = None,
    cover_image_url: str | None = None,
) -> Book:
    """Copy catalog metadata onto a local Book. Empty strings do not wipe title/authors."""
    if title:
        book.title = title
    if authors:
        book.authors = authors
    cover_id = _usable_cover(cover_id)
    if cover_id is not None:
        book.cover_id = cover_id
    remote = storable_cover_image_url(cover_image_url)
    if remote is not None:
        book.cover_image_url = remote
    if year is not None:
        book.year = year
    if description is not None:
        book.description = description
    if pages is not None:
        book.pages = pages
    if subjects is not None:
        book.subjects = subjects_to_json(subjects)
    if ol_rating is not None:
        book.ol_rating = ol_rating
    book.details_fetched_at = utcnow()
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
