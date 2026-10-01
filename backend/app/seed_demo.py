"""Create an isolated local demo: python -m app.seed_demo.

Only data/demo.db is populated. Running this again preserves demo edits.
"""

import json
from datetime import timedelta
from pathlib import Path

from sqlmodel import Session, select

from app import activity
from app.db import init_db
from app.models import (
    Book, BookPost, BookPostReaction, ClubPick, Invite, NextUpBallot,
    NextUpNomination, NextUpVote, PickMilestone, Quote, ShelfEntry,
    ShelfStatus, User, UserFavorite, utcnow,
)
from app.security import hash_password

DEMO_DB = Path(__file__).resolve().parents[2] / "data" / "demo.db"
DEMO_PASSWORD = "DemoBuecher2026!"


def main() -> None:
    engine = init_db(DEMO_DB)
    now = utcnow()
    with Session(engine) as session:
        if session.exec(select(User).limit(1)).first() is not None:
            print(f"Demo already exists; preserving edits: {DEMO_DB}")
            return

        users = []
        for name in ("demo", "anna", "max"):
            user = User(username=name, password_hash=hash_password(DEMO_PASSWORD),
                        locale="de", created_at=now - timedelta(days=45))
            session.add(user)
            session.flush()
            users.append(user)
            activity.record(session, user, "member_joined")

        # Fictional books and original descriptions keep this demo self-contained.
        titles = [
            ("Die Bibliothek am Meer", "Mara Winter", 2025, 320, "Roman",
             "Als Nora eine kleine Bibliothek an der Küste übernimmt, findet sie Briefe zwischen den Seiten. Eine Geschichte über Freundschaft, Neuanfänge und die Bücher, die uns begleiten."),
            ("Zwischen den Sternen", "Jonas Berg", 2024, 384, "Science Fiction",
             "Eine Forscherin empfängt ein Signal aus einer verlassenen Raumstation. Mit einer ungewöhnlichen Crew macht sie sich auf die Suche nach seinem Ursprung."),
            ("Ein Sommer in Lissabon", "Clara Sommer", 2023, 272, "Roman",
             "Drei Freunde treffen sich nach zehn Jahren wieder. In den Gassen von Lissabon entdecken sie, was sich verändert hat und was geblieben ist."),
            ("Der Garten der Zeit", "Emil Falk", 2022, 296, "Fantasy",
             "Hinter einer unscheinbaren Tür liegt ein Garten, in dem jede Pflanze eine Erinnerung bewahrt. Doch einige Erinnerungen beginnen zu verschwinden."),
            ("Kleine Wege, große Welten", "Lea Neumann", 2025, 224, "Sachbuch",
             "Geschichten und Anregungen für Entdeckungen vor der eigenen Haustür: mit offenen Augen durch den Alltag und zu Fuß ins nächste Abenteuer."),
            ("Nachtzug nach Norden", "Finn Hansen", 2024, 352, "Krimi",
             "Ein Nachtzug bleibt im Schnee stehen. Als ein Passagier verschwindet, muss eine pensionierte Ermittlerin herausfinden, wem sie vertrauen kann."),
            ("Das Haus der leisen Stimmen", "Sophie Adler", 2021, 304, "Roman",
             "Nach dem Tod ihrer Großmutter kehrt eine junge Musikerin in ihr Heimatdorf zurück und beginnt, die Geschichte ihrer Familie neu zu hören."),
            ("Kaffee mit dem Morgen", "Paul Richter", 2025, 192, "Erzählungen",
             "Zwölf kurze Geschichten über zufällige Begegnungen in einem Café. Warmherzig, humorvoll und voller kleiner Überraschungen."),
        ]
        books = []
        for index, (title, author, year, pages, subject, description) in enumerate(titles):
            book = Book(ol_work_key=f"/works/BC{index + 1:010x}", title=title,
                        authors=author, year=year, pages=pages, description=description,
                        subjects=json.dumps([subject], ensure_ascii=False),
                        details_fetched_at=now)
            session.add(book)
            session.flush()
            books.append(book)

        statuses = [
            [(0, "currently_reading", 45, None), (1, "want_to_read", None, None),
             (2, "finished", 100, 5), (3, "want_to_read", None, None),
             (4, "finished", 100, 4), (5, "want_to_read", None, None),
             (6, "did_not_finish", None, None), (7, "finished", 100, 4)],
            [(0, "currently_reading", 70, None), (1, "want_to_read", None, None),
             (2, "finished", 100, 4), (3, "finished", 100, 5),
             (5, "want_to_read", None, None)],
            [(0, "currently_reading", 25, None), (1, "want_to_read", None, None),
             (3, "want_to_read", None, None), (4, "finished", 100, 5),
             (5, "finished", 100, 4)],
        ]
        for user, rows in zip(users, statuses):
            for position, (index, status, progress, rating) in enumerate(rows):
                entry = ShelfEntry(user_id=user.id, book_id=books[index].id,
                                   status=ShelfStatus(status), position=position,
                                   progress=progress, rating=rating,
                                   updated_at=now - timedelta(hours=position),
                                   started_at=now - timedelta(days=12 + index)
                                   if status != "want_to_read" else None,
                                   finished_at=now - timedelta(days=index + 2)
                                   if status == "finished" else None,
                                   take="Eine Geschichte, die noch lange nachklingt."
                                   if rating else "",
                                   dnf_reason="Diesmal war es nicht das richtige Buch für mich."
                                   if status == "did_not_finish" else "")
                session.add(entry)
                activity.record(session, user, "shelf_added", book=books[index],
                                status=status, rating=rating)
        session.add(UserFavorite(user_id=users[0].id, book_id=books[2].id, position=1))
        session.add(UserFavorite(user_id=users[0].id, book_id=books[4].id, position=2))

        previous = ClubPick(book_id=books[2].id, set_by_id=users[1].id,
                            note="Unsere letzte gemeinsame Lektüre.",
                            started_at=now - timedelta(days=40), ended_at=now - timedelta(days=15))
        pick = ClubPick(book_id=books[0].id, set_by_id=users[0].id,
                        note="Willkommen im Demo-Buchclub! Beim nächsten Treffen sprechen wir über die Briefe und Noras Neuanfang.",
                        started_at=now - timedelta(days=8), meeting_at=now + timedelta(days=7))
        session.add(previous)
        session.add(pick)
        session.flush()
        activity.record(session, users[0], "pick_set", book=books[0], pick=pick)
        for pos, (title, chapter_to, days) in enumerate([
            ("Ankommen an der Küste", 8, 2), ("Die Briefe in den Büchern", 16, 5),
            ("Das Ende und unser Club-Treffen", 24, 7),
        ]):
            session.add(PickMilestone(pick_id=pick.id, title=title,
                                     chapter_from=pos * 8 + 1, chapter_to=chapter_to,
                                     due_at=now + timedelta(days=days), position=pos,
                                     created_by_id=users[0].id))

        entries = [
            (1, "Die Atmosphäre am Meer gefällt mir sehr. Ich würde sofort in dieser Bibliothek stöbern!", None, 20),
            (0, "Die Briefe sind mein Lieblingsdetail bisher. Bin gespannt, wer sie hinterlassen hat.", 40, 45),
            (2, "Ich bin gerade bei Kapitel 6. Die Figuren wachsen mir langsam ans Herz.", 25, 25),
            (1, "Ab der Mitte bekommen die Briefe eine neue Bedeutung. Darüber müssen wir beim Treffen sprechen!", 65, 70),
        ]
        posts = []
        for index, (user_index, body, spoiler, progress) in enumerate(entries):
            post = BookPost(book_id=books[0].id, author_id=users[user_index].id,
                            body=body, spoiler_upto=spoiler, progress_at=progress,
                            status_at=ShelfStatus.currently_reading, pick_id=pick.id,
                            created_at=now - timedelta(hours=18 - index * 3))
            session.add(post)
            session.flush()
            posts.append(post)
        session.add(BookPost(book_id=books[0].id, author_id=users[0].id,
                             parent_id=posts[0].id, body="Ja! Ein perfekter Ort für unser nächstes Treffen.",
                             progress_at=45, status_at=ShelfStatus.currently_reading,
                             pick_id=pick.id, created_at=now - timedelta(hours=2)))
        session.add(BookPostReaction(post_id=posts[0].id, user_id=users[0].id, emoji="❤️"))
        session.add(BookPostReaction(post_id=posts[1].id, user_id=users[1].id, emoji="👍"))
        session.add(Quote(user_id=users[0].id, book_id=books[0].id,
                          body="Manche Geschichten finden uns genau dann, wenn wir sie brauchen.", page=84))

        vote = NextUpVote(closes_at=now + timedelta(days=4))
        session.add(vote)
        session.flush()
        for index, book_index in enumerate((1, 3, 5)):
            nomination = NextUpNomination(vote_id=vote.id, book_id=books[book_index].id,
                                         nominated_by_id=users[index].id)
            session.add(nomination)
            session.flush()
            if index < 2:
                session.add(NextUpBallot(vote_id=vote.id, nomination_id=nomination.id,
                                        user_id=users[index + 1].id))
        session.add(Invite(code="DEMOTEST2026", created_by_id=users[0].id))
        session.commit()
    print(f"Demo created: {DEMO_DB}")
    print(f"Login: demo / {DEMO_PASSWORD}")


if __name__ == "__main__":
    main()
