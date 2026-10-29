-- The integrator, after ADM2 (#55) met LEG2's purge (#53, Q14): lifting a suspension counts as a
-- sign-in for the purge of empty sign-ups (auth::empty_sign_ups). A suspended account cannot sign
-- in, and its suspension deleted its sessions, so its last sign-in (created_at,
-- password_changed_at, terms_accepted_at, last_seen_at) stays where it was. Without this column,
-- an empty sign-up whose suspension an admin lifts more than 30 days after that sign-in went at
-- the next hourly run, before its owner could sign in again, and its username was free.
ALTER TABLE users ADD COLUMN unsuspended_at timestamptz NULL;

COMMENT ON COLUMN users.unsuspended_at IS
    'When an instance admin last lifted a suspension of the account (POST /api/admin/users/{username}/unsuspend); NULL while none was lifted. The purge of empty sign-ups (auth::empty_sign_ups) counts it as a sign-in. The export carries it with the rest of the users row.';
