from tests.conftest import register


def test_members_hide_self_and_preview_reading(client):
    register(client, "ada")
    client.post(
        "/api/shelf",
        json={
            "ol_work_key": "/works/OL1W",
            "title": "Circe",
            "authors": "Madeline Miller",
            "cover_id": 11,
            "year": 2018,
            "status": "currently_reading",
        },
    )
    invite = client.post("/api/invites").json()["code"]
    client.post("/api/auth/logout")
    register(client, "grace", invite=invite)

    members = client.get("/api/members").json()
    assert len(members) == 1
    assert members[0]["username"] == "ada"
    assert members[0]["currently_reading_count"] == 1
    assert members[0]["currently_reading_preview"][0]["title"] == "Circe"

    mine = client.get("/api/members")
    assert all(row["username"] != "grace" for row in mine.json())
