from tests.conftest import login, register

CIRCE = {
    "ol_work_key": "/works/OL1W",
    "title": "Circe",
    "authors": "Madeline Miller",
    "cover_id": 123,
    "year": 2018,
}


def _second_member(client, name="grace"):
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, name, invite=invite)


# --- spoiler flag -----------------------------------------------------------


def test_post_carries_spoiler_flag_and_viewer_progress(client):
    register(client, "ada")
    client.put("/api/pick", json=CIRCE)
    post = client.post(
        "/api/pick/posts", json={"body": "The pig scene!", "spoiler_upto": 40}
    ).json()
    assert post["spoiler_upto"] == 40
    assert post["mine"] is True
    assert post["edited"] is False
    assert post["reactions"] == []

    thread = client.get("/api/pick/posts").json()
    assert thread["my_progress"] is None
    assert thread["my_status"] is None

    client.post("/api/shelf", json={**CIRCE, "status": "currently_reading", "progress": 25})
    thread = client.get("/api/pick/posts").json()
    assert thread["my_progress"] == 25
    assert thread["my_status"] == "currently_reading"

    shelf_id = client.get("/api/shelf").json()["items"][0]["id"]
    client.patch(f"/api/shelf/{shelf_id}", json={"status": "finished", "position": 0})
    thread = client.get("/api/pick/posts").json()
    assert thread["my_progress"] == 100
    assert thread["my_status"] == "finished"


def test_spoiler_flag_must_be_a_percentage(client):
    register(client, "ada")
    client.put("/api/pick", json=CIRCE)
    assert client.post("/api/pick/posts", json={"body": "x", "spoiler_upto": 101}).status_code == 400
    assert client.post("/api/pick/posts", json={"body": "x", "spoiler_upto": -1}).status_code == 400


# --- edit / delete ----------------------------------------------------------


def test_author_can_edit_and_delete_own_note(client):
    register(client, "ada")
    client.put("/api/pick", json=CIRCE)
    post = client.post("/api/pick/posts", json={"body": "Frist"}).json()

    edited = client.patch(f"/api/pick/posts/{post['id']}", json={"body": "First"})
    assert edited.status_code == 200
    assert edited.json()["body"] == "First"
    assert edited.json()["edited"] is True

    flagged = client.patch(f"/api/pick/posts/{post['id']}", json={"spoiler_upto": 60})
    assert flagged.json()["spoiler_upto"] == 60
    cleared = client.patch(f"/api/pick/posts/{post['id']}", json={"spoiler_upto": None})
    assert cleared.json()["spoiler_upto"] is None

    assert client.patch(f"/api/pick/posts/{post['id']}", json={}).status_code == 400

    gone = client.delete(f"/api/pick/posts/{post['id']}")
    assert gone.status_code == 204
    assert client.get("/api/pick/posts").json()["items"] == []
    assert client.delete(f"/api/pick/posts/{post['id']}").status_code == 404


def test_other_members_cannot_edit_or_delete(client):
    register(client, "ada")
    client.put("/api/pick", json=CIRCE)
    post = client.post("/api/pick/posts", json={"body": "Ada's note"}).json()
    _second_member(client)
    assert client.patch(f"/api/pick/posts/{post['id']}", json={"body": "hijack"}).status_code == 403
    assert client.delete(f"/api/pick/posts/{post['id']}").status_code == 403
    assert client.get("/api/pick/posts").json()["items"][0]["mine"] is False


# --- reactions ---------------------------------------------------------------


def test_reactions_toggle_and_aggregate(client):
    register(client, "ada")
    client.put("/api/pick", json=CIRCE)
    post = client.post("/api/pick/posts", json={"body": "Loved the ending"}).json()
    url = f"/api/pick/posts/{post['id']}/reactions"

    first = client.post(url, json={"emoji": "❤️"})
    assert first.status_code == 200
    assert first.json()["reactions"] == [
        {"emoji": "❤️", "count": 1, "mine": True, "users": ["ada"]}
    ]

    _second_member(client)
    second = client.post(url, json={"emoji": "❤️"}).json()
    assert second["reactions"][0]["count"] == 2
    assert second["reactions"][0]["mine"] is True
    assert second["reactions"][0]["users"] == ["ada", "grace"]

    fire = client.post(url, json={"emoji": "🔥"}).json()
    assert [row["emoji"] for row in fire["reactions"]] == ["❤️", "🔥"]

    untoggled = client.post(url, json={"emoji": "❤️"}).json()
    assert untoggled["reactions"][0] == {
        "emoji": "❤️",
        "count": 1,
        "mine": False,
        "users": ["ada"],
    }

    assert client.post(url, json={"emoji": "🍕"}).status_code == 400
    assert client.post("/api/pick/posts/9999/reactions", json={"emoji": "❤️"}).status_code == 404


def test_deleting_a_note_removes_its_reactions(client):
    register(client, "ada")
    client.put("/api/pick", json=CIRCE)
    post = client.post("/api/pick/posts", json={"body": "bye"}).json()
    client.post(f"/api/pick/posts/{post['id']}/reactions", json={"emoji": "👍"})
    assert client.delete(f"/api/pick/posts/{post['id']}").status_code == 204


# --- password ----------------------------------------------------------------


def test_change_password(client):
    register(client, "ada", password="oldpassword1")
    wrong = client.patch(
        "/api/auth/password",
        json={"current_password": "nope-nope", "new_password": "newpassword1"},
    )
    assert wrong.status_code == 400
    short = client.patch(
        "/api/auth/password",
        json={"current_password": "oldpassword1", "new_password": "short"},
    )
    assert short.status_code == 400
    same = client.patch(
        "/api/auth/password",
        json={"current_password": "oldpassword1", "new_password": "oldpassword1"},
    )
    assert same.status_code == 400

    ok = client.patch(
        "/api/auth/password",
        json={"current_password": "oldpassword1", "new_password": "newpassword1"},
    )
    assert ok.status_code == 204

    client.post("/api/auth/logout")
    assert login(client, "ada", "oldpassword1").status_code == 401
    assert login(client, "ada", "newpassword1").status_code == 204


def test_change_password_requires_auth(client):
    assert client.patch(
        "/api/auth/password",
        json={"current_password": "a", "new_password": "bbbbbbbbb"},
    ).status_code == 401


# --- notification prefs --------------------------------------------------------


def test_notification_prefs_default_on_and_patch(client):
    register(client, "ada")
    prefs = client.get("/api/auth/notifications").json()
    assert prefs == {"notify_meeting": True, "notify_pick": True, "notify_note": True}
    updated = client.patch("/api/auth/notifications", json={"notify_note": False}).json()
    assert updated == {"notify_meeting": True, "notify_pick": True, "notify_note": False}
    assert client.get("/api/auth/notifications").json()["notify_note"] is False


# --- meeting .ics ------------------------------------------------------------


def test_meeting_ics_download(client):
    register(client, "ada")
    assert client.get("/api/pick/meeting.ics").status_code == 404
    client.put("/api/pick", json=CIRCE)
    assert client.get("/api/pick/meeting.ics").status_code == 404

    client.put("/api/pick", json={**CIRCE, "meeting_at": "2026-09-10T19:30", "note": "Bring snacks; wine, too"})
    response = client.get("/api/pick/meeting.ics")
    assert response.status_code == 200
    assert response.headers["content-type"].startswith("text/calendar")
    assert 'filename="bookclub-' in response.headers["content-disposition"]
    body = response.text
    assert body.startswith("BEGIN:VCALENDAR\r\n")
    assert body.endswith("END:VCALENDAR\r\n")
    for line in body.split("\r\n"):
        assert len(line.encode("utf-8")) <= 75
    unfolded = body.replace("\r\n ", "")
    assert "BEGIN:VEVENT" in unfolded
    assert "DTSTART:20260910T193000Z" in unfolded
    assert "DTEND:20260910T210000Z" in unfolded
    assert "SUMMARY:Bookclub: Circe" in unfolded
    assert "Bring snacks\\; wine\\, too" in unfolded
    assert "https://openlibrary.org/works/OL1W" in unfolded


def test_ics_folds_long_lines_without_breaking_utf8():
    from app.ics import build_meeting_ics
    from datetime import datetime, timezone

    text = build_meeting_ics(
        pick_id=7,
        club_name="Bücherclub",
        title="Ä" * 120,
        authors="",
        note="",
        starts_at=datetime(2026, 1, 1, 18, 0, tzinfo=timezone.utc),
        ol_work_key="/works/OL7W",
    )
    for line in text.split("\r\n"):
        assert len(line.encode("utf-8")) <= 75
    unfolded = text.replace("\r\n ", "")
    assert "SUMMARY:Bücherclub: " + "Ä" * 120 in unfolded
