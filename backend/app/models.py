from datetime import datetime, timezone
from enum import Enum
from typing import Optional

from sqlalchemy import Column, DateTime, UniqueConstraint
from sqlmodel import Field, Relationship, SQLModel


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
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )
    notify_meeting: bool = Field(default=True)
    notify_pick: bool = Field(default=True)
    notify_note: bool = Field(default=True)

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
    description: str = Field(default="")
    pages: Optional[int] = None
    # JSON-encoded list of Open Library subject strings.
    subjects: str = Field(default="[]")
    ol_rating: Optional[float] = None
    details_fetched_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True),
    )

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
    started_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True),
    )
    finished_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True),
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
    reminder_sent_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True),
    )

    book: Optional[Book] = Relationship()
    set_by: Optional[User] = Relationship()
    posts: list["ClubPickPost"] = Relationship(back_populates="pick")
    milestones: list["PickMilestone"] = Relationship(back_populates="pick")


class PickMilestone(SQLModel, table=True):
    """A reading-schedule checkpoint for a club pick ("Chapters 1–10 by Sep 5")."""

    __tablename__ = "pick_milestones"

    id: Optional[int] = Field(default=None, primary_key=True)
    pick_id: int = Field(foreign_key="club_picks.id", index=True)
    title: str = Field(max_length=120)
    chapter_from: Optional[int] = None
    chapter_to: Optional[int] = None
    due_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True),
    )
    position: int = Field(default=0)
    created_by_id: int = Field(foreign_key="users.id")
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    pick: Optional[ClubPick] = Relationship(back_populates="milestones")


class ClubPickPost(SQLModel, table=True):
    __tablename__ = "club_pick_posts"

    id: Optional[int] = Field(default=None, primary_key=True)
    pick_id: int = Field(foreign_key="club_picks.id", index=True)
    author_id: int = Field(foreign_key="users.id")
    body: str = Field(max_length=1000)
    # Percentage of the book this note is safe up to; None = no spoilers flagged.
    spoiler_upto: Optional[int] = Field(default=None)
    milestone_id: Optional[int] = Field(default=None, foreign_key="pick_milestones.id")
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )
    updated_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True),
    )

    pick: Optional[ClubPick] = Relationship(back_populates="posts")
    author: Optional[User] = Relationship()
    reactions: list["PostReaction"] = Relationship(back_populates="post")


class PostReaction(SQLModel, table=True):
    __tablename__ = "post_reactions"
    __table_args__ = (
        UniqueConstraint("post_id", "user_id", "emoji", name="uq_reaction_post_user_emoji"),
    )

    id: Optional[int] = Field(default=None, primary_key=True)
    post_id: int = Field(foreign_key="club_pick_posts.id", index=True)
    user_id: int = Field(foreign_key="users.id")
    emoji: str = Field(max_length=16)
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    post: Optional[ClubPickPost] = Relationship(back_populates="reactions")
    user: Optional[User] = Relationship()


class Event(SQLModel, table=True):
    """Append-only club activity log feeding the activity feed."""

    __tablename__ = "events"

    id: Optional[int] = Field(default=None, primary_key=True)
    actor_id: int = Field(foreign_key="users.id", index=True)
    kind: str = Field(max_length=32, index=True)
    book_id: Optional[int] = Field(default=None, foreign_key="books.id")
    pick_id: Optional[int] = Field(default=None, foreign_key="club_picks.id")
    # JSON-encoded extra fields (status, rating, take, count …).
    payload: str = Field(default="{}")
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False, index=True),
    )

    actor: Optional[User] = Relationship()
    book: Optional[Book] = Relationship()


class PushSubscription(SQLModel, table=True):
    __tablename__ = "push_subscriptions"

    id: Optional[int] = Field(default=None, primary_key=True)
    user_id: int = Field(foreign_key="users.id", index=True)
    endpoint: str = Field(unique=True, max_length=1024)
    p256dh: str = Field(max_length=256)
    auth: str = Field(max_length=128)
    user_agent: str = Field(default="", max_length=256)
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    user: Optional[User] = Relationship()


class Quote(SQLModel, table=True):
    __tablename__ = "quotes"

    id: Optional[int] = Field(default=None, primary_key=True)
    user_id: int = Field(foreign_key="users.id", index=True)
    book_id: int = Field(foreign_key="books.id", index=True)
    body: str = Field(max_length=1000)
    page: Optional[int] = None
    created_at: datetime = Field(
        default_factory=utcnow,
        sa_column=Column(DateTime(timezone=True), nullable=False),
    )

    user: Optional[User] = Relationship()
    book: Optional[Book] = Relationship()


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
    closes_at: Optional[datetime] = Field(
        default=None,
        sa_column=Column(DateTime(timezone=True), nullable=True),
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
