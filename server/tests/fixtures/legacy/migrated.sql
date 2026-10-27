BEGIN TRANSACTION;
CREATE TABLE book_post_reactions (
	id INTEGER NOT NULL, 
	post_id INTEGER NOT NULL, 
	user_id INTEGER NOT NULL, 
	emoji VARCHAR(16) NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	CONSTRAINT uq_book_reaction_post_user_emoji UNIQUE (post_id, user_id, emoji), 
	FOREIGN KEY(post_id) REFERENCES book_posts (id), 
	FOREIGN KEY(user_id) REFERENCES users (id)
);
CREATE TABLE book_posts (
	id INTEGER NOT NULL, 
	book_id INTEGER NOT NULL, 
	author_id INTEGER NOT NULL, 
	parent_id INTEGER, 
	body VARCHAR(1000) NOT NULL, 
	spoiler_upto INTEGER, 
	progress_at INTEGER, 
	status_at VARCHAR(17), 
	pick_id INTEGER, 
	legacy_post_id INTEGER, 
	deleted_at DATETIME, 
	created_at DATETIME NOT NULL, 
	updated_at DATETIME, 
	PRIMARY KEY (id), 
	FOREIGN KEY(book_id) REFERENCES books (id), 
	FOREIGN KEY(author_id) REFERENCES users (id), 
	FOREIGN KEY(parent_id) REFERENCES book_posts (id), 
	FOREIGN KEY(pick_id) REFERENCES club_picks (id), 
	UNIQUE (legacy_post_id)
);
INSERT INTO "book_posts" VALUES(1,1,2,NULL,'Written before the diary existed',NULL,NULL,NULL,1,1,NULL,'2024-01-06 20:15:00.000000',NULL);
CREATE TABLE books (
    id INTEGER PRIMARY KEY,
    ol_work_key VARCHAR(64) NOT NULL UNIQUE,
    title VARCHAR NOT NULL,
    authors VARCHAR NOT NULL,
    cover_id INTEGER,
    year INTEGER
, description VARCHAR NOT NULL DEFAULT '', pages INTEGER, subjects VARCHAR NOT NULL DEFAULT '[]', ol_rating FLOAT, details_fetched_at DATETIME, cover_image_url VARCHAR, spine_color VARCHAR);
INSERT INTO "books" VALUES(1,'/works/OL1W','Circe','Madeline Miller',1,2018,'',NULL,'[]',NULL,NULL,NULL,NULL);
INSERT INTO "books" VALUES(2,'/works/OL2W','No Cover','Anon',-1,1999,'',NULL,'[]',NULL,NULL,NULL,NULL);
CREATE TABLE club_pick_posts (
    id INTEGER PRIMARY KEY,
    pick_id INTEGER NOT NULL REFERENCES club_picks(id),
    author_id INTEGER NOT NULL REFERENCES users(id),
    body VARCHAR(1000) NOT NULL,
    created_at DATETIME NOT NULL
, spoiler_upto INTEGER, milestone_id INTEGER REFERENCES pick_milestones(id), updated_at DATETIME);
INSERT INTO "club_pick_posts" VALUES(1,1,2,'Written before the diary existed','2024-01-06 20:15:00',NULL,NULL,NULL);
CREATE TABLE club_picks (
    id INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL REFERENCES books(id),
    set_by_id INTEGER NOT NULL REFERENCES users(id),
    note VARCHAR(280) NOT NULL,
    started_at DATETIME NOT NULL,
    ended_at DATETIME
, meeting_at DATETIME, reminder_sent_at DATETIME);
INSERT INTO "club_picks" VALUES(1,1,1,'First pick','2024-01-05 18:00:00',NULL,NULL,NULL);
CREATE TABLE events (
	id INTEGER NOT NULL, 
	actor_id INTEGER NOT NULL, 
	kind VARCHAR(32) NOT NULL, 
	book_id INTEGER, 
	pick_id INTEGER, 
	payload VARCHAR NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	FOREIGN KEY(actor_id) REFERENCES users (id), 
	FOREIGN KEY(book_id) REFERENCES books (id), 
	FOREIGN KEY(pick_id) REFERENCES club_picks (id)
);
CREATE TABLE invites (
	id INTEGER NOT NULL, 
	code VARCHAR(32) NOT NULL, 
	created_by_id INTEGER, 
	used_by_id INTEGER, 
	created_at DATETIME NOT NULL, 
	used_at DATETIME, 
	PRIMARY KEY (id), 
	FOREIGN KEY(created_by_id) REFERENCES users (id), 
	FOREIGN KEY(used_by_id) REFERENCES users (id)
);
CREATE TABLE next_up_ballots (
	id INTEGER NOT NULL, 
	vote_id INTEGER NOT NULL, 
	nomination_id INTEGER NOT NULL, 
	user_id INTEGER NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	CONSTRAINT uq_next_up_vote_user UNIQUE (vote_id, user_id), 
	FOREIGN KEY(vote_id) REFERENCES next_up_votes (id), 
	FOREIGN KEY(nomination_id) REFERENCES next_up_nominations (id), 
	FOREIGN KEY(user_id) REFERENCES users (id)
);
CREATE TABLE next_up_nominations (
	id INTEGER NOT NULL, 
	vote_id INTEGER NOT NULL, 
	book_id INTEGER NOT NULL, 
	nominated_by_id INTEGER NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	CONSTRAINT uq_next_up_vote_book UNIQUE (vote_id, book_id), 
	FOREIGN KEY(vote_id) REFERENCES next_up_votes (id), 
	FOREIGN KEY(book_id) REFERENCES books (id), 
	FOREIGN KEY(nominated_by_id) REFERENCES users (id)
);
CREATE TABLE next_up_votes (
    id INTEGER PRIMARY KEY,
    started_at DATETIME NOT NULL,
    ended_at DATETIME,
    winner_nomination_id INTEGER
, closes_at DATETIME);
INSERT INTO "next_up_votes" VALUES(1,'2024-01-01 12:00:00','2024-01-04 12:00:00',NULL,NULL);
CREATE TABLE physical_shelf_layouts (
	user_id INTEGER NOT NULL, 
	layout_json VARCHAR NOT NULL, 
	PRIMARY KEY (user_id), 
	FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE
);
CREATE TABLE pick_milestones (
	id INTEGER NOT NULL, 
	pick_id INTEGER NOT NULL, 
	title VARCHAR(120) NOT NULL, 
	chapter_from INTEGER, 
	chapter_to INTEGER, 
	due_at DATETIME, 
	position INTEGER NOT NULL, 
	created_by_id INTEGER NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	FOREIGN KEY(pick_id) REFERENCES club_picks (id), 
	FOREIGN KEY(created_by_id) REFERENCES users (id)
);
CREATE TABLE post_reactions (
	id INTEGER NOT NULL, 
	post_id INTEGER NOT NULL, 
	user_id INTEGER NOT NULL, 
	emoji VARCHAR(16) NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	CONSTRAINT uq_reaction_post_user_emoji UNIQUE (post_id, user_id, emoji), 
	FOREIGN KEY(post_id) REFERENCES club_pick_posts (id), 
	FOREIGN KEY(user_id) REFERENCES users (id)
);
CREATE TABLE push_subscriptions (
	id INTEGER NOT NULL, 
	user_id INTEGER NOT NULL, 
	endpoint VARCHAR(1024) NOT NULL, 
	p256dh VARCHAR(256) NOT NULL, 
	auth VARCHAR(128) NOT NULL, 
	user_agent VARCHAR(256) NOT NULL, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	FOREIGN KEY(user_id) REFERENCES users (id), 
	UNIQUE (endpoint)
);
CREATE TABLE quotes (
	id INTEGER NOT NULL, 
	user_id INTEGER NOT NULL, 
	book_id INTEGER NOT NULL, 
	body VARCHAR(1000) NOT NULL, 
	page INTEGER, 
	created_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	FOREIGN KEY(user_id) REFERENCES users (id), 
	FOREIGN KEY(book_id) REFERENCES books (id)
);
CREATE TABLE search_history (
	id INTEGER NOT NULL, 
	user_id INTEGER NOT NULL, 
	"query" VARCHAR NOT NULL, 
	normalized_query VARCHAR NOT NULL, 
	last_used_at DATETIME NOT NULL, 
	PRIMARY KEY (id), 
	CONSTRAINT uq_search_history_user_query UNIQUE (user_id, normalized_query), 
	FOREIGN KEY(user_id) REFERENCES users (id)
);
CREATE TABLE shelf (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    book_id INTEGER NOT NULL REFERENCES books(id),
    status VARCHAR NOT NULL,
    position INTEGER NOT NULL,
    updated_at DATETIME NOT NULL, shelf_position INTEGER, rating INTEGER, take VARCHAR DEFAULT '', dnf_reason VARCHAR DEFAULT '', progress INTEGER, started_at DATETIME, finished_at DATETIME,
    UNIQUE (user_id, book_id)
);
INSERT INTO "shelf" VALUES(1,1,1,'finished',0,'2024-01-02 00:00:00',NULL,NULL,'','',NULL,NULL,NULL);
INSERT INTO "shelf" VALUES(2,2,2,'did_not_finish',0,'2024-02-02 10:00:00',NULL,NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO "shelf" VALUES(3,2,1,'finished',0,'2024-02-03 11:00:00',NULL,4,NULL,NULL,NULL,NULL,NULL);
CREATE TABLE user_favorites (
	id INTEGER NOT NULL, 
	user_id INTEGER NOT NULL, 
	book_id INTEGER NOT NULL, 
	position INTEGER NOT NULL, 
	PRIMARY KEY (id), 
	CONSTRAINT uq_favorite_user_book UNIQUE (user_id, book_id), 
	CONSTRAINT uq_favorite_user_position UNIQUE (user_id, position), 
	CONSTRAINT ck_favorite_position CHECK (position >= 1 AND position <= 3), 
	FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE, 
	FOREIGN KEY(book_id) REFERENCES books (id) ON DELETE CASCADE
);
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username VARCHAR(32) NOT NULL UNIQUE,
    password_hash VARCHAR NOT NULL,
    created_at DATETIME NOT NULL
, theme VARCHAR(32) NOT NULL DEFAULT 'paper', color_mode VARCHAR(16) NOT NULL DEFAULT 'system', locale VARCHAR(8) NOT NULL DEFAULT 'en', notify_meeting BOOLEAN NOT NULL DEFAULT 1, notify_pick BOOLEAN NOT NULL DEFAULT 1, notify_note BOOLEAN NOT NULL DEFAULT 1, avatar BLOB, avatar_mime VARCHAR(32), avatar_updated_at DATETIME);
INSERT INTO "users" VALUES(1,'ada','x','2024-01-01 00:00:00','paper','system','en',1,1,1,NULL,NULL,NULL);
INSERT INTO "users" VALUES(2,'bea','$argon2id$v=19$m=65536,t=3,p=4$eNpQ0hjz4XDBl334aMDIDg$OWwxoO8q9V8SLqTzhxy4R5WPEPmw3rB+36vq4hPrdeM','2024-02-01 09:30:00','paper','system','en',1,1,1,NULL,NULL,NULL);
CREATE INDEX ix_book_post_reactions_post_id ON book_post_reactions (post_id);
CREATE INDEX ix_book_posts_author_id ON book_posts (author_id);
CREATE INDEX ix_book_posts_book_id ON book_posts (book_id);
CREATE INDEX ix_book_posts_created_at ON book_posts (created_at);
CREATE INDEX ix_book_posts_parent_id ON book_posts (parent_id);
CREATE INDEX ix_events_actor_id ON events (actor_id);
CREATE INDEX ix_events_created_at ON events (created_at);
CREATE INDEX ix_events_kind ON events (kind);
CREATE INDEX ix_next_up_ballots_nomination_id ON next_up_ballots (nomination_id);
CREATE INDEX ix_next_up_ballots_vote_id ON next_up_ballots (vote_id);
CREATE INDEX ix_next_up_nominations_vote_id ON next_up_nominations (vote_id);
CREATE INDEX ix_pick_milestones_pick_id ON pick_milestones (pick_id);
CREATE INDEX ix_post_reactions_post_id ON post_reactions (post_id);
CREATE INDEX ix_push_subscriptions_user_id ON push_subscriptions (user_id);
CREATE INDEX ix_quotes_book_id ON quotes (book_id);
CREATE INDEX ix_quotes_user_id ON quotes (user_id);
CREATE INDEX ix_search_history_user_id ON search_history (user_id);
CREATE INDEX ix_user_favorites_book_id ON user_favorites (book_id);
CREATE INDEX ix_user_favorites_user_id ON user_favorites (user_id);
CREATE UNIQUE INDEX ix_invites_code ON invites (code);
COMMIT;
