from app.models import Book, ShelfEntry
from app.schemas import BookOut, ShelfItemOut


def cover_url(cover_id: int | None) -> str | None:
    if cover_id is None:
        return None
    return f"https://covers.openlibrary.org/b/id/{cover_id}-L.jpg"


def book_out(book: Book) -> BookOut:
    return BookOut(
        id=book.id or 0,
        ol_work_key=book.ol_work_key,
        title=book.title,
        authors=book.authors,
        cover_id=book.cover_id,
        year=book.year,
        cover_url=cover_url(book.cover_id),
    )


def shelf_item_out(entry: ShelfEntry) -> ShelfItemOut:
    if entry.book is None:
        raise RuntimeError("Shelf entry is missing its book")
    return ShelfItemOut(
        id=entry.id or 0,
        status=entry.status,
        position=entry.position,
        updated_at=entry.updated_at,
        book=book_out(entry.book),
    )
