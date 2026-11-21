-- TEXTS (#55, Q11 option C): reports on what readers write, beside the pictures and books of
-- 2101: posts (club posts, replies and diary entries, one kind), quotes, and the take on a shelf
-- entry. docs/phase3/notes/texts.md says what "remove" does to each kind.
--
-- The target of a text report, by kind:
--   post   post_id (the post as reported), target_user_id (its author) and work_id (its book)
--   quote  quote_id, target_user_id (whose quote) and work_id
--   take   shelf_entry_id (the entry whose take it is), target_user_id (whose entry) and work_id
-- The new target columns are ON DELETE SET NULL like the others, so a report outlives the text
-- (its author deleting it, or the account going).
--
-- text_snapshot holds the text as it was when the report was filed, for the instance admin only:
-- the author may edit it since. Bounded by the longest text kind (posts and quotes, 1000
-- characters; a take has 140). The author's account deletion clears it
-- (auth::deletion::delete_user), and the 3-year retention of resolved reports deletes the row.
ALTER TABLE reports DROP CONSTRAINT reports_target_kind_check;
ALTER TABLE reports DROP CONSTRAINT reports_target_user_avatars;
ALTER TABLE reports
    ADD CONSTRAINT reports_target_kind_check
        CHECK (target_kind IN ('cover', 'avatar', 'work', 'edition', 'post', 'quote', 'take')),
    ADD COLUMN post_id bigint NULL REFERENCES posts (id) ON DELETE SET NULL,
    ADD COLUMN quote_id bigint NULL REFERENCES quotes (id) ON DELETE SET NULL,
    ADD COLUMN shelf_entry_id bigint NULL REFERENCES shelf_entries (id) ON DELETE SET NULL,
    ADD COLUMN text_snapshot text NULL CHECK (char_length(text_snapshot) <= 1000),
    ADD CONSTRAINT reports_target_user_kinds
        CHECK (target_kind IN ('avatar', 'post', 'quote', 'take') OR target_user_id IS NULL),
    ADD CONSTRAINT reports_post_targets CHECK (target_kind = 'post' OR post_id IS NULL),
    ADD CONSTRAINT reports_quote_targets CHECK (target_kind = 'quote' OR quote_id IS NULL),
    ADD CONSTRAINT reports_take_targets CHECK (target_kind = 'take' OR shelf_entry_id IS NULL),
    ADD CONSTRAINT reports_text_snapshot_texts
        CHECK (target_kind IN ('post', 'quote', 'take') OR text_snapshot IS NULL);

-- The ON DELETE SET NULL of a post, quote or shelf entry looks the reports up by these columns,
-- and a decision closes every open report of the same text.
CREATE INDEX reports_post_id_idx ON reports (post_id) WHERE post_id IS NOT NULL;
CREATE INDEX reports_quote_id_idx ON reports (quote_id) WHERE quote_id IS NOT NULL;
CREATE INDEX reports_shelf_entry_id_idx ON reports (shelf_entry_id) WHERE shelf_entry_id IS NOT NULL;

COMMENT ON COLUMN reports.target_kind IS
    'What was reported: cover (media_id, and the book that showed it), avatar (media_id, target_user_id), work (target_work_id, a user-added work, and work_id), edition (edition_id and its work_id, a user-added edition), post (post_id, target_user_id the author, work_id), quote (quote_id, target_user_id, work_id) or take (shelf_entry_id, target_user_id, work_id).';
COMMENT ON COLUMN reports.target_user_id IS
    'Avatar reports: the account whose profile picture was reported. Post, quote and take reports: the author of the text. As when reported. Exported to that account without the reporter (reports.about_me).';
COMMENT ON COLUMN reports.text_snapshot IS
    'Post, quote and take reports: the text as it was when the report was filed, shown to instance admins only. Never exported to the reporter; cleared when the author deletes the account.';
