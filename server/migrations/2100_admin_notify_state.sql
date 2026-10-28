-- ADM (#54): the state of the admin notification job (jobs::lock_ids::ADMIN_NOTIFY,
-- server/src/admin/notify.rs). Each report and catalog request records when a mail to the
-- instance admins first counted it. A run mails a queue's open items that no mail has counted
-- yet, when the queue's newest `notified_at` (its last mail) is at least 1 hour (reports) or
-- 24 hours (catalog requests) old, and then stamps them. An item that commits after a newer id
-- is still unstamped on the next run.
--
-- The state lives on the items, not in a table of its own: a new table also needs a class in
-- server/tests/parity_schema.rs and docs/phase1/parity-checklist.md (docs/phase2/notes/adm.md).
-- No foreign key to users, so the export check (conventions §5.7) has nothing to cover.
ALTER TABLE reports ADD COLUMN notified_at timestamptz NULL;
ALTER TABLE catalog_edit_requests ADD COLUMN notified_at timestamptz NULL;

-- The job asks whether any item was stamped within the queue's interval (its last mail).
CREATE INDEX reports_notified_at_idx ON reports (notified_at) WHERE notified_at IS NOT NULL;
CREATE INDEX catalog_edit_requests_notified_at_idx ON catalog_edit_requests (notified_at)
    WHERE notified_at IS NOT NULL;

COMMENT ON COLUMN reports.notified_at IS
    'When the admin notification job (#54) first mailed the instance admins about this open report. NULL until then, and for a report resolved before a mail counted it. The newest value is the last report mail; the next waits 1 hour.';
COMMENT ON COLUMN catalog_edit_requests.notified_at IS
    'When the admin notification job (#54) first counted this open request in a digest to the instance admins. NULL until then, and for a request decided before a digest counted it. The newest value is the last digest; the next waits 24 hours.';
