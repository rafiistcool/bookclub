import sqlite3

from sqlalchemy import text

from app.db import COLUMN_MIGRATIONS, init_db, missing_columns

LEGACY_SCHEMA = """
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username VARCHAR(32) NOT NULL UNIQUE,
    password_hash VARCHAR NOT NULL,
    created_at DATETIME NOT NULL
);
CREATE TABLE books (
    id INTEGER PRIMARY KEY,
    ol_work_key VARCHAR(64) NOT NULL UNIQUE,
    title VARCHAR NOT NULL,
    authors VARCHAR NOT NULL,
    cover_id INTEGER,
    year INTEGER
);
CREATE TABLE shelf (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    book_id INTEGER NOT NULL REFERENCES books(id),
    status VARCHAR NOT NULL,
    position INTEGER NOT NULL,
    updated_at DATETIME NOT NULL,
    UNIQUE (user_id, book_id)
);
CREATE TABLE club_picks (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books(id),
    set_by_id INTEGER NOT NULL REFERENCES users(id),
    note VARCHAR(280) NOT NULL,
    started_at DATETIME NOT NULL,
    ended_at DATETIME
);
CREATE TABLE club_pick_posts (
    id INTEGER PRIMARY KEY,
    pick_id INTEGER NOT NULL REFERENCES club_picks(id),
    author_id INTEGER NOT NULL REFERENCES users(id),
    body VARCHAR(1000) NOT NULL,
    created_at DATETIME NOT NULL
);
CREATE TABLE next_up_votes (
    id INTEGER PRIMARY KEY,
    started_at DATETIME NOT NULL,
    ended_at DATETIME,
    winner_nomination_id INTEGER
);
INSERT INTO users (id, username, password_hash, created_at)
    VALUES (1, 'ada', 'x', '2024-01-01 00:00:00');
INSERT INTO books (id, ol_work_key, title, authors, cover_id, year)
    VALUES (1, '/works/OL1W', 'Circe', 'Madeline Miller', 1, 2018);
INSERT INTO shelf (id, user_id, book_id, status, position, updated_at)
    VALUES (1, 1, 1, 'finished', 0, '2024-01-02 00:00:00');
"""


def test_legacy_database_is_upgraded_in_place(tmp_path):
    path = tmp_path / "legacy.db"
    raw = sqlite3.connect(path)
    raw.executescript(LEGACY_SCHEMA)
    raw.close()

    engine = init_db(path)
    assert missing_columns(engine) == {}

    with engine.begin() as conn:
        shelf_cols = {row[1] for row in conn.execute(text("PRAGMA table_info(shelf)"))}
        assert {"rating", "take", "dnf_reason", "progress", "started_at", "finished_at"} <= shelf_cols
        book_cols = {row[1] for row in conn.execute(text("PRAGMA table_info(books)"))}
        assert {
            "description",
            "pages",
            "subjects",
            "ol_rating",
            "details_fetched_at",
            "cover_image_url",
        } <= book_cols
        user_cols = {row[1] for row in conn.execute(text("PRAGMA table_info(users)"))}
        assert {
            "notify_meeting",
            "notify_pick",
            "notify_note",
            "locale",
            "avatar",
            "avatar_mime",
            "avatar_updated_at",
        } <= user_cols
        locale = conn.execute(text("SELECT locale FROM users")).one()
        assert locale[0] == "en"
        post_cols = {row[1] for row in conn.execute(text("PRAGMA table_info(club_pick_posts)"))}
        assert {"spoiler_upto", "milestone_id", "updated_at"} <= post_cols
        vote_cols = {row[1] for row in conn.execute(text("PRAGMA table_info(next_up_votes)"))}
        assert "closes_at" in vote_cols

        # Existing rows survive and pick up sane defaults.
        row = conn.execute(
            text("SELECT status, rating, take, subjects FROM shelf JOIN books ON books.id = shelf.book_id")
        ).one()
        assert row[0] == "finished"
        assert row[1] is None
        assert row[2] == ""
        assert row[3] == "[]"
        notify = conn.execute(text("SELECT notify_meeting, notify_pick, notify_note FROM users")).one()
        assert tuple(notify) == (1, 1, 1)

        # New tables were created alongside.
        tables = {
            row[0]
            for row in conn.execute(text("SELECT name FROM sqlite_master WHERE type='table'"))
        }
        assert {
            "events",
            "post_reactions",
            "push_subscriptions",
            "pick_milestones",
            "quotes",
            "user_favorites",
        } <= tables
        favorite_indexes = {
            row[1]
            for row in conn.execute(text("PRAGMA index_list(user_favorites)"))
            if row[2]
        }
        assert len(favorite_indexes) >= 2
        favorite_fks = conn.execute(text("PRAGMA foreign_key_list(user_favorites)")).fetchall()
        assert {row[6] for row in favorite_fks} == {"CASCADE"}

    # Running again is a no-op.
    init_db(path)
    assert missing_columns(engine) == {}


def test_fresh_database_matches_migration_list(tmp_path):
    engine = init_db(tmp_path / "fresh.db")
    assert missing_columns(engine) == {}
    # Every migrated column must exist in the SQLModel metadata so create_all
    # and ALTER TABLE agree on the schema.
    from sqlmodel import SQLModel

    for table, columns in COLUMN_MIGRATIONS.items():
        model_cols = set(SQLModel.metadata.tables[table].columns.keys())
        for name in columns:
            assert name in model_cols, f"{table}.{name} migrated but not modelled"
