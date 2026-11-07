-- A personal cover choice (docs/phase3/notes/cover-choice.md): a reader picks the cover a book on
-- their shelf shows, for them and for everyone who sees their shelf. The choice is a picture the
-- catalog already knows (a work's or edition's cover, an Open Library edition cover, a publisher
-- cover from the DNB portal), or a photo of the reader's own copy, which stays theirs: it never
-- becomes a catalog cover (ingest::set_cover refuses it).

ALTER TABLE shelf_entries
    ADD COLUMN cover_media_id bigint NULL REFERENCES media (id) ON DELETE SET NULL;
-- The takedown's and the gone step's clearing (ingest::clear_references), the photo sweeper.
CREATE INDEX shelf_entries_cover_media_id_idx ON shelf_entries (cover_media_id)
    WHERE cover_media_id IS NOT NULL;

ALTER TABLE media
    ADD COLUMN personal boolean NOT NULL DEFAULT false,
    ADD COLUMN unused_since timestamptz NULL;
ALTER TABLE media ADD CONSTRAINT media_personal_upload
    CHECK (NOT personal OR (kind = 'cover' AND source = 'upload'));
ALTER TABLE media ADD CONSTRAINT media_unused_personal
    CHECK (unused_since IS NULL OR personal);
-- The hourly sweeper's scan: live personal photos.
CREATE INDEX media_personal_idx ON media (id) WHERE personal AND removed_at IS NULL;

COMMENT ON COLUMN shelf_entries.cover_media_id IS
    'The cover the reader chose for this book (cover-choice): it wins over the edition''s and the work''s wherever the entry''s book shows (the shelf, Home, the book page for the reader, friends'' and club mates'' views of the shelf). NULL = the standard cover. A takedown or a gone Open Library picture clears it (ingest::clear_references).';
COMMENT ON COLUMN media.personal IS
    'A photo a reader uploaded as the cover of a book on their own shelf (PUT /api/shelf/{entry_id}/cover/photo): never a catalog cover (ingest::set_cover refuses it). The hourly sweeper deletes it 7 days after no shelf entry chooses it any more, account deletion with the account.';
COMMENT ON COLUMN media.unused_since IS
    'Personal photos only: since when no shelf entry chooses it, as the hourly sweeper saw it (auth::sweep_personal_covers). It deletes the photo 7 days later; a choice of it again clears this.';
