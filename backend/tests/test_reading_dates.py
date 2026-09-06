from tests.conftest import register

BOOK = {
    "ol_work_key": "/works/OL1W",
    "title": "Circe",
    "authors": "Madeline Miller",
    "cover_id": 1,
    "year": 2018,
}


def _add(client, status, **extra):
    response = client.post("/api/shelf", json={**BOOK, "status": status, **extra})
    assert response.status_code == 201, response.text
    return response.json()


def _patch(client, entry_id, **body):
    response = client.patch(f"/api/shelf/{entry_id}", json=body)
    assert response.status_code == 200, response.text
    return response.json()


def test_want_to_read_has_no_dates(client):
    register(client, "ada")
    item = _add(client, "want_to_read")
    assert item["started_at"] is None
    assert item["finished_at"] is None


def test_reading_sets_started_and_finish_sets_finished(client):
    register(client, "ada")
    item = _add(client, "want_to_read")
    reading = _patch(client, item["id"], status="currently_reading", position=0)
    assert reading["started_at"] is not None
    assert reading["finished_at"] is None
    started = reading["started_at"]

    finished = _patch(client, item["id"], status="finished", position=0, rating=4)
    assert finished["started_at"] == started
    assert finished["finished_at"] is not None
    assert finished["finished_at"] >= started


def test_added_directly_as_finished_stamps_both(client):
    register(client, "ada")
    item = _add(client, "finished", rating=5)
    assert item["started_at"] is not None
    assert item["finished_at"] is not None


def test_reorder_within_finished_keeps_finished_at(client):
    register(client, "ada")
    item = _add(client, "finished", rating=5)
    first = item["finished_at"]
    moved = _patch(client, item["id"], position=0)
    assert moved["finished_at"] == first


def test_leaving_finished_clears_finished_at_but_keeps_start(client):
    register(client, "ada")
    item = _add(client, "finished", rating=5)
    started = item["started_at"]
    back = _patch(client, item["id"], status="currently_reading", position=0)
    assert back["finished_at"] is None
    assert back["started_at"] == started


def test_back_to_want_to_read_resets_dates(client):
    register(client, "ada")
    item = _add(client, "finished", rating=5)
    reset = _patch(client, item["id"], status="want_to_read", position=0)
    assert reset["started_at"] is None
    assert reset["finished_at"] is None


def test_dnf_has_start_but_no_finish(client):
    register(client, "ada")
    item = _add(client, "did_not_finish", dnf_reason="Too long")
    assert item["started_at"] is not None
    assert item["finished_at"] is None
