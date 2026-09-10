from fastapi import HTTPException
from sqlalchemy.orm import selectinload
from sqlmodel import Session, col, select

from app.models import FAVORITE_LIMIT, Book, User, UserFavorite
from app.schemas import FavoriteOut, FavoritesOut
from app.serialize import book_out

BOOK_NOT_IN_CLUB = "That book is not in the club yet"
FAVOURITES_FULL = "You already have 3 favourites"
TOO_MANY_FAVOURITES = "At most 3 favourites"


def favorite_out(row: UserFavorite) -> FavoriteOut | None:
    if row.book is None:
        return None
    return FavoriteOut(position=row.position, book=book_out(row.book))


def favorites_out(rows: list[UserFavorite]) -> FavoritesOut:
    items = [item for row in rows if (item := favorite_out(row)) is not None]
    return FavoritesOut(items=items)


def list_favorites(session: Session, user: User) -> list[UserFavorite]:
    return list(
        session.exec(
            select(UserFavorite)
            .where(UserFavorite.user_id == user.id)
            .options(selectinload(UserFavorite.book))
            .order_by(UserFavorite.position, UserFavorite.id)
        ).all()
    )


def favorite_position_for(session: Session, user: User, book_id: int | None) -> int | None:
    if book_id is None:
        return None
    row = session.exec(
        select(UserFavorite).where(
            UserFavorite.user_id == user.id, UserFavorite.book_id == book_id
        )
    ).first()
    return row.position if row is not None else None


def _require_club_books(session: Session, book_ids: list[int]) -> dict[int, Book]:
    if not book_ids:
        return {}
    books = session.exec(select(Book).where(col(Book.id).in_(book_ids))).all()
    by_id = {book.id: book for book in books if book.id is not None}
    missing = [book_id for book_id in book_ids if book_id not in by_id]
    if missing:
        raise HTTPException(status_code=400, detail=BOOK_NOT_IN_CLUB)
    return by_id


def replace_favorites(session: Session, user: User, book_ids: list[int]) -> list[UserFavorite]:
    if len(book_ids) > FAVORITE_LIMIT:
        raise HTTPException(status_code=400, detail=TOO_MANY_FAVOURITES)
    _require_club_books(session, book_ids)
    existing = session.exec(
        select(UserFavorite).where(UserFavorite.user_id == user.id)
    ).all()
    for row in existing:
        session.delete(row)
    session.flush()
    for position, book_id in enumerate(book_ids, start=1):
        session.add(
            UserFavorite(
                user_id=user.id or 0,
                book_id=book_id,
                position=position,
            )
        )
    session.commit()
    return list_favorites(session, user)


def add_favorite(session: Session, user: User, book_id: int) -> list[UserFavorite]:
    _require_club_books(session, [book_id])
    current = list_favorites(session, user)
    if any(row.book_id == book_id for row in current):
        return current
    if len(current) >= FAVORITE_LIMIT:
        raise HTTPException(status_code=409, detail=FAVOURITES_FULL)
    session.add(
        UserFavorite(
            user_id=user.id or 0,
            book_id=book_id,
            position=len(current) + 1,
        )
    )
    session.commit()
    return list_favorites(session, user)


def remove_favorite(session: Session, user: User, book_id: int) -> list[UserFavorite]:
    current = list_favorites(session, user)
    remaining = [row.book_id for row in current if row.book_id != book_id]
    if len(remaining) == len(current):
        return current
    return replace_favorites(session, user, remaining)
