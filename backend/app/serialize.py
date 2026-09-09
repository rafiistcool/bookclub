from datetime import timezone

from app.config import get_settings
from app.covers import storable_cover_image_url
from app.i18n import DEFAULT_LOCALE
from app.models import Book, ShelfEntry, User
from app.schemas import BookOut, ShelfItemOut, UserOut
from app.works import isbn_from_work_key


def avatar_url_for(user: User | None) -> str | None:
    if user is None or not user.username:
        return None
    if not user.avatar and not user.avatar_mime:
        return None
    ts = 0
    if user.avatar_updated_at is not None:
        stamp = user.avatar_updated_at
        if stamp.tzinfo is None:
            stamp = stamp.replace(tzinfo=timezone.utc)
        ts = int(stamp.timestamp())
    return f"/api/members/{user.username}/avatar?v={ts}"


def user_out(user: User) -> UserOut:
    return UserOut(
        id=user.id or 0,
        username=user.username,
        theme=user.theme,
        color_mode=user.color_mode,
        locale=user.locale or DEFAULT_LOCALE,
        avatar_url=avatar_url_for(user),
    )


def cover_url(
    cover_id: int | None,
    ol_work_key: str | None = None,
    cover_image_url: str | None = None,
) -> str | None:
    remote = storable_cover_image_url(cover_image_url)
    if remote:
        return remote
    if cover_id is not None and cover_id > 0:
        return f"https://covers.openlibrary.org/b/id/{cover_id}-L.jpg?default=false"
    # Google-only mode: never invent an Open Library ISBN CDN URL.
    if (get_settings().google_books_api_key or "").strip():
        return None
    isbn = isbn_from_work_key(ol_work_key or "")
    if isbn:
        return f"https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false"
    return None


def book_cover_url(book: Book) -> str | None:
    return cover_url(book.cover_id, book.ol_work_key, book.cover_image_url)


def book_out(book: Book) -> BookOut:
    return BookOut(
        id=book.id or 0,
        ol_work_key=book.ol_work_key,
        title=book.title,
        authors=book.authors,
        cover_id=book.cover_id,
        year=book.year,
        cover_url=book_cover_url(book),
        pages=book.pages,
    )


def shelf_item_out(entry: ShelfEntry) -> ShelfItemOut:
    if entry.book is None:
        raise RuntimeError("Shelf entry is missing its book")
    return ShelfItemOut(
        id=entry.id or 0,
        status=entry.status,
        position=entry.position,
        updated_at=entry.updated_at,
        started_at=entry.started_at,
        finished_at=entry.finished_at,
        book=book_out(entry.book),
        rating=entry.rating,
        take=entry.take,
        dnf_reason=entry.dnf_reason,
        progress=entry.progress,
    )
