-- Row counts of the legacy SQLite database (#52, #38), independent of the importer.
-- deploy/cutover/compare.sh runs this file next to target-counts.sql; see its header.
--
--   sqlite3 -readonly <copy of bookclub.db> < legacy-counts.sql
--
-- Run it on a copy in a directory you can write: a database in WAL mode needs to create its
-- -wal and -shm files there, even read-only (compare.sh makes that copy).
--
-- Every output line is one value, |-separated:
--   count|<entity>|<rows>               the source column of the import report, by its entity
--                                       names (import-legacy reads the same rows)
--   same|<name>|<rows>                  rows import-legacy creates from the legacy rows without
--                                       a report line of their own: target-counts.sql must print
--                                       the same number
--   user|<id>|<username>|<what>|<rows>  per-user counts, the same names as target-counts.sql
--   hash|<id>|<username>|<password_hash> every account (compare.sh compares them and prints
--                                       only ids and usernames)
--
-- The v0.10.0 app creates every table at boot, so a database it has run has all of them.
-- server/tests/cutover_counts.rs runs this file on the importer fixtures (without the dot
-- commands); a query that no longer fits the legacy schema or the report fails there.

.bail on
.headers off
.mode list

-- The report's entity lines, in its order.
SELECT 'count|users|' || count(*) FROM users;
-- Users with an avatar blob: avatars --> media.
SELECT 'count|avatars|' || count(*) FROM users WHERE avatar IS NOT NULL AND length(avatar) > 0;
SELECT 'count|invites|' || count(*) FROM invites;
SELECT 'count|books|' || count(*) FROM books;
-- Books with an Open Library cover id (0 and negative ids mean "no cover").
SELECT 'count|covers|' || count(*) FROM books WHERE CAST(cover_id AS INTEGER) > 0;
SELECT 'count|shelf|' || count(*) FROM shelf;
SELECT 'count|physical_shelf_layouts|' || count(*) FROM physical_shelf_layouts;
SELECT 'count|user_favorites|' || count(*) FROM user_favorites;
SELECT 'count|club_picks|' || count(*) FROM club_picks;
SELECT 'count|pick_milestones|' || count(*) FROM pick_milestones;
SELECT 'count|book_posts|' || count(*) FROM book_posts;
SELECT 'count|club_pick_posts|' || count(*) FROM club_pick_posts;
SELECT 'count|book_post_reactions|' || count(*) FROM book_post_reactions;
SELECT 'count|post_reactions|' || count(*) FROM post_reactions;
SELECT 'count|quotes|' || count(*) FROM quotes;
SELECT 'count|next_up_votes|' || count(*) FROM next_up_votes;
SELECT 'count|next_up_nominations|' || count(*) FROM next_up_nominations;
SELECT 'count|next_up_ballots|' || count(*) FROM next_up_ballots;
SELECT 'count|events|' || count(*) FROM events;
SELECT 'count|search_history|' || count(*) FROM search_history;
SELECT 'count|push_subscriptions|' || count(*) FROM push_subscriptions;

-- What the target must hold besides the report's lines. Every user becomes a member of the
-- imported club, and every redeemed invite keeps its redemption.
SELECT 'same|club_memberships|' || count(*) FROM users;
SELECT 'same|club_invite_uses|' || count(*) FROM invites WHERE used_by_id IS NOT NULL;

-- Per user. A pick note that was mirrored into the diary (a book_posts row with its id in
-- legacy_post_id) is one post with its copy, so posts and reactions count the copy only.
SELECT 'user|' || u.id || '|' || u.username || '|shelf|'
       || (SELECT count(*) FROM shelf s WHERE s.user_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|favorites|'
       || (SELECT count(*) FROM user_favorites f WHERE f.user_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|quotes|'
       || (SELECT count(*) FROM quotes q WHERE q.user_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|posts|'
       || ((SELECT count(*) FROM book_posts p WHERE p.author_id = u.id)
           + (SELECT count(*) FROM club_pick_posts n
              WHERE n.author_id = u.id
                AND NOT EXISTS (SELECT 1 FROM book_posts p WHERE p.legacy_post_id = n.id)))
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|reactions|'
       || ((SELECT count(*) FROM book_post_reactions r WHERE r.user_id = u.id)
           + (SELECT count(*) FROM post_reactions r
              WHERE r.user_id = u.id
                AND NOT EXISTS (SELECT 1 FROM book_posts p WHERE p.legacy_post_id = r.post_id)))
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|searches|'
       || (SELECT count(*) FROM search_history h WHERE h.user_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|ballots|'
       || (SELECT count(*) FROM next_up_ballots b WHERE b.user_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|events|'
       || (SELECT count(*) FROM events e WHERE e.actor_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|layout|'
       || (SELECT count(*) FROM physical_shelf_layouts l WHERE l.user_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|avatar|'
       || (CASE WHEN u.avatar IS NOT NULL AND length(u.avatar) > 0 THEN 1 ELSE 0 END)
FROM users u ORDER BY u.id;
-- Every user is a member of the imported club.
SELECT 'user|' || u.id || '|' || u.username || '|membership|1' FROM users u ORDER BY u.id;

-- The accounts. import-legacy copies the hashes byte for byte; a login re-hashes them.
SELECT 'hash|' || id || '|' || username || '|' || password_hash FROM users ORDER BY id;
