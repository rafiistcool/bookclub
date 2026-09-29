from tests.conftest import login, register


def test_history_requires_login(client):
    assert client.get('/api/search-history').status_code == 401
    assert client.post('/api/search-history', json={'query': 'Dune'}).status_code == 401
    assert client.delete('/api/search-history').status_code == 401


def test_history_deduplicates_orders_and_caps(client):
    register(client, 'ada')
    for index in range(22):
        response = client.post('/api/search-history', json={'query': f'Book {index}'})
        assert response.status_code == 200, response.text
    history = client.get('/api/search-history').json()
    assert len(history) == 20
    assert history[0]['query'] == 'Book 21'
    assert history[-1]['query'] == 'Book 2'
    original = history[-1]['id']
    response = client.post('/api/search-history', json={'query': '  BOOK   2  '})
    assert response.json()[0]['id'] == original
    assert response.json()[0]['query'] == 'BOOK 2'
    assert len(response.json()) == 20
    assert client.post('/api/search-history', json={'query': '  '}).status_code == 400
    assert client.post('/api/search-history', json={'query': 'x' * 301}).status_code == 400


def test_history_is_private_and_delete_is_scoped(client):
    register(client, 'ada')
    entry = client.post('/api/search-history', json={'query': 'Circe'}).json()[0]
    invite = client.post('/api/invites').json()['code']
    register(client, 'bob', invite=invite)
    assert client.get('/api/search-history').json() == []
    assert client.delete(f"/api/search-history/{entry['id']}").status_code == 404
    client.post('/api/search-history', json={'query': 'Dune'})
    assert client.delete('/api/search-history').status_code == 204
    assert client.get('/api/search-history').json() == []
    login(client, 'ada')
    assert client.get('/api/search-history').json()[0]['query'] == 'Circe'
    assert client.delete(f"/api/search-history/{entry['id']}").status_code == 204
    assert client.get('/api/search-history').json() == []
