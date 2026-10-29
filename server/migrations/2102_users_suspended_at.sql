-- ADM2 (#55, Q15): an instance admin can suspend an account. A suspended account cannot sign
-- in (login and bearer token creation answer 403), every session and bearer token it had is
-- deleted when it is suspended, and a session that survives is refused where the server
-- resolves the caller (auth::extract). Its content stays. Unsuspending clears the column.
ALTER TABLE users ADD COLUMN suspended_at timestamptz NULL;

-- `GET /api/admin/users/suspended` lists them, newest first.
CREATE INDEX users_suspended_at_idx ON users (suspended_at) WHERE suspended_at IS NOT NULL;

COMMENT ON COLUMN users.suspended_at IS
    'When an instance admin suspended the account (POST /api/admin/users/{username}/suspend); NULL while it may sign in. The export carries it with the rest of the users row.';
