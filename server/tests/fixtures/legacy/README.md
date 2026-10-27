# Legacy importer fixtures (#38)

SQLite databases of the Python app, committed as SQL text (never as a binary DB), for the
`tests/importer_*.rs` tests. Every value in them is synthetic; the production database is never
committed (docs/phase1/legacy-mapping.md §0 "Test fixtures").

| File | What it is |
|---|---|
| `fresh.sql` | `app.seed_demo` (with `DEMO_DB` overridden) plus the extra rows of docs/phase1/spec/platform.md §5.8: every key class (OL, BC, ISBN matched/unmatched, an ISBN-10 key, an invalid ISBN, GB, OTHER), the normalisation cases (cover id −1, pages 0 and 200000, year 99999, a 1200-character title, a blank title, 60 subjects, subjects that are not JSON), avatars (JPEG with and without MIME type and timestamp, a PNG), used, unused and bootstrap invites, mirrored and unmirrored legacy pick notes, a reaction added and one removed after mirroring, a deleted mirror copy, a tombstone with a reply, layouts (one not JSON), favourites with a gap, an ended vote whose winner is a merged nomination, events of every kind, an edited and a deleted quote, search history and push subscriptions. |
| `migrated.sql` | The hand-written `LEGACY_SCHEMA` of `backend/tests/test_migrations.py`, booted once through `init_db`: the ALTER TABLE shape (nullable `take`/`dnf_reason`, migrated columns appended) and timestamps without fractions, plus a few rows. |
| `legacy_schema.sql` | `LEGACY_SCHEMA` as written, never booted: a very old database whose `COLUMN_MIGRATIONS` columns and later tables are missing. |
| `generate.py` | Writes the three files above. |
| `harness.rs` | The Rust helpers the importer tests share (`#[path]`-included): load a fixture into a temporary SQLite file, the importer flags, a run against a `TestApp`. |

The fixtures are deterministic: `generate.py` freezes the app's clock (2026-09-01 12:00:00.123456
UTC) and gives every `fresh.sql` user one known argon2-cffi hash of `DemoBuecher2026!`, so a legacy
user can sign in with it after the import. Indexes are sorted in the dump, because SQLAlchemy
creates them in set order.

## Regenerating

Run the generator with the backend's virtualenv (Python 3.12+, `pip install -e "backend[dev]"`;
it needs the app's own dependencies and Pillow) from the repository root:

```bash
backend/.venv/bin/python server/tests/fixtures/legacy/generate.py
```

On the `agents` VM, the integrator checkout's virtualenv works too:
`/home/rafi/Projects/bookclub/backend/.venv/bin/python server/tests/fixtures/legacy/generate.py`.

Run it whenever the fixture needs new rows or the Python models change, check that two runs give
the same bytes, then run the importer tests:

```bash
cd server && cargo test --test importer_run --test importer_books
```

Tests that need an extra edge case (an x/X key pair, a bad username or timestamp) load a fixture
and change the SQLite copy with `Legacy::exec` instead of growing the fixture. CI needs no Python:
the tests load the SQL text with `rusqlite`.
