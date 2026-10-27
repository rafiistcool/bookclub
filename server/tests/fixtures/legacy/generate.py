#!/usr/bin/env python3
"""Generate the legacy SQLite fixtures of the importer tests (#38).

Writes, next to this file:

- fresh.sql: a database made by the Python app (`app.seed_demo` with `DEMO_DB` overridden,
  then `init_db`), plus the extra rows docs/phase1/spec/platform.md §5.8 and
  docs/phase1/legacy-mapping.md §0 "Test fixtures" ask for.
- migrated.sql: the hand-written LEGACY_SCHEMA of backend/tests/test_migrations.py, booted once
  through `init_db` (columns added by ALTER TABLE, nullable take/dnf_reason, timestamps without
  fractions), plus a few rows.
- legacy_schema.sql: LEGACY_SCHEMA as it is, never booted: a very old database whose
  COLUMN_MIGRATIONS columns and later tables are missing.

Every value is synthetic. Time is frozen, and the password hashes are one fixed argon2-cffi hash
of KNOWN_PASSWORD, so the output only changes when this script or the Python app does. The dumps
are `sqlite3.Connection.iterdump()` text, which the Rust tests load with `execute_batch`.

Run it with the backend's virtualenv from the repository root:

    backend/.venv/bin/python server/tests/fixtures/legacy/generate.py
"""

from __future__ import annotations

import importlib.util
import io
import json
import sqlite3
import sys
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
sys.path.insert(0, str(REPO / "backend"))

# The time every default timestamp gets. A fraction, so the fresh dump has `.ffffff` stamps.
FIXED_NOW = datetime(2026, 9, 1, 12, 0, 0, 123456, tzinfo=timezone.utc)
# argon2-cffi `PasswordHasher().hash(KNOWN_PASSWORD)`, made once. Every fresh.sql user has it.
KNOWN_PASSWORD = "DemoBuecher2026!"
KNOWN_HASH = (
    "$argon2id$v=19$m=65536,t=3,p=4$eNpQ0hjz4XDBl334aMDIDg$"
    "OWwxoO8q9V8SLqTzhxy4R5WPEPmw3rB+36vq4hPrdeM"
)

# Synthetic ISBNs with valid checksums (no real book).
ISBN_MATCH = "9780000000019"  # FakeCatalog resolves it in the tests (the merge fixture)
ISBN_UNMATCHED = "9783161484100"  # a German-group ISBN nobody resolves
ISBN10_KEY = "316100003X"  # an ISBN-10 key with an X check digit
ISBN_INVALID = "9780000000000"  # wrong checksum


def freeze_time() -> None:
    """Every `utcnow()` in the app returns FIXED_NOW.

    `utcnow` looks `datetime` up in `app.models` at call time, so replacing that name freezes
    the column defaults too (patching `utcnow` itself would not: `default_factory` holds the
    function object).
    """
    import app.models as models

    class FrozenDatetime(datetime):
        @classmethod
        def now(cls, tz=None):  # type: ignore[override]
            return FIXED_NOW if tz is not None else FIXED_NOW.replace(tzinfo=None)

    models.datetime = FrozenDatetime  # type: ignore[attr-defined]


def jpeg(color: tuple[int, int, int], size: int = 256) -> bytes:
    from PIL import Image

    out = io.BytesIO()
    Image.new("RGB", (size, size), color).save(out, format="JPEG", quality=85)
    return out.getvalue()


def png(color: tuple[int, int, int], size: int = 256) -> bytes:
    from PIL import Image

    out = io.BytesIO()
    Image.new("RGB", (size, size), color).save(out, format="PNG")
    return out.getvalue()


def dump(path: Path, target: Path) -> None:
    conn = sqlite3.connect(path)
    try:
        statements = list(conn.iterdump())
    finally:
        conn.close()
    # SQLAlchemy creates a table's indexes in set order, which changes from run to run; sort
    # them so the dump only changes when the data does.
    indexes = sorted(s for s in statements if s.startswith(("CREATE INDEX", "CREATE UNIQUE INDEX")))
    if indexes:
        first = next(i for i, s in enumerate(statements) if s in indexes)
        rest = [s for s in statements if s not in indexes]
        statements = rest[:first] + indexes + rest[first:]
    text = "\n".join(statements) + "\n"
    target.write_text(text, encoding="utf-8")
    print(f"wrote {target.relative_to(REPO)} ({len(text)} bytes)")


def at(days: float = 0, hours: float = 0, seconds: float = 0) -> datetime:
    return FIXED_NOW + timedelta(days=days, hours=hours, seconds=seconds)


# ---------------------------------------------------------------------------------------------
# fresh.sql
# ---------------------------------------------------------------------------------------------


def build_fresh(path: Path) -> None:
    import app.seed_demo as seed_demo

    seed_demo.DEMO_DB = path
    seed_demo.hash_password = lambda _password: KNOWN_HASH  # type: ignore[assignment]
    seed_demo.main()
    add_extras(path)


def add_extras(path: Path) -> None:
    from sqlmodel import Session, select

    from app import activity
    from app.db import init_db
    from app.models import (
        Book,
        BookPost,
        BookPostReaction,
        ClubPick,
        ClubPickPost,
        Event,
        Invite,
        NextUpBallot,
        NextUpNomination,
        NextUpVote,
        PickMilestone,
        PostReaction,
        PushSubscription,
        Quote,
        SearchHistory,
        ShelfEntry,
        ShelfStatus,
        User,
        UserFavorite,
        UserPhysicalShelfLayout,
    )

    engine = init_db(path)
    with Session(engine) as session:
        users = {u.username: u for u in session.exec(select(User)).all()}
        demo, anna, max_ = users["demo"], users["anna"], users["max"]
        books = {b.ol_work_key: b for b in session.exec(select(Book)).all()}
        b1 = books["/works/BC0000000001"]
        b2 = books["/works/BC0000000002"]
        b3 = books["/works/BC0000000003"]
        picks = session.exec(select(ClubPick).order_by(ClubPick.id)).all()
        previous_pick, current_pick = picks[0], picks[1]
        milestones = session.exec(
            select(PickMilestone).order_by(PickMilestone.position)
        ).all()

        # --- users: an English one with every notification off and a JPEG avatar; one whose
        # avatar has neither a MIME type nor a timestamp; max gets a PNG (an unknown type).
        lena = User(
            username="lena",
            password_hash=KNOWN_HASH,
            theme="forest",
            color_mode="dark",
            locale="en",
            notify_meeting=False,
            notify_pick=False,
            notify_note=False,
            avatar=jpeg((200, 120, 80)),
            avatar_mime="image/jpeg",
            avatar_updated_at=at(days=-3),
            created_at=at(days=-30),
        )
        otto = User(
            username="otto",
            password_hash=KNOWN_HASH,
            theme="sepia",  # not a theme any more: imported as paper
            color_mode="system",
            locale="fr",  # not a locale: imported as en
            avatar=jpeg((40, 90, 160)),
            avatar_mime=None,
            avatar_updated_at=None,
            created_at=at(days=-20),
        )
        session.add(lena)
        session.add(otto)
        session.flush()
        max_.avatar = png((10, 200, 10))
        max_.avatar_mime = "image/png"
        max_.avatar_updated_at = at(days=-10)
        session.add(max_)
        for user in (lena, otto):
            ev = activity.record(session, user, "member_joined")
            ev.created_at = user.created_at

        # --- invites: the bootstrap invite (no creator) demo redeemed, one lena redeemed, one
        # otto redeemed without a used_at, and an unused CLI invite (seed: DEMOTEST2026, unused).
        session.add(
            Invite(code="BOOTSTRAPSECRETCODE2024", created_by_id=None, used_by_id=demo.id,
                   created_at=at(days=-46), used_at=at(days=-45))
        )
        session.add(
            Invite(code="LENA7K2M9Q", created_by_id=demo.id, used_by_id=lena.id,
                   created_at=at(days=-31), used_at=at(days=-30))
        )
        session.add(
            Invite(code="OTTO4H8P2X", created_by_id=anna.id, used_by_id=otto.id,
                   created_at=at(days=-21), used_at=None)
        )
        session.add(Invite(code="CLIMINTED01", created_by_id=None, created_at=at(days=-5)))

        # --- books of every key class and the normalisation cases.
        ol = Book(
            ol_work_key="/works/OL45883W", title="Middlemarch", authors="George Eliot",
            cover_id=12345, spine_color="#a1b2c3", year=1871,
            description="A study of provincial life.", pages=880,
            subjects=json.dumps(["Fiction", "England -- Fiction", "Domestic fiction"]),
            ol_rating=4.1, details_fetched_at=at(days=-40),
        )
        isbn_match = Book(
            ol_work_key=f"/works/ISBN{ISBN_MATCH}", title="Middlemarch (Synthetic Edition)",
            authors="George Eliot", cover_id=None,
            cover_image_url="http://books.google.com/books/content?id=synthVol0001&img=1",
            spine_color="#123456", year=2003,
            description="A synthetic Google description.", pages=0,
            subjects=json.dumps(["Fiction / Classics"]), ol_rating=3.9,
        )
        isbn_unmatched = Book(
            ol_work_key=f"/works/ISBN{ISBN_UNMATCHED}", title="Ein erfundenes Buch",
            authors="Erika Mustermann", cover_id=777, spine_color="",
            cover_image_url="http://books.google.com/books/content?id=synthVol0002&img=1",
            year=2019, description="Synthetic Google text.", pages=250,
            subjects=json.dumps(["Fiction / General"]), ol_rating=4.5,
        )
        isbn10 = Book(
            ol_work_key=f"/works/ISBN{ISBN10_KEY}", title="Zehnstellig",
            authors="Max Mustermann", year=1999, pages=120,
        )
        isbn_invalid = Book(
            ol_work_key=f"/works/ISBN{ISBN_INVALID}", title="Kaputte Nummer",
            authors="Niemand", year=2001, pages=99,
        )
        google = Book(
            ol_work_key="/works/GBsynthVol42_x-Y", title="Synthetic Volume",
            authors="A. Nonymous",
            cover_image_url="http://books.google.com/books/content?id=synthVol42_x-Y&img=1",
            spine_color="#654321", year=99999, description="Synthetic Google blurb.",
            pages=150, subjects=json.dumps(["Synthetic / Subjects"]), ol_rating=2.0,
        )
        other = Book(
            ol_work_key="/works/MISC-legacy-1", title="  Loose Key Book  ",
            authors="Unbekannt", year=1950, pages=200_000,
            description="Entered through an old code path.",
        )
        long_title = Book(
            ol_work_key="/works/BCabcdef0123", title="Sehr lang " * 120,
            authors="Vielschreiber", cover_id=-1, spine_color="", year=2020, pages=300,
            description="Ein Buch mit einem sehr langen Titel.",
            subjects=json.dumps([f"Thema {n}" for n in range(60)]),
        )
        untitled = Book(
            ol_work_key="/works/BCabcdef0124", title="   ", authors="", year=None,
            pages=None, subjects="not json",
        )
        extra_books = [ol, isbn_match, isbn_unmatched, isbn10, isbn_invalid, google, other,
                       long_title, untitled]
        for book in extra_books:
            session.add(book)
        session.flush()

        # --- shelf: the merge pair (OL and ISBN rows of one work) for demo, and the
        # normalisation cases.
        def shelf(user, book, status, position, **fields):
            entry = ShelfEntry(user_id=user.id, book_id=book.id, status=ShelfStatus(status),
                               position=position, updated_at=fields.pop("updated_at", at(days=-2)),
                               **fields)
            session.add(entry)
            session.flush()
            return entry

        demo_ol = shelf(demo, ol, "finished", 3, rating=4, take="Klug und lang.",
                        started_at=at(days=-60), finished_at=at(days=-35),
                        updated_at=at(days=-35), shelf_position=2)
        demo_isbn = shelf(demo, isbn_match, "currently_reading", 1, progress=30,
                          started_at=at(days=-70), updated_at=at(days=-1))
        shelf(lena, isbn_unmatched, "want_to_read", 0)
        shelf(lena, google, "finished", 0, rating=5, take="Wunderbar.",
              started_at=at(days=-9), finished_at=at(days=-4))
        shelf(lena, other, "did_not_finish", 0, dnf_reason="Zu lang.", started_at=at(days=-8))
        shelf(lena, long_title, "currently_reading", 5, progress=150, shelf_position=3,
              started_at=at(days=-7))
        shelf(otto, isbn10, "finished", 0, rating=5, take="", started_at=at(days=-12),
              finished_at=at(days=-6))
        shelf(otto, isbn_invalid, "want_to_read", 7, rating=3, take="Nicht gelesen.")
        shelf(otto, untitled, "want_to_read", 9)

        # --- favourites: demo's third is the OL row; anna has both merge rows (the ISBN row
        # first); lena has a gap in her positions.
        session.add(UserFavorite(user_id=demo.id, book_id=ol.id, position=3))
        session.add(UserFavorite(user_id=anna.id, book_id=isbn_match.id, position=1))
        session.add(UserFavorite(user_id=anna.id, book_id=b2.id, position=2))
        session.add(UserFavorite(user_id=anna.id, book_id=ol.id, position=3))
        session.add(UserFavorite(user_id=lena.id, book_id=google.id, position=1))
        session.add(UserFavorite(user_id=lena.id, book_id=other.id, position=3))

        # --- layouts: demo's places both merge rows, a deleted entry id and a decoration;
        # otto's is not JSON.
        demo_entries = session.exec(
            select(ShelfEntry).where(ShelfEntry.user_id == demo.id).order_by(ShelfEntry.id)
        ).all()
        rows = [[e.id for e in demo_entries[:4]] + ["decor:plant1"],
                [demo_ol.id, demo_isbn.id, 99999], []]
        layout = {
            "rows": rows,
            "material": "walnut",
            "spines": {
                str(demo_ol.id): {"color": "#A1B2C3", "width": 30},
                str(demo_isbn.id): {"color": "#00ff00", "display": "cover"},
                "99999": {"color": "#000000"},
            },
            "decorations": {"decor:plant1": {"kind": "plant", "color": "#2E7D32"}},
        }
        session.add(UserPhysicalShelfLayout(user_id=demo.id, layout_json=json.dumps(layout)))
        session.add(UserPhysicalShelfLayout(user_id=otto.id, layout_json="{not json"))

        # --- picks and milestones: a title-only checkpoint on the previous pick.
        session.add(PickMilestone(pick_id=previous_pick.id, title="Erste Hälfte", position=0,
                                  created_by_id=anna.id, created_at=at(days=-38)))

        # --- legacy pick notes, written before the diary existed; init_db mirrors them below.
        note1 = ClubPickPost(pick_id=previous_pick.id, author_id=anna.id,
                             body="Alte Notiz zur letzten Lektüre.", spoiler_upto=10,
                             created_at=at(days=-30), updated_at=at(days=-29))
        note2 = ClubPickPost(pick_id=current_pick.id, author_id=max_.id,
                             body="Notiz zum ersten Abschnitt.", spoiler_upto=30,
                             milestone_id=milestones[0].id, created_at=at(days=-6))
        note3 = ClubPickPost(pick_id=current_pick.id, author_id=demo.id,
                             body="Diese Kopie löscht der Autor im Tagebuch.",
                             created_at=at(days=-5))
        # note3 gets a lower id than note2, so once its diary copy is deleted it lies below a
        # mirrored note's id (legacy-mapping.md §9.2: skipped, not imported as new).
        for note in (note1, note3, note2):
            session.add(note)
        session.flush()
        session.add(PostReaction(post_id=note1.id, user_id=demo.id, emoji="🔥",
                                 created_at=at(days=-29)))
        session.add(PostReaction(post_id=note1.id, user_id=max_.id, emoji="👍",
                                 created_at=at(days=-29)))
        session.add(PostReaction(post_id=note2.id, user_id=anna.id, emoji="❤️",
                                 created_at=at(days=-5)))
        for note, author in ((note1, anna), (note2, max_), (note3, demo)):
            ev = activity.record(session, author, "note_posted",
                                 book=b3 if note is note1 else b1,
                                 pick=previous_pick if note is note1 else current_pick,
                                 post_id=note.id)
            ev.created_at = note.created_at
        ev = activity.record(session, demo, "reaction", book=b3, pick=previous_pick,
                             emoji="🔥", post_id=note1.id, author="anna")
        ev.created_at = at(days=-29)
        session.commit()
        ids = {"note1": note1.id, "note2": note2.id, "note3": note3.id, "lena": lena.id,
               "otto": otto.id, "demo": demo.id, "anna": anna.id, "max": max_.id,
               "b1": b1.id, "b2": b2.id, "b3": b3.id, "ol": ol.id, "isbn_match": isbn_match.id,
               "isbn_unmatched": isbn_unmatched.id, "google": google.id, "other": other.id,
               "current_pick": current_pick.id, "previous_pick": previous_pick.id}

    # Boot again: import_legacy_pick_posts mirrors the three notes into book_posts.
    engine = init_db(path)
    with Session(engine) as session:
        mirrors = {p.legacy_post_id: p for p in session.exec(
            select(BookPost).where(BookPost.legacy_post_id.is_not(None))).all()}
        assert set(mirrors) == {ids["note1"], ids["note2"], ids["note3"]}, mirrors

        # A reaction added to a legacy note after mirroring (only in post_reactions).
        session.add(PostReaction(post_id=ids["note1"], user_id=ids["lena"], emoji="😂",
                                 created_at=at(days=-2)))
        # A reaction removed in the diary after mirroring (still in post_reactions).
        removed = session.exec(select(BookPostReaction).where(
            BookPostReaction.post_id == mirrors[ids["note1"]].id,
            BookPostReaction.user_id == ids["max"])).one()
        session.delete(removed)
        # The author deleted the diary copy of note 3 (a hard delete: it had no replies).
        session.delete(mirrors[ids["note3"]])
        # A reply to the mirror copy of note 2.
        session.add(BookPost(book_id=ids["b1"], author_id=ids["anna"],
                             parent_id=mirrors[ids["note2"]].id, body="Antwort auf die Notiz.",
                             created_at=at(days=-4)))
        # A genuinely new pick note, written through /api/pick/posts after the last boot.
        note4 = ClubPickPost(pick_id=ids["current_pick"], author_id=ids["lena"],
                             body="Neue Notiz nach dem letzten Start.", spoiler_upto=50,
                             milestone_id=None, created_at=at(days=-1))
        session.add(note4)
        session.flush()
        session.add(PostReaction(post_id=note4.id, user_id=ids["demo"], emoji="❤️",
                                 created_at=at(hours=-20)))
        lena = session.get(User, ids["lena"])
        demo = session.get(User, ids["demo"])
        anna = session.get(User, ids["anna"])
        max_ = session.get(User, ids["max"])
        otto = session.get(User, ids["otto"])
        b1 = session.get(Book, ids["b1"])
        b2 = session.get(Book, ids["b2"])
        current_pick = session.get(ClubPick, ids["current_pick"])
        ev = activity.record(session, lena, "note_posted", book=b1, pick=current_pick,
                             post_id=note4.id)
        ev.created_at = note4.created_at
        ev = activity.record(session, demo, "reaction", book=b1, pick=current_pick, emoji="❤️",
                             post_id=note4.id, author="lena")
        ev.created_at = at(hours=-20)
        # A reaction event on the deleted copy's note, by a since-unknown author name.
        ev = activity.record(session, anna, "reaction", book=b1, pick=current_pick, emoji="👍",
                             post_id=ids["note3"], author="ghost")
        ev.created_at = at(days=-4)

        # A diary tombstone with a reply (personal: no pick).
        root = BookPost(book_id=b2.id, author_id=max_.id, body="", status_at=None,
                        deleted_at=at(days=-3), created_at=at(days=-6), updated_at=at(days=-3))
        session.add(root)
        session.flush()
        session.add(BookPost(book_id=b2.id, author_id=anna.id, parent_id=root.id,
                             body="Schade, dass der Eintrag weg ist.", created_at=at(days=-5)))
        # A diary entry on a merge row and one with out-of-range positions.
        session.add(BookPost(book_id=ids["isbn_match"], author_id=demo.id,
                             body="Notiz zur Ausgabe.", progress_at=30,
                             status_at="currently_reading", created_at=at(days=-1)))
        session.add(BookPost(book_id=ids["google"], author_id=lena.id,
                             body="Viel zu sagen.", spoiler_upto=140, progress_at=-5,
                             created_at=at(days=-4)))

        # Quotes: one edited after its event, one deleted (its event stays), the seed's quote
        # gets its event, and a page outside 0..20000.
        seed_quote = session.exec(select(Quote)).one()
        ev = activity.record(session, demo, "quote_added", book=b1, page=seed_quote.page,
                             excerpt=seed_quote.body[:80])
        ev.created_at = seed_quote.created_at + timedelta(seconds=0.4)
        edited = Quote(user_id=lena.id, book_id=ids["google"], body="Ursprünglicher Satz.",
                       page=12, created_at=at(days=-4, seconds=10))
        session.add(edited)
        session.flush()
        ev = activity.record(session, lena, "quote_added",
                             book=session.get(Book, ids["google"]), page=12,
                             excerpt=edited.body[:80])
        ev.created_at = edited.created_at + timedelta(seconds=1)
        edited.body = "Geänderter Satz nach dem Speichern."
        ev = activity.record(session, otto, "quote_added", book=b2, page=3,
                             excerpt="Ein gelöschtes Zitat")
        ev.created_at = at(days=-2)
        session.add(Quote(user_id=otto.id, book_id=b2.id, body="Seite außerhalb.", page=25000,
                          created_at=at(days=-1)))

        # An ended vote that nominated both merge rows; its winner is the ISBN row's
        # nomination.
        vote = NextUpVote(started_at=at(days=-50), ended_at=at(days=-41),
                          closes_at=at(days=-41))
        session.add(vote)
        session.flush()
        nom_ol = NextUpNomination(vote_id=vote.id, book_id=ids["ol"], nominated_by_id=demo.id,
                                  created_at=at(days=-49))
        nom_isbn = NextUpNomination(vote_id=vote.id, book_id=ids["isbn_match"],
                                    nominated_by_id=anna.id, created_at=at(days=-48))
        nom_other = NextUpNomination(vote_id=vote.id, book_id=ids["isbn_unmatched"],
                                     nominated_by_id=lena.id, created_at=at(days=-47))
        for nom in (nom_ol, nom_isbn, nom_other):
            session.add(nom)
        session.flush()
        session.add(NextUpBallot(vote_id=vote.id, nomination_id=nom_isbn.id, user_id=max_.id,
                                 created_at=at(days=-46)))
        session.add(NextUpBallot(vote_id=vote.id, nomination_id=nom_ol.id, user_id=lena.id,
                                 created_at=at(days=-46)))
        session.add(NextUpBallot(vote_id=vote.id, nomination_id=nom_isbn.id, user_id=anna.id,
                                 created_at=at(days=-45)))
        vote.winner_nomination_id = nom_isbn.id

        # Events of every kind the seed does not write.
        events = [
            (anna, "shelf_moved", b2, None, {"status": "currently_reading"}),
            (demo, "shelf_finished", session.get(Book, ids["ol"]), None,
             {"rating": 4, "take": "Klug und lang."}),
            (lena, "shelf_dnf", session.get(Book, ids["other"]), None, {"reason": "Zu lang."}),
            (otto, "shelf_removed", b2, None, {"status": "want_to_read"}),
            (demo, "progress", session.get(Book, ids["isbn_match"]), None, {"progress": 30}),
            (anna, "pick_cleared", session.get(Book, ids["b3"]),
             session.get(ClubPick, ids["previous_pick"]), {}),
            (demo, "pick_set", b1, current_pick,
             {"meeting_at": (FIXED_NOW + timedelta(days=7)).isoformat()}),
            (demo, "nominated", session.get(Book, ids["ol"]), None, {}),
            (max_, "voted", session.get(Book, ids["isbn_match"]), None, {}),
            (anna, "vote_closed", session.get(Book, ids["isbn_match"]), None,
             {"votes": 2, "auto": True}),
            (demo, "milestone_added", b1, current_pick,
             {"title": "Ankommen an der Küste",
              "due_at": (FIXED_NOW + timedelta(days=2)).isoformat()}),
            # A personal kind that carried the current pick (dropped on import).
            (max_, "shelf_added", b1, current_pick, {"status": "want_to_read"}),
        ]
        for offset, (actor, kind, book, pick, payload) in enumerate(events):
            ev = activity.record(session, actor, kind, book=book, pick=pick, **payload)
            ev.created_at = at(days=-1, seconds=offset)
        broken = Event(actor_id=anna.id, kind="progress", book_id=b2.id, payload="[1, 2]",
                       created_at=at(hours=-2))
        session.add(broken)
        session.add(Event(actor_id=anna.id, kind="shelf_moved", book_id=b2.id,
                          payload="not json", created_at=at(hours=-1)))

        # Search history, one query over the 300-character limit.
        for user, query, when in ((demo, "Middlemarch", at(days=-3)),
                                  (demo, "George ELIOT", at(days=-2)),
                                  (lena, "Straße der Bücher", at(days=-1)),
                                  (lena, "x" * 320, at(hours=-3))):
            session.add(SearchHistory(user_id=user.id, query=query,
                                      normalized_query=query.casefold(), last_used_at=when))

        # Push subscriptions (dropped by the importer).
        session.add(PushSubscription(user_id=demo.id,
                                     endpoint="https://push.example.test/sub/1",
                                     p256dh="BOrsynthetic", auth="synthetic",
                                     user_agent="Firefox", created_at=at(days=-10)))
        session.add(PushSubscription(user_id=lena.id,
                                     endpoint="https://push.example.test/sub/2",
                                     p256dh="BOrsynthetic2", auth="synthetic2",
                                     created_at=at(days=-9)))
        session.commit()


# ---------------------------------------------------------------------------------------------
# migrated.sql and legacy_schema.sql
# ---------------------------------------------------------------------------------------------


def legacy_schema() -> str:
    spec = importlib.util.spec_from_file_location(
        "legacy_test_migrations", REPO / "backend" / "tests" / "test_migrations.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.LEGACY_SCHEMA


# Rows for the migrated database, written as old code did: plain SQL, no fractions.
MIGRATED_EXTRAS = """
INSERT INTO users (id, username, password_hash, created_at)
    VALUES (2, 'bea', '{hash}', '2024-02-01 09:30:00');
INSERT INTO books (id, ol_work_key, title, authors, cover_id, year)
    VALUES (2, '/works/OL2W', 'No Cover', 'Anon', -1, 1999);
INSERT INTO club_picks (id, book_id, set_by_id, note, started_at, ended_at)
    VALUES (1, 1, 1, 'First pick', '2024-01-05 18:00:00', NULL);
INSERT INTO club_pick_posts (id, pick_id, author_id, body, created_at)
    VALUES (1, 1, 2, 'Written before the diary existed', '2024-01-06 20:15:00');
INSERT INTO next_up_votes (id, started_at, ended_at, winner_nomination_id)
    VALUES (1, '2024-01-01 12:00:00', '2024-01-04 12:00:00', NULL);
"""

# Written after the boot, through the columns ALTER TABLE added: NULL take and dnf_reason.
MIGRATED_AFTER_BOOT = """
INSERT INTO shelf (id, user_id, book_id, status, position, updated_at, take, dnf_reason)
    VALUES (2, 2, 2, 'did_not_finish', 0, '2024-02-02 10:00:00', NULL, NULL);
INSERT INTO shelf (id, user_id, book_id, status, position, updated_at, take, dnf_reason, rating)
    VALUES (3, 2, 1, 'finished', 0, '2024-02-03 11:00:00', NULL, NULL, 4);
"""


def build_migrated(path: Path) -> None:
    from app.db import init_db

    raw = sqlite3.connect(path)
    raw.executescript(legacy_schema())
    raw.executescript(MIGRATED_EXTRAS.format(hash=KNOWN_HASH))
    raw.close()
    init_db(path).dispose()
    raw = sqlite3.connect(path)
    raw.executescript(MIGRATED_AFTER_BOOT)
    raw.close()


def main() -> None:
    freeze_time()
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        fresh = tmp_path / "fresh.db"
        build_fresh(fresh)
        dump(fresh, HERE / "fresh.sql")
        migrated = tmp_path / "migrated.db"
        build_migrated(migrated)
        dump(migrated, HERE / "migrated.sql")
        (HERE / "legacy_schema.sql").write_text(legacy_schema().lstrip(), encoding="utf-8")
        print(f"wrote {(HERE / 'legacy_schema.sql').relative_to(REPO)}")


if __name__ == "__main__":
    main()
