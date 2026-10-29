-- Ends every access to the app that can be reused from a copy of its database: after a
-- compromise of the VM (README.md, "Restoring from a Storage Box snapshot"), before the app
-- starts on the restored database. Whoever had root on the VM could read the database, and
-- write it while they were in; the restored snapshot is from before the damage, not from
-- before the intrusion.
--
--   sessions      A session is the SHA-256 of its token, with no secret of the server's, so a
--                 row planted for a token the attacker holds signs them in on any server with
--                 this database. Every session ends; everyone signs in again with their
--                 password (cookie and app tokens alike).
--   club_invites  Club and sign-up invite codes are stored as they are, and a link invite
--                 admits anyone until it expires. Every open invite is revoked; club admins and
--                 instance admins create new ones.
--   email_tokens  Verification and reset links not used yet: a planted row works like a
--                 session. Every one counts as used; people ask for a new link.
--   friendships   A pending friend request becomes a friendship with one click by the account
--                 it is for, as an open invite becomes a membership: a planted request from
--                 every account to one the attacker keeps would make it everyone's audience.
--                 Every pending request goes (requests never expire, and the sender can ask
--                 again); accepted friendships stay and are reviewed by hand.
--
-- One transaction, all or nothing. From /opt/bookclub, with the file from the checkout the VM
-- was set up from:
--
--   docker compose exec -T postgres psql -X -v ON_ERROR_STOP=1 -U bookclub -d bookclub \
--       < deploy/hosted/revoke-access.sql
--
-- psql prints how many rows each statement changed. Admin flags, memberships, accepted
-- friendships (a planted one grants the same view as a planted membership) and addresses
-- are reviewed by hand afterwards (README.md). server/tests/auth_sessions.rs runs this file.
BEGIN;
DELETE FROM sessions;
UPDATE club_invites SET revoked_at = now() WHERE revoked_at IS NULL;
UPDATE email_tokens SET used_at = now() WHERE used_at IS NULL;
DELETE FROM friendships WHERE accepted_at IS NULL;
COMMIT;
