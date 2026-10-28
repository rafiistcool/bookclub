-- ADM (#54): the state of the admin notification job (jobs::lock_ids::ADMIN_NOTIFY,
-- server/src/admin/notify.rs). One row per queue: the newest item the last mail covered and
-- when that mail went out. Open reports are mailed at most hourly, open catalog requests as a
-- daily digest, each only when items arrived after `last_item_id`.
--
-- No foreign key to users: the mails go to every instance admin with a verified address, and
-- nothing here names an account, so the export (conventions §5.7) has nothing to cover.
CREATE TABLE admin_notify_state (
    queue        text COLLATE "C" PRIMARY KEY CHECK (queue IN ('reports', 'edit_requests')),
    last_item_id bigint NOT NULL DEFAULT 0 CHECK (last_item_id >= 0),
    sent_at      timestamptz NULL
);

INSERT INTO admin_notify_state (queue) VALUES ('reports'), ('edit_requests');

COMMENT ON TABLE admin_notify_state IS
    'Admin notification job (#54): per queue, the newest item id the last mail covered and when it was sent.';
COMMENT ON COLUMN admin_notify_state.last_item_id IS
    'reports.id or catalog_edit_requests.id of the newest open item the last mail counted; 0 before the first mail. A mail goes out only for open items with a larger id.';
COMMENT ON COLUMN admin_notify_state.sent_at IS
    'When the last mail of this queue went out; the next waits 1 hour (reports) or 24 hours (edit_requests). NULL before the first.';
