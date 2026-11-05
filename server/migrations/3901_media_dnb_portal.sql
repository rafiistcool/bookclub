-- Publisher covers from the DNB portal (docs/phase3/notes/publisher-covers.md): with
-- DNB_PORTAL_COVERS=true the cover search fetches the cover the DNB portal shows for a
-- German-group ISBN (the publishers' covers from the VLB) and stores it as its own media row.
-- The source keeps those rows apart, so the public launch removes them in one statement.

ALTER TABLE media DROP CONSTRAINT media_source_check;
ALTER TABLE media ADD CONSTRAINT media_source_check
    CHECK (source IN ('upload', 'openlibrary', 'legacy', 'dnb_portal'));

COMMENT ON COLUMN media.source IS
    'upload (a reader''s or an admin''s picture), openlibrary (fetched by ol_cover_id when needed), legacy (the importer), dnb_portal (a publisher cover the cover search fetched from the DNB portal; private instance only, removed before the public launch).';

-- Books the search found nothing for wait 30 days or more. Those with a German-group ISBN are
-- due once now, so an instance that turns the portal on asks it on the next runs (without the
-- setting the search asks Open Library once more and changes nothing).
UPDATE cover_search cs SET next_search_at = now()
WHERE cs.outcome = 'no_match' AND cs.next_search_at > now()
  AND EXISTS (SELECT 1 FROM editions e WHERE e.work_id = cs.work_id AND e.isbn13 LIKE '9783%');
