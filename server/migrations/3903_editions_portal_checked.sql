-- The cover choice's DNB portal step (docs/phase3/notes/cover-choice.md): when the candidate
-- list asked the portal for an edition's publisher cover and it had none, the edition is not
-- asked again for a while (choice::PORTAL_RECHECK_DAYS), so opening the chooser again does not
-- send the same requests to the portal each time.

ALTER TABLE editions ADD COLUMN portal_checked_at timestamptz NULL;

COMMENT ON COLUMN editions.portal_checked_at IS
    'When the cover choice''s candidate list last asked the DNB portal for this edition''s publisher cover and got none (DNB_PORTAL_COVERS, private instance only). The list asks again only after choice::PORTAL_RECHECK_DAYS. NULL = never asked, or the portal had no answer.';
