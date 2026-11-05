-- Original titles (docs/phase3/notes/original-title.md): a translated edition's original title
-- from the DNB record (MARC 240 $a, CC0). It feeds works.search_extra, so "The name of the
-- wind" finds "Der Name des Windes", and the cover search, which asks Open Library by it when
-- the German title finds nothing.

ALTER TABLE editions
    ADD COLUMN original_title text NULL CHECK (char_length(original_title) BETWEEN 1 AND 1000),
    ADD COLUMN original_title_checked_at timestamptz NULL;

COMMENT ON COLUMN editions.original_title IS
    'The title of the work this edition translates, from the DNB record (MARC 240 $a). Written only by DNB ingest (authoritative: a DNB record without 240 clears it) and by the cover search''s one DNB check (ingest::record_original_title). Also feeds works.search_extra.';
COMMENT ON COLUMN editions.original_title_checked_at IS
    'When a DNB record last told original_title (ingest of a DNB record, or the cover search''s one DNB check by ISBN). NULL on a 978-3 edition = the cover search asks the DNB once.';
COMMENT ON COLUMN works.search_extra IS
    'Edition titles and original titles that differ from the work title (deduplicated by case fold, " / "-joined), maintained by ingest so a translated or retitled edition is found by its own title and by its original title.';

-- Books the search found nothing for wait 30 days or more. Those with a German-group ISBN are
-- due once now, so their DNB check and the request by the original title run on the next runs.
UPDATE cover_search cs SET next_search_at = now()
WHERE cs.outcome = 'no_match' AND cs.next_search_at > now()
  AND EXISTS (SELECT 1 FROM editions e WHERE e.work_id = cs.work_id AND e.isbn13 LIKE '9783%');
