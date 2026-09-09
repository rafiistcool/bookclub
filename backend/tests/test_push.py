import base64
from datetime import datetime, timedelta, timezone

import pytest
from sqlmodel import Session, select

from app import push
from app.models import ClubPick
from tests.conftest import register

CIRCE = {"ol_work_key": "/works/OL1W", "title": "Circe", "authors": "Madeline Miller", "cover_id": 1, "year": 2018}
ACHILLES = {"ol_work_key": "/works/OL2W", "title": "The Song of Achilles", "authors": "Madeline Miller", "cover_id": 2, "year": 2011}


def _sub(endpoint="https://push.example/abc123def456"):
    return {"endpoint": endpoint, "keys": {"p256dh": "BPK", "auth": "AUTH"}, "user_agent": "Test/1.0"}


@pytest.fixture
def sent(monkeypatch):
    calls: list[tuple[dict, dict]] = []

    def fake_send(target, payload):
        calls.append((target, payload))
        return False if target["endpoint"].endswith("/dead") else True

    monkeypatch.setattr(push, "send_to_subscription", fake_send)
    return calls


def _add_member(client, name):
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, name, invite=invite)


def test_vapid_key_is_generated_and_stable(client, tmp_path):
    register(client, "ada")
    push.reset_keys()
    first = client.get("/api/push/vapid").json()["public_key"]
    assert len(first) == 87  # 65-byte uncompressed point, base64url, no padding
    padded = first + "=" * (-len(first) % 4)
    raw = base64.urlsafe_b64decode(padded)
    assert raw[0] == 0x04 and len(raw) == 65
    assert (tmp_path / ".vapid_private.pem").is_file()
    push.reset_keys()
    assert client.get("/api/push/vapid").json()["public_key"] == first


def test_subscribe_list_unsubscribe(client, sent):
    register(client, "ada")
    assert client.get("/api/push/subscriptions").json() == {"items": []}
    assert client.post("/api/push/test").status_code == 400
    bad = client.post("/api/push/subscriptions", json=_sub("http://insecure"))
    assert bad.status_code == 400

    created = client.post("/api/push/subscriptions", json=_sub())
    assert created.status_code == 201
    assert created.json()["endpoint_tail"] == "abc123def456"
    assert created.json()["user_agent"] == "Test/1.0"

    # Re-subscribing the same endpoint updates instead of duplicating.
    again = client.post("/api/push/subscriptions", json=_sub())
    assert again.status_code == 201
    assert len(client.get("/api/push/subscriptions").json()["items"]) == 1

    test = client.post("/api/push/test")
    assert test.status_code == 200
    assert test.json() == {"sent_to": 1}
    assert sent[-1][1]["kind"] == "test"

    assert client.request("DELETE", "/api/push/subscriptions", json={"endpoint": "https://push.example/nope"}).status_code == 404
    assert client.request("DELETE", "/api/push/subscriptions", json={"endpoint": "https://push.example/abc123def456"}).status_code == 204
    assert client.get("/api/push/subscriptions").json() == {"items": []}


def test_new_pick_notifies_other_members_who_opted_in(client, sent):
    register(client, "ada")
    client.post("/api/push/subscriptions", json=_sub("https://push.example/ada"))
    _add_member(client, "grace")
    client.post("/api/push/subscriptions", json=_sub("https://push.example/grace"))
    _add_member(client, "tom")
    client.post("/api/push/subscriptions", json=_sub("https://push.example/tom"))
    client.patch("/api/auth/notifications", json={"notify_pick": False})

    client.put("/api/pick", json=CIRCE)  # tom sets the pick
    targets = sorted(target["endpoint"] for target, _ in sent)
    assert targets == ["https://push.example/ada", "https://push.example/grace"]
    payload = sent[0][1]
    assert payload["kind"] == "pick"
    assert "tom chose Circe" in payload["body"]
    assert payload["title"].startswith("Bookclub")

    # Updating the same pick (meeting only) does not re-notify.
    sent.clear()
    client.put("/api/pick", json={**CIRCE, "meeting_at": "2030-01-01T18:00"})
    assert sent == []


def test_new_note_notifies_except_author_and_respects_pref(client, sent):
    register(client, "ada")
    client.post("/api/push/subscriptions", json=_sub("https://push.example/ada"))
    _add_member(client, "grace")
    client.post("/api/push/subscriptions", json=_sub("https://push.example/grace"))
    client.patch("/api/auth/notifications", json={"notify_note": False})
    client.put("/api/pick", json=CIRCE)
    sent.clear()

    client.post("/api/pick/posts", json={"body": "A very long note " * 10})
    # grace posted; grace excluded as author; ada gets it.
    assert [t["endpoint"] for t, _ in sent] == ["https://push.example/ada"]
    payload = sent[0][1]
    assert payload["kind"] == "note"
    assert payload["title"] == "grace on Circe"
    assert payload["body"].endswith("…")
    assert len(payload["body"]) <= 90

    sent.clear()
    client.post("/api/auth/logout")
    client.post("/api/auth/login", json={"username": "ada", "password": "password1"})
    client.post("/api/pick/posts", json={"body": "Reply"})
    assert sent == []  # grace opted out of note pushes


def test_dead_subscriptions_are_pruned(client, sent):
    register(client, "ada")
    client.post("/api/push/subscriptions", json=_sub("https://push.example/dead"))
    _add_member(client, "grace")
    client.put("/api/pick", json=CIRCE)
    assert [t["endpoint"] for t, _ in sent] == ["https://push.example/dead"]
    client.post("/api/auth/logout")
    client.post("/api/auth/login", json={"username": "ada", "password": "password1"})
    assert client.get("/api/push/subscriptions").json() == {"items": []}


def test_push_copy_follows_recipient_locale(client, sent):
    register(client, "ada")
    client.patch("/api/auth/me/preferences", json={"locale": "de"})
    client.post("/api/push/subscriptions", json=_sub("https://push.example/ada"))
    _add_member(client, "grace")
    client.post("/api/push/subscriptions", json=_sub("https://push.example/grace"))
    _add_member(client, "tom")

    client.put("/api/pick", json=CIRCE)
    by_endpoint = {target["endpoint"]: payload for target, payload in sent}
    assert "gewählt" in by_endpoint["https://push.example/ada"]["body"]
    assert "chose Circe" in by_endpoint["https://push.example/grace"]["body"]


def test_meeting_reminder_sent_once_inside_24h(client, sent):
    register(client, "ada")
    client.post("/api/push/subscriptions", json=_sub("https://push.example/ada"))
    soon = (datetime.now(timezone.utc) + timedelta(hours=5)).strftime("%Y-%m-%dT%H:%M")
    client.put("/api/pick", json={**CIRCE, "meeting_at": soon})
    sent.clear()

    client.get("/api/pick")
    assert len(sent) == 1
    assert sent[0][1]["kind"] == "meeting"
    assert "Circe" in sent[0][1]["body"]

    client.get("/api/pick")
    client.get("/api/pick")
    assert len(sent) == 1  # reminder is one-shot

    with Session(client.app.state.engine) as session:
        pick = session.exec(select(ClubPick).where(ClubPick.ended_at.is_(None))).one()
        assert pick.reminder_sent_at is not None


def test_meeting_reminder_not_sent_far_ahead_or_when_opted_out(client, sent):
    register(client, "ada")
    client.post("/api/push/subscriptions", json=_sub("https://push.example/ada"))
    far = (datetime.now(timezone.utc) + timedelta(days=10)).strftime("%Y-%m-%dT%H:%M")
    client.put("/api/pick", json={**CIRCE, "meeting_at": far})
    sent.clear()
    client.get("/api/pick")
    assert sent == []

    client.patch("/api/auth/notifications", json={"notify_meeting": False})
    soon = (datetime.now(timezone.utc) + timedelta(hours=2)).strftime("%Y-%m-%dT%H:%M")
    client.put("/api/pick", json={**CIRCE, "meeting_at": soon})
    sent.clear()
    client.get("/api/pick")
    assert sent == []


def test_vote_close_notifies_new_pick(client, sent):
    register(client, "ada")
    _add_member(client, "grace")
    client.post("/api/push/subscriptions", json=_sub("https://push.example/grace"))
    client.post("/api/auth/logout")
    client.post("/api/auth/login", json={"username": "ada", "password": "password1"})
    client.post("/api/vote/nominations", json=ACHILLES)
    nomination_id = client.get("/api/vote").json()["nominations"][0]["id"]
    sent.clear()
    client.post("/api/vote/apply", json={"nomination_id": nomination_id})
    assert [t["endpoint"] for t, _ in sent] == ["https://push.example/grace"]
    assert sent[0][1]["kind"] == "pick"
