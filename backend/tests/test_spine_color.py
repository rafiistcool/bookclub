from io import BytesIO

import httpx
import pytest
from PIL import Image
from sqlmodel import Session, select

from app import spine_color
from app.models import Book
from tests.conftest import register


def _jpeg(color: tuple[int, int, int], size=(120, 180), border=None) -> bytes:
    image = Image.new("RGB", size, border or color)
    if border:
        image.paste(color, (12, 18, size[0] - 12, size[1] - 18))
    buffer = BytesIO()
    image.save(buffer, format="JPEG", quality=92)
    return buffer.getvalue()


def _rgb(value: str) -> tuple[int, int, int]:
    return tuple(int(value[index : index + 2], 16) for index in (1, 3, 5))


def test_dominant_color_reads_the_cover_and_ignores_its_border():
    color = spine_color.dominant_color(_jpeg((30, 90, 160), border=(255, 255, 255)))
    red, green, blue = _rgb(color)
    assert blue > green > red
    assert abs(blue - 160) < 20


def test_dominant_color_prefers_a_vivid_colour_over_a_slightly_larger_pale_one():
    # A light grey panel covers a little more of the cover than the deep green one.
    image = Image.new("RGB", (120, 180), (205, 205, 200))
    image.paste((20, 95, 40), (0, 0, 120, 80))
    buffer = BytesIO()
    image.save(buffer, format="PNG")
    red, green, blue = _rgb(spine_color.dominant_color(buffer.getvalue()))
    assert green > red + 40 and green > blue + 30


def test_dominant_color_keeps_extremes_printable_and_rejects_junk():
    # Pure black and white still leave room for the spine's shading.
    assert _rgb(spine_color.dominant_color(_jpeg((0, 0, 0))))[0] > 30
    assert _rgb(spine_color.dominant_color(_jpeg((255, 255, 255))))[0] < 235
    assert spine_color.dominant_color(b"") is None
    assert spine_color.dominant_color(b"not an image at all, just some text") is None
    # Open Library's 1×1 "no cover" pixel is not a colour.
    assert spine_color.dominant_color(_jpeg((200, 20, 20), size=(1, 1))) is None


def _add(client, work: int, **overrides) -> dict:
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


def test_spine_colors_are_sampled_once_and_served_with_the_book(client, monkeypatch):
    register(client, "ada")
    covered = _add(client, 1, cover_id=111)
    bare = _add(client, 2)
    calls: list[str] = []

    async def fake_fetch(url: str) -> bytes:
        calls.append(url)
        return _jpeg((150, 40, 40))

    monkeypatch.setattr(spine_color, "fetch_cover", fake_fetch)
    assert covered["book"]["spine_color"] is None

    result = client.post("/api/shelf/spine-colors").json()
    assert result["remaining"] == 0
    assert list(result["colors"]) == [str(covered["book"]["id"])]
    assert calls == ["https://covers.openlibrary.org/b/id/111-M.jpg?default=false"]

    books = {item["id"]: item["book"] for item in client.get("/api/shelf").json()["items"]}
    assert books[covered["id"]]["spine_color"] == result["colors"][str(covered["book"]["id"])]
    # A book without a cover is settled too, and nothing is fetched twice.
    assert books[bare["id"]]["spine_color"] is None
    assert client.post("/api/shelf/spine-colors").json() == {"colors": {}, "remaining": 0}
    assert len(calls) == 1


def test_spine_colors_retry_after_network_trouble_but_not_after_a_missing_cover(
    client, monkeypatch
):
    register(client, "ada")
    _add(client, 1, cover_id=111)
    outcome: dict[str, object] = {"value": httpx.ConnectError("down")}

    async def fake_fetch(url: str) -> bytes | None:
        if isinstance(outcome["value"], Exception):
            raise outcome["value"]
        return outcome["value"]

    monkeypatch.setattr(spine_color, "fetch_cover", fake_fetch)
    assert client.post("/api/shelf/spine-colors").json() == {"colors": {}, "remaining": 0}
    with Session(client.app.state.engine) as session:
        assert session.exec(select(Book)).one().spine_color is None

    outcome["value"] = None  # the host answered: there is no such cover
    assert client.post("/api/shelf/spine-colors").json() == {"colors": {}, "remaining": 0}
    with Session(client.app.state.engine) as session:
        assert session.exec(select(Book)).one().spine_color == ""


def test_a_new_cover_is_sampled_again(client, monkeypatch):
    register(client, "ada")
    _add(client, 1, cover_id=111)

    async def fake_fetch(url: str) -> bytes:
        return _jpeg((40, 120, 60) if "222" in url else (150, 40, 40))

    monkeypatch.setattr(spine_color, "fetch_cover", fake_fetch)
    first = client.post("/api/shelf/spine-colors").json()["colors"]
    with Session(client.app.state.engine) as session:
        book = session.exec(select(Book)).one()
        from app.shelf_ops import apply_book_details

        apply_book_details(book, cover_id=222)
        session.add(book)
        session.commit()
    second = client.post("/api/shelf/spine-colors").json()["colors"]
    assert second and second != first


def test_spine_colors_only_cover_the_callers_shelf_and_need_a_login(client, monkeypatch):
    assert client.post("/api/shelf/spine-colors").status_code == 401
    register(client, "ada")
    _add(client, 1, cover_id=111)
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)

    async def fake_fetch(url: str) -> bytes:
        pytest.fail("another member's books must not be fetched")

    monkeypatch.setattr(spine_color, "fetch_cover", fake_fetch)
    assert client.post("/api/shelf/spine-colors").json() == {"colors": {}, "remaining": 0}
