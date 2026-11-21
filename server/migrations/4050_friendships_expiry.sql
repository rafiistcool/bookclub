-- REQS (#55, docs/phase3/notes/reqs.md): before open sign-up, pending friend requests expire after
-- 30 days (friends::requests::expire_requests, in the hourly cleanup, auth::cleanup), an account
-- has at most 50 pending requests out at once (friends::requests::MAX_PENDING_OUT), and instance
-- admins see the accounts whose requests pile up or get blocked
-- (GET /api/admin/users/friend-requests).

-- The expiry's search: the pending rows by age. Accepted rows, the bulk of the table, are left out.
CREATE INDEX friendships_pending_idx ON friendships (created_at) WHERE accepted_at IS NULL;

-- The admin list counts the blocks each account received in the last 30 days.
CREATE INDEX user_blocks_created_idx ON user_blocks (created_at);

COMMENT ON TABLE friendships IS
    'One row per pair of accounts (friendships_pair_key): a pending request from requester_id to addressee_id while accepted_at is NULL, a friendship from accepted_at on. A declined or cancelled request, an unfriend and a block delete the row; a pending request (hidden ones too) is deleted by the hourly cleanup 30 days after created_at, and an account has at most 50 pending requests out at once (REQS). Friends are each other''s audience (bc_can_see_user, bc_visible_owners, bc_friends); a pending request grants nothing. Exported to both accounts (key friends); a friend list is never served to anyone else.';
