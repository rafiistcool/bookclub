-- LEG2 (#53, Q14 A): the purge of empty sign-ups deletes an account only when it shows no
-- sign-in for 30 days (auth::empty_sign_ups). A live session keeps its last use in
-- sessions.last_used_at, but a logout, the revocation of a session, a password change or reset
-- and a suspension delete the session row before it expires, and the evidence of the sign-in
-- went with it. This column keeps the last use of such a session.
--
-- The trigger fires only for a session deleted before it expired. An expired session was last
-- used more than 30 days ago (a cookie session expires 30 days after its last use, an app token
-- 90 days), so the hourly cleanup of expired rows writes nothing here.
--
-- Its UPDATE skips a users row that another transaction holds instead of waiting for it. A
-- logout holds its session row and would then wait for the users row, while a password change,
-- a reset, a suspension, an account deletion or the purge hold the users row and then delete or
-- lock the sessions: waiting could deadlock. Whoever holds the row writes a newer trace of its
-- own (password_changed_at, a new session), deletes the account, or is the purge, which locks
-- the sessions after the users row. A transaction that holds the users row itself (a password
-- change deleting the other sessions) is not skipped. When the account itself is deleted
-- (ON DELETE CASCADE), the UPDATE finds no row.
ALTER TABLE users ADD COLUMN last_seen_at timestamptz NULL;

CREATE FUNCTION sessions_keep_last_use() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE users SET last_seen_at = OLD.last_used_at
    WHERE id = (SELECT u.id FROM users u
                WHERE u.id = OLD.user_id
                  AND (u.last_seen_at IS NULL OR u.last_seen_at < OLD.last_used_at)
                FOR NO KEY UPDATE SKIP LOCKED);
    RETURN NULL;
END
$$;

CREATE TRIGGER sessions_keep_last_use
    AFTER DELETE ON sessions
    FOR EACH ROW
    WHEN (OLD.expires_at > now())
    EXECUTE FUNCTION sessions_keep_last_use();

COMMENT ON COLUMN users.last_seen_at IS
    'The last use (sessions.last_used_at) of the newest session of the account that ended before it expired: a logout, a revoked session, a password change or reset, a suspension. Written only by the trigger sessions_keep_last_use; live sessions keep their own last use. NULL until such a session ends. The purge of empty sign-ups (auth::empty_sign_ups) counts it as a sign-in; the export carries it with the rest of the users row.';
