from datetime import datetime, timezone
from enum import Enum
from typing import Optional

from sqlalchemy import Column, DateTime, UniqueConstraint
from sqlmodel import Field, Relationship, SQLModel

from app.themes import DEFAULT_COLOR_MODE, DEFAULT_THEME_ID


def utcnow() -> datetime:
    return datetime.now(timezone.utc)


class ShelfStatus(str, Enum):
    want_to_read = "want_to_read"
    currently_reading = "currently_reading"
    finished = "finished"
    did_not_finish = "did_not_finish"


class User(SQLModel, table=True):
    __tablename__ = "users"

    id: Optional[int] = Field(default=None, primary_key=True)
    username: str = Field(unique=True, index=True, max_length=32)
    password_hash: str
    theme: str = Field(default=DEFAULT_THEME_ID, max_length=32)
    color_mode: str = Field(default=DEFAULT_COLOR_MODE, max_length=16)
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    shelf_entries: list["ShelfEntry"] = Relationship(back_populates="user")


class Invite(SQLModel, table=True):
    __tablename__ = "invites"

    id: Optional[int] = Field(default=None, primary_key=True)
    code: str = Field(unique=True, index=True, max_length=32)
    created_by_id: Optional[int] = Field(default=None, foreign_key="users.id")
    used_by_id: Optional[int] = Field(default=None, foreign_key="users.id")
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )
    used_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True),
    )


class Book(SQLModel, table=True):
    __tablename__ = "books"

    id: Optional[int] = Field(default=None, primary_key=True)
    ol_work_key: str = Field(unique=True, index=True, max_length=64)
    title: str
    authors: str = ""
    cover_id: Optional[int] = None
    year: Optional[int] = None

    shelf_entries: list["ShelfEntry"] = Relationship(back_populates="book")


class ShelfEntry(SQLModel, table=True):
    __tablename__ = "shelf"
    __table_args__ = (
        UniqueConstraint("user_id", "book_id", name="uq_shelf_user_book"),
    )

    id: Optional[int] = Field(default=None, primary_key=True)
    user_id: int = Field(foreign_key="users.id", index=True)
    book_id: int = Field(foreign_key="books.id")
    status: ShelfStatus = Field(index=True)
    position: int = Field(default=0)
    rating: Optional[int] = Field(default=None)
    take: str = Field(default="", max_length=140)
    dnf_reason: str = Field(default="", max_length=200)
    progress: Optional[int] = Field(default=None)
    updated_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    user: Optional[User] = Relationship(back_populates="shelf_entries")
    book: Optional[Book] = Relationship(back_populates="shelf_entries")


class ClubPick(SQLModel, table=True):
    __tablename__ = "club_picks"

    id: Optional[int] = Field(default=None, primary_key=True)
    book_id: int = Field(foreign_key="books.id", index=True)
    set_by_id: int = Field(foreign_key="users.id")
    note: str = Field(default="", max_length=280)
    started_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )
    ended_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True, index=True),
    )
    meeting_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True),
    )

    book: Optional[Book] = Relationship()
    set_by: Optional[User] = Relationship()
    posts: list["ClubPickPost"] = Relationship(back_populates="pick")


class ClubPickPost(SQLModel, table=True):
    __tablename__ = "club_pick_posts"

    id: Optional[int] = Field(default=None, primary_key=True)
    pick_id: int = Field(foreign_key="club_picks.id", index=True)
    author_id: int = Field(foreign_key="users.id")
    body: str = Field(max_length=1000)
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    pick: Optional[ClubPick] = Relationship(back_populates="posts")
    author: Optional[User] = Relationship()


NOMINATION_LIMIT = 6


class NextUpVote(SQLModel, table=True):
    __tablename__ = "next_up_votes"

    id: Optional[int] = Field(default=None, primary_key=True)
    started_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )
    ended_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True, index=True),
    )
    winner_nomination_id: Optional[int] = Field(default=None)

    nominations: list["NextUpNomination"] = Relationship(back_populates="vote")
    ballots: list["NextUpBallot"] = Relationship(back_populates="vote")


class NextUpNomination(SQLModel, table=True):
    __tablename__ = "next_up_nominations"
    __table_args__ = (
        UniqueConstraint("vote_id", "book_id", name="uq_next_up_vote_book"),
    )

    id: Optional[int] = Field(default=None, primary_key=True)
    vote_id: int = Field(foreign_key="next_up_votes.id", index=True)
    book_id: int = Field(foreign_key="books.id")
    nominated_by_id: int = Field(foreign_key="users.id")
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    vote: Optional[NextUpVote] = Relationship(back_populates="nominations")
    book: Optional[Book] = Relationship()
    nominated_by: Optional[User] = Relationship()
    ballots: list["NextUpBallot"] = Relationship(back_populates="nomination")


class NextUpBallot(SQLModel, table=True):
    __tablename__ = "next_up_ballots"
    __table_args__ = (
        UniqueConstraint("vote_id", "user_id", name="uq_next_up_vote_user"),
    )

    id: Optional[int] = Field(default=None, primary_key=True)
    vote_id: int = Field(foreign_key="next_up_votes.id", index=True)
    nomination_id: int = Field(foreign_key="next_up_nominations.id", index=True)
    user_id: int = Field(foreign_key="users.id")
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    vote: Optional[NextUpVote] = Relationship(back_populates="ballots")
    nomination: Optional[NextUpNomination] = Relationship(back_populates="ballots")
    user: Optional[User] = Relationship()
