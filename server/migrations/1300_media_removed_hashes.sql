-- MED (#43): the hashes a taken-down picture must never come back under.
--
-- media.sha256 is the hash of the bytes as received: the original upload or Open Library's JPEG.
-- Readers never get those bytes; they see and save the WebP variants served at
-- /media/{public_id}/{variant}. A takedown therefore records the hash of every stored variant,
-- and the original's once more, so the record also outlives the media row (account deletion
-- deletes a user's avatar rows, taken-down ones included). An upload whose SHA-256 is listed here
-- or matches a removed media row is refused with 400 "This picture was removed".
CREATE TABLE media_removed_hashes (
    sha256     bytea PRIMARY KEY CHECK (octet_length(sha256) = 32),
    media_id   bigint NULL REFERENCES media (id) ON DELETE SET NULL,
    variant    text NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX media_removed_hashes_media_id_idx ON media_removed_hashes (media_id)
    WHERE media_id IS NOT NULL;

COMMENT ON TABLE media_removed_hashes IS
    'SHA-256 of every byte sequence a taken-down picture was received or served as (MED takedown). Uploads with a listed hash are refused.';
COMMENT ON COLUMN media_removed_hashes.media_id IS
    'The taken-down picture; NULL once its row was deleted (the hash stays).';
COMMENT ON COLUMN media_removed_hashes.variant IS
    'The stored variant (l, s or a) whose bytes have this hash; NULL for the bytes as received (media.sha256).';
