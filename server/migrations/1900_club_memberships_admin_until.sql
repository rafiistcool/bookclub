-- Integrator, security review 2026-10-28 #9: a removal revokes the invites the removed person
-- could list. An admin lists every invite of the club; a demotion made them a plain member
-- again but left the codes in their head, so a removal after a demotion revoked too little.
ALTER TABLE club_memberships ADD COLUMN admin_until timestamptz NULL;

COMMENT ON COLUMN club_memberships.admin_until IS
    'When this member last stopped being an admin (the demotion, clock_timestamp() under the club lock); NULL if never. Their removal revokes every active invite of the club created up to then, as for a current admin all of them. Gone with the row: a member who left and came back starts over.';
