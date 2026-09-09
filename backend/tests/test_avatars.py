from io import BytesIO

from PIL import Image

from tests.conftest import login, register


def _png(size=48, color=(180, 40, 40)) -> bytes:
    buf = BytesIO()
    Image.new("RGB", (size, size), color).save(buf, format="PNG")
    return buf.getvalue()


def _upload(client, data: bytes, filename="me.png", content_type="image/png"):
    return client.put(
        "/api/auth/me/avatar",
        files={"file": (filename, data, content_type)},
    )


def test_upload_serves_and_deletes_avatar(client):
    register(client, "ada")
    response = _upload(client, _png())
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["username"] == "ada"
    assert body["avatar_url"] is not None
    assert body["avatar_url"].startswith("/api/members/ada/avatar?v=")
    assert "avatar" not in body

    image = client.get(body["avatar_url"])
    assert image.status_code == 200
    assert image.headers["content-type"] == "image/jpeg"
    assert image.content[:3] == b"\xff\xd8\xff"
    decoded = Image.open(BytesIO(image.content))
    assert decoded.size == (256, 256)

    me = client.get("/api/auth/me").json()
    assert me["avatar_url"] == body["avatar_url"]

    removed = client.delete("/api/auth/me/avatar")
    assert removed.status_code == 200
    assert removed.json()["avatar_url"] is None
    assert client.get(body["avatar_url"]).status_code == 404
    assert client.get("/api/auth/me").json()["avatar_url"] is None


def test_other_members_see_avatar_url(client):
    register(client, "ada")
    _upload(client, _png())
    ada_url = client.get("/api/auth/me").json()["avatar_url"]
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)

    members = client.get("/api/members").json()
    ada = next(row for row in members if row["username"] == "ada")
    assert ada["avatar_url"] == ada_url
    assert client.get(ada_url).status_code == 200

    shelf = client.get("/api/shelf", params={"username": "ada"}).json()
    assert shelf["user"]["avatar_url"] == ada_url


def test_rejects_non_image_and_oversize(client):
    register(client, "ada")
    not_image = _upload(client, b"<svg xmlns='http://www.w3.org/2000/svg'></svg>", "x.svg")
    assert not_image.status_code == 400
    huge = _upload(client, b"\xff\xd8\xff" + b"x" * (2 * 1024 * 1024), "big.jpg")
    assert huge.status_code == 400
    assert client.get("/api/auth/me").json()["avatar_url"] is None


def test_avatar_requires_session(client):
    register(client, "ada")
    _upload(client, _png())
    url = client.get("/api/auth/me").json()["avatar_url"]
    client.post("/api/auth/logout")
    assert _upload(client, _png()).status_code == 401
    assert client.delete("/api/auth/me/avatar").status_code == 401
    assert client.get(url).status_code == 401
    assert login(client, "ada").status_code == 204
    assert client.get(url).status_code == 200
