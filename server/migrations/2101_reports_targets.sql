-- ADM2 (#54, Q11 option B): reports on profile pictures and on works and editions readers
-- added, beside covers: everything people outside a club can see. docs/phase2/notes/adm2.md
-- says what "remove" does to each kind.
--
-- The target of a report, by kind:
--   cover    media_id (the picture), work_id and edition_id (the book that showed it)
--   avatar   media_id (the picture), target_user_id (whose profile picture it was when reported:
--            the picture row goes when its owner replaces it, the report keeps the owner)
--   work     work_id (a work with source 'user')
--   edition  edition_id (an edition with source 'user') and work_id (its work)
-- Every target column is ON DELETE SET NULL, so a report outlives what it named.
ALTER TABLE reports DROP CONSTRAINT reports_target_kind_check;
ALTER TABLE reports
    ADD CONSTRAINT reports_target_kind_check
        CHECK (target_kind IN ('cover', 'avatar', 'work', 'edition')),
    ADD COLUMN target_user_id bigint NULL REFERENCES users (id) ON DELETE SET NULL,
    ADD CONSTRAINT reports_media_targets CHECK (target_kind IN ('cover', 'avatar') OR media_id IS NULL),
    ADD CONSTRAINT reports_target_user_avatars CHECK (target_kind = 'avatar' OR target_user_id IS NULL);

-- How a report came in: `app` (a reader's report) or `notice` (an emailed notice an admin
-- recorded with a direct takedown, D2). Before, a notice was told from a report of a reader who
-- deleted the account by nothing; both have no `created_by`.
ALTER TABLE reports
    ADD COLUMN via text NOT NULL DEFAULT 'app' CHECK (via IN ('app', 'notice'));

-- The report or notice arrived when its picture was down already, so it was closed at once and
-- no admin decided it (docs/phase2/notes/adm.md, "Closed on arrival"). Before, three columns
-- told it (status 'removed', no resolved_by, resolved_at = created_at), which a decision whose
-- admin later deleted the account could match too.
ALTER TABLE reports
    ADD COLUMN closed_on_arrival boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT reports_closed_on_arrival_removed CHECK (NOT closed_on_arrival OR status = 'removed');

-- Rows written before this migration, by the old rules. A notice was the only row written with
-- no reporter and an admin's decision in one go; a reader's report of an account deleted since
-- is classed as a notice too (no production data predates this migration).
UPDATE reports SET closed_on_arrival = true
    WHERE status = 'removed' AND resolved_by IS NULL AND resolved_at = created_at;
UPDATE reports SET via = 'notice' WHERE created_by IS NULL AND status = 'removed';

-- The user deletion's ON DELETE SET NULL looks the reports up by this column.
CREATE INDEX reports_target_user_id_idx ON reports (target_user_id) WHERE target_user_id IS NOT NULL;
-- "Every open report of this work (edition)": a decision closes them together.
CREATE INDEX reports_open_work_idx ON reports (work_id) WHERE status = 'open' AND target_kind = 'work';
CREATE INDEX reports_open_edition_idx ON reports (edition_id) WHERE status = 'open' AND target_kind = 'edition';

COMMENT ON COLUMN reports.target_kind IS
    'What was reported: cover (media_id, and the book that showed it), avatar (media_id, target_user_id), work (work_id, a user-added work) or edition (edition_id and its work_id, a user-added edition).';
COMMENT ON COLUMN reports.target_user_id IS
    'Avatar reports only: the account whose profile picture was reported, when it was reported. Exported to that account without the reporter (reports.about_me).';
COMMENT ON COLUMN reports.via IS
    'app: a signed-in reader filed it; notice: an admin recorded an emailed notice with a direct takedown (created_by is NULL).';
COMMENT ON COLUMN reports.closed_on_arrival IS
    'The picture was taken down before the report or notice arrived: status removed, no admin decided it, and the response-time statistics leave it out.';
