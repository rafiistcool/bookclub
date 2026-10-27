CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username VARCHAR(32) NOT NULL UNIQUE,
    password_hash VARCHAR NOT NULL,
    created_at DATETIME NOT NULL
);
CREATE TABLE books (
    id INTEGER PRIMARY KEY,
    ol_work_key VARCHAR(64) NOT NULL UNIQUE,
    title VARCHAR NOT NULL,
    authors VARCHAR NOT NULL,
    cover_id INTEGER,
    year INTEGER
);
CREATE TABLE shelf (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    book_id INTEGER NOT NULL REFERENCES books(id),
    status VARCHAR NOT NULL,
    position INTEGER NOT NULL,
    updated_at DATETIME NOT NULL,
    UNIQUE (user_id, book_id)
);
CREATE TABLE club_picks (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books(id),
    set_by_id INTEGER NOT NULL REFERENCES users(id),
    note VARCHAR(280) NOT NULL,
    started_at DATETIME NOT NULL,
    ended_at DATETIME
);
CREATE TABLE club_pick_posts (
    id INTEGER PRIMARY KEY,
    pick_id INTEGER NOT NULL REFERENCES club_picks(id),
    author_id INTEGER NOT NULL REFERENCES users(id),
    body VARCHAR(1000) NOT NULL,
    created_at DATETIME NOT NULL
);
CREATE TABLE next_up_votes (
    id INTEGER PRIMARY KEY,
    started_at DATETIME NOT NULL,
    ended_at DATETIME,
    winner_nomination_id INTEGER
);
INSERT INTO users (id, username, password_hash, created_at)
    VALUES (1, 'ada', 'x', '2024-01-01 00:00:00');
INSERT INTO books (id, ol_work_key, title, authors, cover_id, year)
    VALUES (1, '/works/OL1W', 'Circe', 'Madeline Miller', 1, 2018);
INSERT INTO shelf (id, user_id, book_id, status, position, updated_at)
    VALUES (1, 1, 1, 'finished', 0, '2024-01-02 00:00:00');
