import pytest
from fastapi.testclient import TestClient


@pytest.mark.parametrize("path", ["/", "/index.html", "/discover", "/book/ISBN9781408857892"])
def test_app_html_is_not_cached_across_releases(client, tmp_path, monkeypatch, path):
    from app import main

    static = tmp_path / "static"
    static.mkdir()
    (static / "index.html").write_text('<html><script src="/assets/current.js"></script></html>')
    monkeypatch.setattr(main, "STATIC_DIR", static)
    with TestClient(main.create_app()) as browser:
        response = browser.get(path)
    assert response.status_code == 200
    assert "current.js" in response.text
    assert response.headers["cache-control"] == "no-store"
