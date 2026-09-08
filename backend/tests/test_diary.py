from sqlmodel import Session, select

from app.db import import_legacy_pick_posts
from app.models import BookPost, BookPostReaction
from tests.conftest import register
from tests.test_search import _FakeClient
from app.routers import books as books_router


CIRCE = {
    "ol_work_key": "/works/OL1W",
    "title": "Circe",
    "authors": "Madeline Miller",
    "cover_id": 123,
    "year": 2018,
}
CIRCE_BOOK = {k: v for k, v in CIRCE.items() if k != "ol_work_key"}

ACHILLES = {
    "ol_work_key": "/works/OL2W",
    "title": "The Song of Achilles",
    "authors": "Madeline Miller",
    "cover_id": 456,
    "year": 2011,
}

DIARY = "/api/books/works/OL1W/posts"


def _shelve(client, book=CIRCE, status="currently_reading", **extra):
    return client.post("/api/shelf", json={**book, "status": status, **extra})


def _second_member(client, name="grace"):
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, name, invite=invite)


def _login(client, name):
    client.post("/api/auth/logout")
    client.post("/api/auth/login", json={"username": name, "password": "password1"})


def test_requires_auth_and_valid_work_id(client):
    assert client.get(DIARY).status_code == 401
    register(client, "ada")
    assert client.get("/api/books/works/nope/posts").status_code == 404
    assert client.post("/api/books/works/nope/posts", json={"body": "x"}).status_code == 404


def test_diary_on_an_unknown_book_is_empty_not_missing(client):
    register(client, "ada")
    body = client.get(DIARY).json()
    assert body["ol_work_key"] == "/works/OL1W"
    assert body["book_id"] is None
    assert body["items"] == []
    assert body["my_progress"] is None
    assert body["my_status"] is None


def test_first_entry_creates_the_book_when_given_its_details(client):
    register(client, "ada")
    missing = client.post(DIARY, json={"body": "Loved the opening"})
    assert missing.status_code == 400

    created = client.post(DIARY, json={"body": "  Loved the opening  ", "book": CIRCE_BOOK})
    assert created.status_code == 201
    entry = created.json()
    assert entry["author"] == "ada"
    assert entry["mine"] is True
    assert entry["body"] == "Loved the opening"
    assert entry["progress_at"] is None
    assert entry["status_at"] is None
    assert entry["created_label"]

    diary = client.get(DIARY).json()
    assert diary["book_id"]
    assert [row["body"] for row in diary["items"]] == ["Loved the opening"]
    # The book now exists for everyone else too.
    assert client.get("/api/shelf").json()["items"] == []


def test_entry_snapshots_where_the_author_was(client):
    register(client, "ada")
    _shelve(client)
    client.patch("/api/shelf/1", json={"progress": 40})
    reading = client.post(DIARY, json={"body": "Halfway-ish", "spoiler_upto": 40}).json()
    assert reading["progress_at"] == 40
    assert reading["status_at"] == "currently_reading"
    assert reading["spoiler_upto"] == 40
    assert reading["author_rating"] is None

    client.patch("/api/shelf/1", json={"status": "finished", "rating": 4})
    done = client.post(DIARY, json={"body": "That ending."}).json()
    assert done["progress_at"] == 100
    assert done["status_at"] == "finished"
    assert done["author_rating"] == 4

    diary = client.get(DIARY).json()
    # The old entry keeps its position, but the author's stars are live.
    first, second = diary["items"]
    assert first["progress_at"] == 40
    assert first["author_rating"] == 4
    assert second["progress_at"] == 100
    assert diary["my_progress"] == 100
    assert diary["my_status"] == "finished"


def test_viewer_position_drives_the_spoiler_shield_fields(client):
    register(client, "ada")
    _shelve(client, status="finished", rating=5)
    client.post(DIARY, json={"body": "Ending spoiler", "spoiler_upto": 100})
    _second_member(client)
    _shelve(client)
    client.patch("/api/shelf/2", json={"progress": 25})
    diary = client.get(DIARY).json()
    assert diary["my_progress"] == 25
    assert diary["my_status"] == "currently_reading"
    assert diary["items"][0]["spoiler_upto"] == 100
    assert diary["items"][0]["mine"] is False


def test_replies_are_one_level_and_stay_on_the_book(client):
    register(client, "ada")
    root = client.post(DIARY, json={"body": "Thoughts?", "book": CIRCE_BOOK}).json()
    _second_member(client)
    reply = client.post(DIARY, json={"body": "Yes!", "parent_id": root["id"]})
    assert reply.status_code == 201
    assert reply.json()["parent_id"] == root["id"]

    nested = client.post(DIARY, json={"body": "Deeper", "parent_id": reply.json()["id"]})
    assert nested.status_code == 400

    other = client.post(
        "/api/books/works/OL2W/posts",
        json={"body": "Wrong book", "parent_id": root["id"], "book": {"title": "Achilles"}},
    )
    assert other.status_code == 400

    diary = client.get(DIARY).json()
    assert len(diary["items"]) == 1
    assert [row["body"] for row in diary["items"][0]["replies"]] == ["Yes!"]
    assert diary["items"][0]["replies"][0]["author"] == "grace"


def test_only_the_author_edits_or_deletes(client):
    register(client, "ada")
    entry = client.post(DIARY, json={"body": "Draft", "book": CIRCE_BOOK}).json()
    _second_member(client)
    assert client.patch(f"/api/posts/{entry['id']}", json={"body": "Hijack"}).status_code == 403
    assert client.delete(f"/api/posts/{entry['id']}").status_code == 403

    _login(client, "ada")
    assert client.patch(f"/api/posts/{entry['id']}", json={}).status_code == 400
    edited = client.patch(
        f"/api/posts/{entry['id']}", json={"body": "Final", "spoiler_upto": 60}
    ).json()
    assert edited["body"] == "Final"
    assert edited["spoiler_upto"] == 60
    assert edited["edited"] is True

    cleared = client.patch(f"/api/posts/{entry['id']}", json={"spoiler_upto": None}).json()
    assert cleared["spoiler_upto"] is None

    assert client.delete(f"/api/posts/{entry['id']}").status_code == 204
    assert client.get(DIARY).json()["items"] == []
    assert client.delete(f"/api/posts/{entry['id']}").status_code == 404


def test_deleting_an_entry_with_replies_leaves_a_tombstone(client):
    register(client, "ada")
    root = client.post(DIARY, json={"body": "Root", "spoiler_upto": 50, "book": CIRCE_BOOK}).json()
    client.post(f"/api/posts/{root['id']}/reactions", json={"emoji": "❤️"})
    _second_member(client)
    client.post(DIARY, json={"body": "Reply", "parent_id": root["id"]})

    _login(client, "ada")
    assert client.delete(f"/api/posts/{root['id']}").status_code == 204
    diary = client.get(DIARY).json()
    ghost = diary["items"][0]
    assert ghost["deleted"] is True
    assert ghost["body"] == ""
    assert ghost["spoiler_upto"] is None
    assert ghost["reactions"] == []
    assert [row["body"] for row in ghost["replies"]] == ["Reply"]

    # Nobody can bring it back or hang more on it.
    assert client.patch(f"/api/posts/{root['id']}", json={"body": "x"}).status_code == 404
    assert client.post(f"/api/posts/{root['id']}/reactions", json={"emoji": "👍"}).status_code == 404
    assert client.post(DIARY, json={"body": "Late", "parent_id": root["id"]}).status_code == 400
    # Tombstones stay out of the club feed.
    assert [row["entry"]["body"] for row in client.get("/api/diary").json()["items"]] == ["Reply"]


def test_reactions_toggle_and_aggregate(client):
    register(client, "ada")
    entry = client.post(DIARY, json={"body": "Root", "book": CIRCE_BOOK}).json()
    assert client.post(f"/api/posts/{entry['id']}/reactions", json={"emoji": "🎉"}).status_code == 400

    first = client.post(f"/api/posts/{entry['id']}/reactions", json={"emoji": "❤️"}).json()
    assert first["reactions"] == [{"emoji": "❤️", "count": 1, "mine": True, "users": ["ada"]}]

    _second_member(client)
    second = client.post(f"/api/posts/{entry['id']}/reactions", json={"emoji": "❤️"}).json()
    assert second["reactions"][0]["count"] == 2
    assert second["reactions"][0]["mine"] is True
    assert second["reactions"][0]["users"] == ["ada", "grace"]

    untoggled = client.post(f"/api/posts/{entry['id']}/reactions", json={"emoji": "❤️"}).json()
    assert untoggled["reactions"] == [{"emoji": "❤️", "count": 1, "mine": False, "users": ["ada"]}]


def test_entry_on_the_current_pick_records_the_pick(client):
    register(client, "ada")
    pick_id = client.put("/api/pick", json=CIRCE).json()["id"]
    entry = client.post(DIARY, json={"body": "Pick note"}).json()
    with Session(client.app.state.engine) as session:
        row = session.get(BookPost, entry["id"])
        assert row is not None and row.pick_id == pick_id
    other = client.post(
        "/api/books/works/OL2W/posts", json={"body": "Side read", "book": {"title": "Achilles"}}
    ).json()
    with Session(client.app.state.engine) as session:
        row = session.get(BookPost, other["id"])
        assert row is not None and row.pick_id is None


def test_feed_exposes_viewer_position_for_spoiler_shield(client):
    register(client, "ada")
    _shelve(client)
    client.patch("/api/shelf/1", json={"progress": 80})
    spoiler = "The coconut pillow on p. 237 is a tell."
    client.post(DIARY, json={"body": spoiler, "spoiler_upto": 70})
    client.post(
        "/api/books/works/OL2W/posts",
        json={"body": "Plain note", "book": {"title": "The Song of Achilles"}},
    )

    own = {row["book"]["title"]: row for row in client.get("/api/diary").json()["items"]}
    assert own["Circe"]["my_progress"] == 80
    assert own["Circe"]["my_status"] == "currently_reading"
    assert own["Circe"]["entry"]["mine"] is True
    assert own["Circe"]["entry"]["body"] == spoiler
    assert own["The Song of Achilles"]["my_progress"] is None
    assert own["The Song of Achilles"]["my_status"] is None

    _second_member(client)
    unshelved = {row["book"]["title"]: row for row in client.get("/api/diary").json()["items"]}
    assert unshelved["Circe"]["my_progress"] is None
    assert unshelved["Circe"]["my_status"] is None
    assert unshelved["Circe"]["entry"]["mine"] is False
    assert unshelved["Circe"]["entry"]["spoiler_upto"] == 70

    _shelve(client)
    client.patch("/api/shelf/2", json={"progress": 25})
    behind = {row["book"]["title"]: row for row in client.get("/api/diary").json()["items"]}
    assert behind["Circe"]["my_progress"] == 25
    assert behind["Circe"]["my_status"] == "currently_reading"
    assert behind["The Song of Achilles"]["my_progress"] is None

    client.patch("/api/shelf/2", json={"status": "finished", "rating": 4})
    done = {row["book"]["title"]: row for row in client.get("/api/diary").json()["items"]}
    assert done["Circe"]["my_progress"] == 100
    assert done["Circe"]["my_status"] == "finished"

    client.patch("/api/shelf/2", json={"status": "did_not_finish"})
    dnf = {row["book"]["title"]: row for row in client.get("/api/diary").json()["items"]}
    assert dnf["Circe"]["my_progress"] is None
    assert dnf["Circe"]["my_status"] == "did_not_finish"


def test_feed_is_newest_first_across_books_and_pages(client):
    register(client, "ada")
    root = client.post(DIARY, json={"body": "Circe 1", "book": CIRCE_BOOK}).json()
    client.post(
        "/api/books/works/OL2W/posts",
        json={"body": "Achilles 1", "book": {"title": "The Song of Achilles"}},
    )
    _second_member(client)
    client.post(DIARY, json={"body": "Circe reply", "parent_id": root["id"]})

    page = client.get("/api/diary", params={"limit": 2}).json()
    assert [row["entry"]["body"] for row in page["items"]] == ["Circe reply", "Achilles 1"]
    assert [row["book"]["title"] for row in page["items"]] == ["Circe", "The Song of Achilles"]
    assert page["items"][0]["parent_author"] == "ada"
    assert page["items"][1]["parent_author"] is None
    assert page["has_more"] is True

    rest = client.get(
        "/api/diary", params={"limit": 2, "before": page["items"][-1]["entry"]["id"]}
    ).json()
    assert [row["entry"]["body"] for row in rest["items"]] == ["Circe 1"]
    assert rest["has_more"] is False


def test_legacy_pick_notes_are_imported_once_with_reactions(client):
    register(client, "ada")
    pick_id = client.put("/api/pick", json=CIRCE).json()["id"]
    legacy = client.post("/api/pick/posts", json={"body": "Old-style note", "spoiler_upto": 30}).json()
    client.post(f"/api/pick/posts/{legacy['id']}/reactions", json={"emoji": "🔥"})

    engine = client.app.state.engine
    assert import_legacy_pick_posts(engine) == 1
    assert import_legacy_pick_posts(engine) == 0

    diary = client.get(DIARY).json()
    assert [row["body"] for row in diary["items"]] == ["Old-style note"]
    assert diary["items"][0]["spoiler_upto"] == 30
    assert diary["items"][0]["author"] == "ada"
    assert diary["items"][0]["reactions"] == [
        {"emoji": "🔥", "count": 1, "mine": True, "users": ["ada"]}
    ]
    with Session(engine) as session:
        rows = session.exec(select(BookPost)).all()
        assert len(rows) == 1
        assert rows[0].pick_id == pick_id
        assert rows[0].legacy_post_id == legacy["id"]
        assert len(session.exec(select(BookPostReaction)).all()) == 1


def test_club_rating_averages_finished_readers(client, monkeypatch):
    register(client, "ada")
    _FakeClient.reset()
    monkeypatch.setattr(books_router.httpx, "AsyncClient", _FakeClient)
    _shelve(client, status="finished", rating=5)
    _second_member(client)
    _shelve(client, status="finished", rating=4)
    _second_member(client, name="tom")
    # Reading with no rating does not count.
    _shelve(client)

    body = client.get("/api/books/works/OL1W").json()
    assert body["club_rating"] == 4.5
    assert body["rating_count"] == 2

    _second_member(client, name="ivy")
    fresh = client.get("/api/books/works/OL2W").json()
    assert fresh["club_rating"] is None
    assert fresh["rating_count"] == 0
