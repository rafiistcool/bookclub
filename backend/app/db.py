from pathlib import Path

from sqlalchemy import event, text
from sqlmodel import Session, SQLModel, create_engine, select
from sqlalchemy.engine import Engine

from app.config import Settings
from app.models import Invite
from app.security import normalize_invite_code


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
    _ensure_club_pick_columns(engine)
    _ensure_shelf_note_columns(engine)
    return engine


def _ensure_club_pick_columns(engine: Engine) -> None:
    with engine.begin() as conn:
        rows = conn.execute(text("PRAGMA table_info(club_picks)")).fetchall()
        names = {row[1] for row in rows}
        if rows and "meeting_at" not in names:
            conn.execute(text("ALTER TABLE club_picks ADD COLUMN meeting_at DATETIME"))


def _ensure_shelf_note_columns(engine: Engine) -> None:
    with engine.begin() as conn:
        rows = conn.execute(text("PRAGMA table_info(shelf)")).fetchall()
        names = {row[1] for row in rows}
        if not rows:
            return
        if "rating" not in names:
            conn.execute(text("ALTER TABLE shelf ADD COLUMN rating INTEGER"))
        if "take" not in names:
            conn.execute(text("ALTER TABLE shelf ADD COLUMN take VARCHAR DEFAULT ''"))
        if "dnf_reason" not in names:
            conn.execute(text("ALTER TABLE shelf ADD COLUMN dnf_reason VARCHAR DEFAULT ''"))
        if "progress" not in names:
            conn.execute(text("ALTER TABLE shelf ADD COLUMN progress INTEGER"))


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
