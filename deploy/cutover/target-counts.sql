-- Row counts of the imported Postgres database (#52, #38), independent of the importer.
-- deploy/cutover/compare.sh runs this file next to legacy-counts.sql; see its header.
--
--   psql "$DATABASE_URL" -f target-counts.sql
--   docker compose exec -T postgres psql -U bookclub -d bookclub < target-counts.sql
--
-- Run it right after import-legacy, before anyone signs in: the counts describe a fresh target
-- that holds only the import, and a login re-hashes a legacy password hash.
--
-- Every output line is one value, |-separated:
--   count|<entity>|<rows>               the target column of the import report, by its entity
--                                       names; a+b is the sum of a's and b's target columns
--                                       (both land in one table)
--   user|<id>|<username>|<what>|<rows>  per-user counts, the same names as legacy-counts.sql
--   hash|<id>|<username>|<password_hash> every account (compare.sh compares them and prints
--                                       only ids and usernames)
--
-- The report's covers line has no count here: the cover media of the works a lookup matched
-- are in the same table, so no query separates the legacy cover ids.
-- server/tests/cutover_counts.rs runs this file after importing the fixtures (without the
-- backslash commands); a query that no longer fits the schema or the report fails there.

\set QUIET on
\set ON_ERROR_STOP on
\pset format unaligned
\pset tuples_only on
\pset pager off

-- The report's entity lines, in its order.
SELECT 'count|users|' || count(*) FROM users;
SELECT 'count|avatars|' || count(*) FROM users WHERE avatar_media_id IS NOT NULL;
SELECT 'count|club|' || count(*) FROM clubs;
SELECT 'count|invites|' || count(*) FROM club_invites;
-- Legacy books map onto works; merged books share one.
SELECT 'count|books|' || count(DISTINCT work_id) FROM legacy_work_keys;
-- The editions the legacy ISBN keys identified.
SELECT 'count|editions|' || count(DISTINCT edition_id) FROM legacy_work_keys;
SELECT 'count|shelf|' || count(*) FROM shelf_entries;
SELECT 'count|physical_shelf_layouts|' || count(*) FROM physical_shelf_layouts;
SELECT 'count|user_favorites|' || count(*) FROM user_favorites;
SELECT 'count|club_picks|' || count(*) FROM club_picks;
SELECT 'count|pick_milestones|' || count(*) FROM milestones;
-- Diary entries, their club copies and the new pick notes are all posts.
SELECT 'count|book_posts+club_pick_posts|' || count(*) FROM posts;
SELECT 'count|book_post_reactions+post_reactions|' || count(*) FROM post_reactions;
SELECT 'count|quotes|' || count(*) FROM quotes;
SELECT 'count|next_up_votes|' || count(*) FROM next_up_votes;
SELECT 'count|next_up_nominations|' || count(*) FROM next_up_nominations;
SELECT 'count|next_up_ballots|' || count(*) FROM next_up_ballots;
SELECT 'count|events|' || count(*) FROM events;
SELECT 'count|search_history|' || count(*) FROM search_history;

-- Per user.
SELECT 'user|' || u.id || '|' || u.username || '|shelf|'
       || (SELECT count(*) FROM shelf_entries s WHERE s.user_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|favorites|'
       || (SELECT count(*) FROM user_favorites f WHERE f.user_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|quotes|'
       || (SELECT count(*) FROM quotes q WHERE q.user_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|posts|'
       || (SELECT count(*) FROM posts p WHERE p.author_id = u.id)
FROM users u ORDER BY u.id;
SELECT 'user|' || u.id || '|' || u.username || '|reactions|'
       || (SELECT count(*) FROM post_reactions r WHERE r.user_id = u.id)
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
       || (CASE WHEN u.avatar_media_id IS NOT NULL THEN 1 ELSE 0 END)
FROM users u ORDER BY u.id;

-- The accounts.
SELECT 'hash|' || id || '|' || username || '|' || password_hash FROM users ORDER BY id;
