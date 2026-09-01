import re
from datetime import datetime

from pydantic import BaseModel, Field, field_validator

from app.models import ShelfStatus

USERNAME_RE = re.compile(r"^[a-z0-9_]{2,32}$")


class UserOut(BaseModel):
    id: int
    username: str


class RegisterIn(BaseModel):
    username: str
    password: str
    invite_code: str

    @field_validator("username")
    @classmethod
    def username_ok(cls, value: str) -> str:
        value = value.strip().lower()
        if not USERNAME_RE.fullmatch(value):
            raise ValueError(
                "Username must be 2–32 characters: lowercase letters, numbers, underscore"
            )
        return value

    @field_validator("password")
    @classmethod
    def password_ok(cls, value: str) -> str:
        if len(value) < 8:
            raise ValueError("Password must be at least 8 characters")
        return value


class LoginIn(BaseModel):
    username: str
    password: str

    @field_validator("username")
    @classmethod
    def username_ok(cls, value: str) -> str:
        return value.strip().lower()


class PasswordChangeIn(BaseModel):
    current_password: str
    new_password: str

    @field_validator("new_password")
    @classmethod
    def password_ok(cls, value: str) -> str:
        if len(value) < 8:
            raise ValueError("New password must be at least 8 characters")
        return value


class NotificationPrefsIn(BaseModel):
    notify_meeting: bool | None = None
    notify_pick: bool | None = None
    notify_note: bool | None = None


class NotificationPrefsOut(BaseModel):
    notify_meeting: bool
    notify_pick: bool
    notify_note: bool


class InviteOut(BaseModel):
    code: str
    used: bool
    used_by: str | None
    created_at: datetime


class InviteCreated(BaseModel):
    code: str


class BookOut(BaseModel):
    id: int
    ol_work_key: str
    title: str
    authors: str
    cover_id: int | None
    year: int | None
    cover_url: str | None
    pages: int | None = None


class SearchHit(BaseModel):
    ol_work_key: str
    title: str
    authors: str
    cover_id: int | None
    year: int | None
    on_shelf: ShelfStatus | None = None
    shelf_id: int | None = None
    club_pick: bool = False


class SearchPage(BaseModel):
    items: list[SearchHit]
    page: int
    has_more: bool


class BookMember(BaseModel):
    username: str
    status: ShelfStatus
    rating: int | None = None
    take: str = ""
    progress: int | None = None
    finished_at: datetime | None = None


class BookDetailsOut(BaseModel):
    ol_work_key: str
    title: str
    authors: str
    cover_id: int | None
    year: int | None
    cover_url: str | None
    description: str
    pages: int | None
    subjects: list[str]
    ol_rating: float | None = None
    ol_rating_count: int | None = None
    on_shelf: ShelfStatus | None = None
    shelf_id: int | None = None
    club_pick: bool = False
    members: list[BookMember] = []
    quote_count: int = 0


class IsbnHitOut(BaseModel):
    isbn: str
    ol_work_key: str
    title: str
    authors: str
    cover_id: int | None
    year: int | None
    pages: int | None
    on_shelf: ShelfStatus | None = None
    shelf_id: int | None = None


class ShelfItemOut(BaseModel):
    id: int
    status: ShelfStatus
    position: int
    updated_at: datetime
    started_at: datetime | None = None
    finished_at: datetime | None = None
    book: BookOut
    rating: int | None = None
    take: str = ""
    dnf_reason: str = ""
    progress: int | None = None


class ShelfAddIn(BaseModel):
    ol_work_key: str
    title: str
    authors: str = ""
    cover_id: int | None = None
    year: int | None = None
    status: ShelfStatus = ShelfStatus.want_to_read
    rating: int | None = Field(default=None, ge=1, le=5)
    take: str = ""
    dnf_reason: str = ""
    progress: int | None = Field(default=None, ge=0, le=100)

    @field_validator("ol_work_key")
    @classmethod
    def work_key_ok(cls, value: str) -> str:
        value = value.strip()
        if not value.startswith("/works/"):
            raise ValueError("ol_work_key must be an Open Library work key")
        return value

    @field_validator("title")
    @classmethod
    def title_ok(cls, value: str) -> str:
        value = value.strip()
        if not value:
            raise ValueError("title is required")
        return value

    @field_validator("take")
    @classmethod
    def take_ok(cls, value: str) -> str:
        value = value.strip()
        if len(value) > 140:
            raise ValueError("Keep the take to 140 characters")
        return value

    @field_validator("dnf_reason")
    @classmethod
    def dnf_ok(cls, value: str) -> str:
        value = value.strip()
        if len(value) > 200:
            raise ValueError("Keep the reason to 200 characters")
        return value


class ShelfPatchIn(BaseModel):
    status: ShelfStatus | None = None
    position: int | None = Field(default=None, ge=0)
    rating: int | None = Field(default=None, ge=1, le=5)
    take: str | None = None
    dnf_reason: str | None = None
    progress: int | None = Field(default=None, ge=0, le=100)

    @field_validator("take")
    @classmethod
    def take_ok(cls, value: str | None) -> str | None:
        if value is None:
            return None
        value = value.strip()
        if len(value) > 140:
            raise ValueError("Keep the take to 140 characters")
        return value

    @field_validator("dnf_reason")
    @classmethod
    def dnf_ok(cls, value: str | None) -> str | None:
        if value is None:
            return None
        value = value.strip()
        if len(value) > 200:
            raise ValueError("Keep the reason to 200 characters")
        return value


class ShelfListOut(BaseModel):
    user: UserOut
    items: list[ShelfItemOut]


class GoodreadsSkipOut(BaseModel):
    title: str
    reason: str


class GoodreadsImportOut(BaseModel):
    imported: int
    skipped: int
    skips: list[GoodreadsSkipOut]


class ReadingPreview(BaseModel):
    title: str
    cover_id: int | None


class MemberOut(BaseModel):
    username: str
    currently_reading_count: int
    currently_reading_preview: list[ReadingPreview]


class ClubPickSetIn(BaseModel):
    ol_work_key: str
    title: str
    authors: str = ""
    cover_id: int | None = None
    year: int | None = None
    note: str = ""
    meeting_at: str | None = None

    @field_validator("ol_work_key")
    @classmethod
    def work_key_ok(cls, value: str) -> str:
        value = value.strip()
        if not value.startswith("/works/"):
            raise ValueError("ol_work_key must be an Open Library work key")
        return value

    @field_validator("title")
    @classmethod
    def title_ok(cls, value: str) -> str:
        value = value.strip()
        if not value:
            raise ValueError("title is required")
        return value

    @field_validator("note")
    @classmethod
    def note_ok(cls, value: str) -> str:
        value = value.strip()
        if len(value) > 280:
            raise ValueError("note must be 280 characters or fewer")
        return value


class ClubPickReader(BaseModel):
    username: str
    status: ShelfStatus
    rating: int | None = None
    take: str = ""
    dnf_reason: str = ""
    progress: int | None = None


class ClubPickOut(BaseModel):
    id: int
    book: BookOut
    set_by: str
    note: str
    started_at: datetime
    ended_at: datetime | None = None
    meeting_at: datetime | None = None
    meeting_local: str | None = None
    meeting_label: str | None = None
    on_shelf: ShelfStatus | None = None
    shelf_id: int | None = None
    readers: list[ClubPickReader] = []
    reading: list[str] = []
    finished: list[str] = []


class ClubPickCurrentOut(BaseModel):
    pick: ClubPickOut | None
    timezone: str


class ClubPickHistoryOut(BaseModel):
    items: list[ClubPickOut]
    timezone: str


class OverlapMember(BaseModel):
    username: str
    status: ShelfStatus


class OverlapBookOut(BaseModel):
    book: BookOut
    count: int
    members: list[OverlapMember]


class OverlapOut(BaseModel):
    items: list[OverlapBookOut]
    include_reading: bool


def _post_body_ok(value: str) -> str:
    value = value.strip()
    if not value:
        raise ValueError("Write a short take, quote, or meeting note")
    if len(value) > 1000:
        raise ValueError("Keep it to 1000 characters")
    return value


class PickPostIn(BaseModel):
    body: str
    # Percent of the book this note is safe up to (None = no spoiler flag).
    spoiler_upto: int | None = Field(default=None, ge=0, le=100)
    milestone_id: int | None = None

    @field_validator("body")
    @classmethod
    def body_ok(cls, value: str) -> str:
        return _post_body_ok(value)


class PickPostPatchIn(BaseModel):
    body: str | None = None
    spoiler_upto: int | None = Field(default=None, ge=0, le=100)

    @field_validator("body")
    @classmethod
    def body_ok(cls, value: str | None) -> str | None:
        if value is None:
            return None
        return _post_body_ok(value)


REACTIONS = ("❤️", "👍", "😂", "😮", "🔥", "📚")


class ReactionIn(BaseModel):
    emoji: str

    @field_validator("emoji")
    @classmethod
    def emoji_ok(cls, value: str) -> str:
        value = value.strip()
        if value not in REACTIONS:
            raise ValueError("Pick one of the club reactions")
        return value


class ReactionOut(BaseModel):
    emoji: str
    count: int
    mine: bool
    users: list[str]


class PickPostOut(BaseModel):
    id: int
    author: str
    mine: bool
    body: str
    spoiler_upto: int | None = None
    milestone_id: int | None = None
    created_at: datetime
    created_label: str
    edited: bool = False
    reactions: list[ReactionOut] = []


class PickPostListOut(BaseModel):
    pick_id: int
    can_post: bool
    # The viewer's own reading progress for this pick, so the client can blur
    # notes flagged past where they are.
    my_progress: int | None = None
    my_status: ShelfStatus | None = None
    items: list[PickPostOut]
    timezone: str


class VoteNominateIn(BaseModel):
    ol_work_key: str
    title: str
    authors: str = ""
    cover_id: int | None = None
    year: int | None = None

    @field_validator("ol_work_key")
    @classmethod
    def work_key_ok(cls, value: str) -> str:
        value = value.strip()
        if not value.startswith("/works/"):
            raise ValueError("ol_work_key must be an Open Library work key")
        return value

    @field_validator("title")
    @classmethod
    def title_ok(cls, value: str) -> str:
        value = value.strip()
        if not value:
            raise ValueError("title is required")
        return value


class VoteCastIn(BaseModel):
    nomination_id: int


class VoteApplyIn(BaseModel):
    nomination_id: int
    meeting_at: str | None = None


class VoteNominationOut(BaseModel):
    id: int
    book: BookOut
    nominated_by: str
    votes: int
    voters: list[str]
    mine: bool


class VoteOut(BaseModel):
    vote_id: int | None
    nominations: list[VoteNominationOut]
    my_vote_id: int | None
    nomination_limit: int
    can_nominate: bool
    timezone: str
    closes_at: datetime | None = None
    closes_local: str | None = None
    closes_label: str | None = None
    # Members who have not cast a ballot in the open vote.
    not_voted: list[str] = []
    voted_count: int = 0
    member_count: int = 0
    # Nomination currently in the lead (None on a tie at zero or no nominations).
    leader_id: int | None = None


class VoteDeadlineIn(BaseModel):
    closes_at: str | None = None


class VoteApplyOut(BaseModel):
    pick: ClubPickOut
    vote: VoteOut


class VoteSuggestionOut(BaseModel):
    book: BookOut
    score: int
    reasons: list[str]


class VoteSuggestionsOut(BaseModel):
    items: list[VoteSuggestionOut]
