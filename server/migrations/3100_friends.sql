-- FRIENDS (#57, docs/phase3/work-plan.md §4): friendships, blocks and friend invites into clubs.
--
-- A friend is audience (crate::visibility::Relation::Audience): friendships join the audience
-- through bc_can_see_user and bc_visible_owners, as 0001_init.sql's visibility section planned,
-- so every read that goes through the policy functions (shelves, favourites, layouts, quotes,
-- stats, diaries, the book page's readers, reaction names, avatar report rights, `?username=`)
-- shows friends what it shows co-members. bc_friends lists the viewer's friends for the other
-- callers. The home feed (activity/feed.rs, conventions §6.4) is the one other place,
-- deliberately: its personal events are joined by club membership (from joined_at), so it gains
-- a friend branch of its own (from accepted_at), inside its personal-event group, so a friend's
-- club events never reach a viewer outside that club. Club data (overlap, suggestions, pick
-- readers, club stats, the members' previews, the club feed and club posts) stays club-only:
-- it joins club_memberships.

-- One row per pair. accepted_at NULL = a pending request from requester to addressee.
CREATE TABLE friendships (
    requester_id bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    addressee_id bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   timestamptz NOT NULL DEFAULT now(),
    accepted_at  timestamptz NULL,
    PRIMARY KEY (requester_id, addressee_id),
    CONSTRAINT friendships_not_self CHECK (requester_id <> addressee_id),
    CONSTRAINT friendships_accepted_after CHECK (accepted_at IS NULL OR accepted_at >= created_at)
);
CREATE UNIQUE INDEX friendships_pair_key
    ON friendships ((least(requester_id, addressee_id)), (greatest(requester_id, addressee_id)));
CREATE INDEX friendships_addressee_idx ON friendships (addressee_id, requester_id);

COMMENT ON TABLE friendships IS
    'One row per pair of accounts (friendships_pair_key): a pending request from requester_id to addressee_id while accepted_at is NULL, a friendship from accepted_at on. A declined or cancelled request, an unfriend and a block delete the row; a request never expires. Friends are each other''s audience (bc_can_see_user, bc_visible_owners, bc_friends); a pending request grants nothing. Exported to both accounts (key friends); a friend list is never served to anyone else.';
COMMENT ON COLUMN friendships.accepted_at IS
    'When the addressee accepted (or the requester sent a request back); NULL while pending. The home feed shows a friend''s personal events created at or after it.';

-- user_id blocked blocked_user_id. Independent of friendships: a block deletes the pair's row.
CREATE TABLE user_blocks (
    user_id         bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    blocked_user_id bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, blocked_user_id),
    CONSTRAINT user_blocks_not_self CHECK (user_id <> blocked_user_id)
);
CREATE INDEX user_blocks_blocked_idx ON user_blocks (blocked_user_id);

COMMENT ON TABLE user_blocks IS
    'user_id blocked blocked_user_id: silent, never told to the blocked account. A block deletes the pair''s friendships row and revokes the pair''s open bound invites; requests the blocked account sends afterwards are stored and hidden from user_id. It hides nothing in clubs the two share. Exported to user_id (key blocks), never to blocked_user_id (DSGVO Art. 15 (4)).';

-- A friend invite: a single-use club invite only this account can redeem.
ALTER TABLE club_invites ADD COLUMN invitee_id bigint NULL REFERENCES users (id) ON DELETE CASCADE;
ALTER TABLE club_invites ADD CONSTRAINT club_invites_bound
    CHECK (invitee_id IS NULL OR (club_id IS NOT NULL AND max_uses = 1));
CREATE INDEX club_invites_invitee_idx ON club_invites (invitee_id) WHERE invitee_id IS NOT NULL;

COMMENT ON COLUMN club_invites.invitee_id IS
    'A friend invite (kind friend): the one account that may redeem it (clubs::invites::consume adds AND (invitee_id IS NULL OR invitee_id = <the redeeming account>)), so a forwarded code is useless and a bound code never signs anyone up. ON DELETE CASCADE, not SET NULL: NULL would turn it into an open single-use code. Revoked when the invitee declines, when the pair unfriends or blocks, and when the invitee is removed from the club.';

------------------------------------------------------------------------------
-- Visibility policy: friends join the audience. CREATE OR REPLACE with the same names,
-- parameter names and OUT columns, never DROP: bc_visible_shelf, bc_visible_quotes,
-- bc_visible_posts and bc_can_see_item depend on them. Every friend branch filters on
-- accepted_at IS NOT NULL, so a pending request grants nothing. The set-returning functions
-- stay single SQL statements, so the planner still inlines them
-- (visibility_policy.rs::policy_functions_still_inline).
------------------------------------------------------------------------------

-- The viewer's friends, with the time the friendship began (accepted_at). For lists of friends
-- and for the home feed's friend branch; bc_visible_owners reads it too.
CREATE FUNCTION bc_friends(viewer bigint) RETURNS TABLE (user_id bigint, since timestamptz)
    LANGUAGE sql STABLE PARALLEL SAFE
    BEGIN ATOMIC
        SELECT f.addressee_id, f.accepted_at FROM friendships AS f
        WHERE f.requester_id = viewer AND f.accepted_at IS NOT NULL
        UNION ALL
        SELECT f.requester_id, f.accepted_at FROM friendships AS f
        WHERE f.addressee_id = viewer AND f.accepted_at IS NOT NULL;
    END;

-- True when `viewer` may see `owner`'s personal data: same user, the two share at least one
-- club, or they are friends. Never NULL: an unknown or NULL owner (a deleted account) gives
-- false. The friend branch uses the explicit pair form, never least/greatest, which ignore
-- NULLs.
CREATE OR REPLACE FUNCTION bc_can_see_user(viewer bigint, owner bigint) RETURNS boolean
    LANGUAGE sql STABLE PARALLEL SAFE
    RETURN coalesce(viewer = owner, false) OR EXISTS (
        SELECT 1 FROM club_memberships AS a
        JOIN club_memberships AS b ON b.club_id = a.club_id
        WHERE a.user_id = viewer AND b.user_id = owner
    ) OR EXISTS (
        SELECT 1 FROM friendships AS f
        WHERE f.accepted_at IS NOT NULL
          AND ((f.requester_id = viewer AND f.addressee_id = owner)
               OR (f.requester_id = owner AND f.addressee_id = viewer))
    );

-- Everyone whose personal data `viewer` may see, INCLUDING the viewer (is_self = true): the
-- co-members of the viewer's clubs and the viewer's friends. The DISTINCT covers both sources,
-- so a friend who is also a co-member appears once (reader lists, rating means, counts and
-- reaction names do not double). Lists of "other readers" add WHERE NOT is_self.
CREATE OR REPLACE FUNCTION bc_visible_owners(viewer bigint) RETURNS TABLE (user_id bigint, is_self boolean)
    LANGUAGE sql STABLE PARALLEL SAFE
    BEGIN ATOMIC
        SELECT viewer, true
        UNION ALL
        SELECT DISTINCT o.user_id, false
        FROM (SELECT b.user_id FROM club_memberships AS a
              JOIN club_memberships AS b ON b.club_id = a.club_id
              WHERE a.user_id = viewer
              UNION ALL
              SELECT fr.user_id FROM bc_friends(viewer) AS fr) AS o
        WHERE o.user_id <> viewer;
    END;
