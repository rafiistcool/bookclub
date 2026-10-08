import json

import pytest
from fastapi.testclient import TestClient
from pydantic import ValidationError
from sqlmodel import Session

from app.models import Book, ShelfEntry, UserPhysicalShelfLayout
from app.schemas import PhysicalShelfLayout
from tests.conftest import login, register


PATH = "/api/shelf/physical-layout"
DEFAULT = {"rows": [[], [], []], "material": "oak", "spines": {}, "decorations": {}}


def _add(client, work: int, **overrides):
    response = client.post(
        "/api/shelf",
        json={
            "ol_work_key": f"/works/OL{work}W",
            "title": f"Book {work}",
            "status": "want_to_read",
            **overrides,
        },
    )
    assert response.status_code == 201, response.text
    return response.json()


def test_layout_requires_authentication(client):
    assert client.get(PATH).status_code == 401
    assert client.put(PATH, json=DEFAULT).status_code == 401


def test_layout_persists_on_reload_and_restart_without_changing_reading_data(client):
    register(client, "ada")
    first = _add(client, 1, status="currently_reading", progress=30)
    second = _add(client, 2, status="finished", rating=5, take="A favourite")
    before = client.get("/api/shelf").json()["items"]
    assert client.get(PATH).json() == DEFAULT
    layout = {
        "rows": [["decor:plant", second["id"], "decor:vase"], [], [first["id"], "decor:frame"], ["decor:candle"]],
        "material": "walnut",
        "spines": {str(first["id"]): {"color": "#Ab3C45", "width": 37}},
        "decorations": {
            "decor:plant": {"kind": "plant", "color": "#AbCdEf"},
            "decor:vase": {"kind": "vase", "color": "#123456"},
            "decor:frame": {"kind": "frame", "color": "#654321"},
            "decor:candle": {"kind": "candle", "color": "#ffeedd"},
        },
    }
    expected = {
        **layout,
        "spines": {str(first["id"]): {"color": "#ab3c45", "width": 37}},
        "decorations": {**layout["decorations"], "decor:plant": {"kind": "plant", "color": "#abcdef"}},
    }
    saved = client.put(PATH, json=layout)
    assert saved.status_code == 200, saved.text
    assert saved.json() == expected
    assert client.get(PATH).json() == expected
    assert client.get("/api/shelf").json()["items"] == before

    # A new app instance connects to the same database; storage is durable.
    from app.main import create_app

    with TestClient(create_app()) as restarted:
        assert login(restarted, "ada").status_code == 204
        assert restarted.get(PATH).json() == expected
        replacement = {"rows": [[], [first["id"], second["id"]]], "material": "white", "spines": {}, "decorations": {}}
        assert restarted.put(PATH, json=replacement).json() == replacement
    assert client.get(PATH).json() == replacement
    with Session(client.app.state.engine) as session:
        assert session.get(UserPhysicalShelfLayout, 1).layout_json


def test_layout_is_private_and_rejects_other_members_and_book_ids(client):
    register(client, "ada")
    ada_book = _add(client, 1)
    layout = {
        "rows": [[ada_book["id"], "decor:plant"], []],
        "material": "walnut",
        "spines": {},
        "decorations": {"decor:plant": {"kind": "plant", "color": "#123456"}},
    }
    assert client.put(PATH, json=layout).status_code == 200
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)
    # The shared book has a different shelf entry ID for each member.
    grace_book = _add(client, 1)
    assert grace_book["id"] != grace_book["book"]["id"]
    assert client.get(PATH, params={"username": "ada"}).json() == DEFAULT
    assert client.put(PATH, json=layout).status_code == 400
    assert client.put(PATH, json={"rows": [[grace_book["book"]["id"]]]}).status_code == 400
    assert client.put(PATH, json={"spines": {str(ada_book["id"]): {"color": "#224466", "width": 30}}}).status_code == 400
    grace_layout = {
        "rows": [[], ["decor:plant", grace_book["id"]]],
        "material": "white",
        "spines": {},
        "decorations": {"decor:plant": {"kind": "vase", "color": "#abcdef"}},
    }
    assert client.put(PATH, json=grace_layout).status_code == 200
    assert login(client, "ada").status_code == 204
    assert client.get(PATH).json() == layout
    assert login(client, "grace").status_code == 204
    assert client.get(PATH).json() == grace_layout


def test_status_changes_do_not_move_books_and_deleted_entries_are_pruned(client):
    register(client, "ada")
    first = _add(client, 1)
    second = _add(client, 2)
    layout = {
        "rows": [["decor:plant"], [first["id"], "decor:frame", second["id"]], []],
        "material": "oak",
        "spines": {
            str(first["id"]): {"color": "#123456", "width": 26},
            str(second["id"]): {"color": "#abcdef", "width": 64},
        },
        "decorations": {
            "decor:plant": {"kind": "plant", "color": "#123456"},
            "decor:frame": {"kind": "frame", "color": "#abcdef"},
        },
    }
    assert client.put(PATH, json=layout).status_code == 200
    assert client.patch(f"/api/shelf/{first['id']}", json={"status": "finished"}).status_code == 200
    assert client.get(PATH).json() == layout
    newest = _add(client, 3)
    assert newest["id"] not in {entry_id for row in client.get(PATH).json()["rows"] for entry_id in row}
    assert client.delete(f"/api/shelf/{first['id']}").status_code == 204
    assert client.get(PATH).json() == {
        **layout,
        "rows": [["decor:plant"], ["decor:frame", second["id"]], []],
        "spines": {str(second["id"]): layout["spines"][str(second["id"])]},
    }


def test_deleted_spine_does_not_leak_to_a_book_with_a_reused_id(client):
    register(client, "ada")
    first = _add(client, 1)
    assert client.put(PATH, json={
        "rows": [[], [first["id"]]],
        "spines": {str(first["id"]): {"color": "#abcdef", "width": 44}},
    }).status_code == 200
    assert client.delete(f"/api/shelf/{first['id']}").status_code == 204
    # No intervening GET; the delete itself must clean the stored layout.
    replacement = _add(client, 2)
    assert replacement["id"] == first["id"]
    assert client.get(PATH).json() == {"rows": [[], []], "material": "oak", "spines": {}, "decorations": {}}


def test_get_prunes_entries_removed_outside_the_shelf_endpoint(client):
    register(client, "ada")
    first = _add(client, 1)
    assert client.put(PATH, json={
        "rows": [["decor:plant", first["id"], "decor:vase"], [], []],
        "spines": {str(first["id"]): {"color": "#abcdef", "width": 44}},
        "decorations": {
            "decor:plant": {"kind": "plant", "color": "#123456"},
            "decor:vase": {"kind": "vase", "color": "#abcdef"},
        },
    }).status_code == 200
    with Session(client.app.state.engine) as session:
        session.delete(session.get(ShelfEntry, first["id"]))
        session.commit()
    assert client.get(PATH).json() == {
        **DEFAULT,
        "rows": [["decor:plant", "decor:vase"], [], []],
        "decorations": {
            "decor:plant": {"kind": "plant", "color": "#123456"},
            "decor:vase": {"kind": "vase", "color": "#abcdef"},
        },
    }


@pytest.mark.parametrize(
    "invalid",
    [
        {"rows": []},
        {"rows": [[] for _ in range(13)]},
        {"rows": [[1], [1]]},
        {"rows": [list(range(1, 2002))]},
        {"rows": [list(range(1, 1002)), list(range(1002, 2002))]},
        {"rows": [[0]]},
        {"rows": [[True]]},
        {"rows": [["1"]]},
        {"rows": [[1.5]]},
        {"rows": [[9999]]},
        {"material": "pine"},
        {"spines": {"1": {"color": "red", "width": 30}}},
        {"spines": {"1": {"color": "#abcd", "width": 30}}},
        {"spines": {"1": {"color": "#123456", "width": 25}}},
        {"spines": {"1": {"color": "#123456", "width": 65}}},
        {"spines": {"1": {"color": "#123456", "width": 30.5}}},
        {"spines": {"01": {"color": "#123456", "width": 30}}},
        {"spines": {"foo": {"color": "#123456", "width": 30}}},
        {"spines": {"9999": {"color": "#123456", "width": 30}}},
        {"spines": {"9" * 5000: {"color": "#123456", "width": 30}}},
        {"rows": [["decor:missing"]]},
        {"decorations": {"decor:orphan": {"kind": "plant", "color": "#123456"}}},
        {
            "rows": [["decor:plant"], ["decor:plant"]],
            "decorations": {"decor:plant": {"kind": "plant", "color": "#123456"}},
        },
        {
            "rows": [["decor:plant"]],
            "decorations": {"decor:other": {"kind": "plant", "color": "#123456"}},
        },
        {"rows": [["decor:"]], "decorations": {"decor:": {"kind": "plant", "color": "#123456"}}},
        {"rows": [["plant"]], "decorations": {"plant": {"kind": "plant", "color": "#123456"}}},
        {
            "rows": [["decor:space name"]],
            "decorations": {"decor:space name": {"kind": "plant", "color": "#123456"}},
        },
        {
            "rows": [["decor:" + "a" * 65]],
            "decorations": {"decor:" + "a" * 65: {"kind": "plant", "color": "#123456"}},
        },
        {
            "rows": [["decor:plant"]],
            "decorations": {"decor:plant": {"kind": "unknown", "color": "#123456"}},
        },
        {
            "rows": [["decor:plant"]],
            "decorations": {"decor:plant": {"kind": "plant", "color": "red"}},
        },
        {
            "rows": [["decor:plant"]],
            "decorations": {"decor:plant": {"kind": "plant", "color": "#abcd"}},
        },
        {"rows": [["decor:plant"]], "decorations": {"decor:plant": {"kind": "plant"}}},
        {"rows": [["decor:plant"]], "decorations": {"decor:plant": {"color": "#123456"}}},
        {"decorations": []},
        {
            "rows": [[f"decor:{i}" for i in range(101)]],
            "decorations": {f"decor:{i}": {"kind": "plant", "color": "#123456"} for i in range(101)},
        },
    ],
)
def test_invalid_layout_does_not_replace_previous_layout(client, invalid):
    register(client, "ada")
    first = _add(client, 1)
    layout = {"rows": [[first["id"]]], "material": "walnut", "spines": {}, "decorations": {}}
    assert client.put(PATH, json=layout).status_code == 200
    rejected = client.put(PATH, json=invalid)
    assert rejected.status_code == 400, rejected.text
    assert client.get(PATH).json() == layout


def test_legacy_stored_layout_still_loads_and_prunes_missing_books(client):
    register(client, "ada")
    first = _add(client, 1)
    legacy = {
        "rows": [[], [first["id"], 9999], []],
        "material": "walnut",
        "spines": {
            str(first["id"]): {"color": "#abcdef", "width": 44},
            "9999": {"color": "#123456", "width": 26},
        },
    }
    with Session(client.app.state.engine) as session:
        session.add(UserPhysicalShelfLayout(user_id=1, layout_json=json.dumps(legacy)))
        session.commit()
    assert client.get(PATH).json() == {
        "rows": [[], [first["id"]], []],
        "material": "walnut",
        "spines": {str(first["id"]): legacy["spines"][str(first["id"])]},
        "decorations": {},
    }
    assert client.delete(f"/api/shelf/{first['id']}").status_code == 204
    assert client.get(PATH).json() == {**DEFAULT, "material": "walnut"}


def test_decorations_can_be_saved_without_books_and_removed(client):
    register(client, "ada")
    decoration_id = "decor:" + "Aa_0-" + "b" * 59
    layout = {
        **DEFAULT,
        "rows": [[decoration_id]],
        "decorations": {decoration_id: {"kind": "plant", "color": "#123456"}},
    }
    assert client.put(PATH, json=layout).json() == layout
    assert client.get(PATH).json() == layout
    assert client.put(PATH, json=DEFAULT).json() == DEFAULT
    assert client.get(PATH).json() == DEFAULT


def test_mixed_layout_limits_books_and_decorations_separately():
    decorations = {f"decor:{i}": {"kind": "plant", "color": "#123456"} for i in range(100)}
    rows = [list(range(1, 2001)), list(decorations)]
    layout = PhysicalShelfLayout.model_validate({"rows": rows, "decorations": decorations})
    assert layout.rows == rows
    assert len(layout.decorations) == 100
    with pytest.raises(ValidationError, match="At most 2000 books"):
        PhysicalShelfLayout.model_validate({"rows": [list(range(1, 2002))]})
    with pytest.raises(ValidationError, match="At most 2100 items"):
        PhysicalShelfLayout.model_validate({"rows": [list(range(1, 2002)), list(decorations)], "decorations": decorations})


def test_shelf_book_exposes_known_page_count(client):
    register(client, "ada")
    entry = _add(client, 1)
    assert entry["book"]["pages"] is None
    with Session(client.app.state.engine) as session:
        book = session.get(Book, entry["book"]["id"])
        book.pages = 480
        session.add(book)
        session.commit()
    assert client.get("/api/shelf").json()["items"][0]["book"]["pages"] == 480
