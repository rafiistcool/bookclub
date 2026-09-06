from pathlib import Path

from sqlalchemy import event, text
from sqlmodel import Session, SQLModel, create_engine, select
from sqlalchemy.engine import Engine

from app.config import Settings
from app.models import (
    BookPost,
    BookPostReaction,
    ClubPick,
    ClubPickPost,
    Invite,
    PostReaction,
)
from app.security import normalize_invite_code
from app.themes import DEFAULT_COLOR_MODE, DEFAULT_THEME_ID


def init_db(path: Path) -> Engine:
    path.parent.mkdir(parents=True, exist_ok=True)
    engine = create_engine(
        f"sqlite:///{path}",
        connect_args={"check_same_thread": False},
    )

    @event.listens_for(engine, "connect")
    def _set_sqlite_pragma(dbapi_connection, _connection_record) -> None:  # type: ignore[no-untyped-def]
        cursor = dbapi_connection.cursor()
        cursor.execute("PRAGMA journal_mode=WAL")
        cursor.execute("PRAGMA foreign_keys=ON")
        cursor.close()

    SQLModel.metadata.create_all(engine)
    apply_migrations(engine)
    import_legacy_pick_posts(engine)
    return engine


# Columns added after a table first shipped. `create_all` only creates missing
# tables, so every new column on an existing table is listed here with the DDL
# SQLite needs. Order within a table does not matter; entries are idempotent.
COLUMN_MIGRATIONS: dict[str, dict[str, str]] = {
    "users": {
        # SQLite fills existing rows from the DEFAULT clause, so users created
        # before this migration read back as paper/system rather than NULL.
        "theme": f"VARCHAR(32) NOT NULL DEFAULT '{DEFAULT_THEME_ID}'",
        "color_mode": f"VARCHAR(16) NOT NULL DEFAULT '{DEFAULT_COLOR_MODE}'",
        "notify_meeting": "BOOLEAN NOT NULL DEFAULT 1",
        "notify_pick": "BOOLEAN NOT NULL DEFAULT 1",
        "notify_note": "BOOLEAN NOT NULL DEFAULT 1",
    },
    "books": {
        "description": "VARCHAR NOT NULL DEFAULT ''",
        "pages": "INTEGER",
        "subjects": "VARCHAR NOT NULL DEFAULT '[]'",
        "ol_rating": "FLOAT",
        "details_fetched_at": "DATETIME",
    },
    "shelf": {
        "rating": "INTEGER",
        "take": "VARCHAR DEFAULT ''",
        "dnf_reason": "VARCHAR DEFAULT ''",
        "progress": "INTEGER",
        "started_at": "DATETIME",
        "finished_at": "DATETIME",
    },
    "club_picks": {
        "meeting_at": "DATETIME",
        "reminder_sent_at": "DATETIME",
    },
    "club_pick_posts": {
        "spoiler_upto": "INTEGER",
        "milestone_id": "INTEGER REFERENCES pick_milestones(id)",
        "updated_at": "DATETIME",
    },
    "next_up_votes": {
        "closes_at": "DATETIME",
    },
}


def missing_columns(engine: Engine) -> dict[str, list[str]]:
    """Report columns from COLUMN_MIGRATIONS that the live schema lacks."""
    missing: dict[str, list[str]] = {}
    with engine.begin() as conn:
        for table, columns in COLUMN_MIGRATIONS.items():
            rows = conn.execute(text(f"PRAGMA table_info({table})")).fetchall()
            if not rows:
                continue
            present = {row[1] for row in rows}
            absent = [name for name in columns if name not in present]
            if absent:
                missing[table] = absent
    return missing


def apply_migrations(engine: Engine) -> None:
    with engine.begin() as conn:
        for table, columns in COLUMN_MIGRATIONS.items():
            rows = conn.execute(text(f"PRAGMA table_info({table})")).fetchall()
            if not rows:
                continue
            present = {row[1] for row in rows}
            for name, ddl in columns.items():
                if name in present:
                    continue
                conn.execute(text(f"ALTER TABLE {table} ADD COLUMN {name} {ddl}"))


def import_legacy_pick_posts(engine: Engine) -> int:
    """Copy pre-diary club pick notes into the per-book diary, once.

    Notes used to hang off a club pick (`club_pick_posts`); the diary hangs
    off the book. Each legacy row becomes a top-level entry on the pick's
    book with the pick as context, and its reactions come along. The
    `legacy_post_id` marker makes this safe to run on every start.
    """
    with Session(engine) as session:
        done = {
            row
            for row in session.exec(
                select(BookPost.legacy_post_id).where(BookPost.legacy_post_id.is_not(None))
            ).all()
        }
        pending = session.exec(
            select(ClubPickPost, ClubPick)
            .join(ClubPick, ClubPick.id == ClubPickPost.pick_id)
            .order_by(ClubPickPost.created_at, ClubPickPost.id)
        ).all()
        imported = 0
        for legacy, pick in pending:
            if legacy.id in done:
                continue
            post = BookPost(
                book_id=pick.book_id,
                author_id=legacy.author_id,
                body=legacy.body,
                spoiler_upto=legacy.spoiler_upto,
                pick_id=pick.id,
                legacy_post_id=legacy.id,
                created_at=legacy.created_at,
                updated_at=legacy.updated_at,
            )
            session.add(post)
            session.flush()
            for reaction in session.exec(
                select(PostReaction).where(PostReaction.post_id == legacy.id)
            ).all():
                session.add(
                    BookPostReaction(
                        post_id=post.id or 0,
                        user_id=reaction.user_id,
                        emoji=reaction.emoji,
                        created_at=reaction.created_at,
                    )
                )
            imported += 1
        session.commit()
    return imported


def ensure_bootstrap_invite(session: Session, settings: Settings) -> str | None:
    existing = session.exec(select(Invite).limit(1)).first()
    if existing is not None:
        return None
    code = normalize_invite_code(settings.bookclub_bootstrap_invite)
    if not code:
        raise RuntimeError("BOOKCLUB_BOOTSTRAP_INVITE is empty")
    if not settings.debug and code == "DEVONLY":
        raise RuntimeError("Set BOOKCLUB_BOOTSTRAP_INVITE to a secret code when DEBUG=0")
    session.add(Invite(code=code, created_by_id=None))
    session.commit()
    return code
