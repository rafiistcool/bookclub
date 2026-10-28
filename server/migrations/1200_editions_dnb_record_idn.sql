-- HAR (#42), done by the integrator: a deleted DNB record withdraws every edition it delivered.
--
-- A DNB record with several ISBNs maps to one edition per ISBN (conventions §7.5), but
-- editions.dnb_idn is unique, so only the first ISBN's edition carries the record's IDN. A deleted
-- OAI header names only that IDN, and withdraw_dnb_edition used to withdraw only the first
-- edition: the record's other editions stayed live in search, book detail and new-releases.
--
-- dnb_record_idn is the IDN of the newest DNB record that delivered the edition, on every edition
-- of the record. It is not unique. Ingest writes it from EditionRecord.dnb_record_idn (map_record
-- sets it on each edition), else from EditionRecord.dnb_idn; the newest record's IDN wins, like
-- dnb_idn. withdraw_dnb_edition withdraws by it, so a deleted header withdraws exactly the
-- editions whose newest delivering record it names: an edition that a newer record delivered
-- again (a duplicate record, an ISBN that moved) stays live.
ALTER TABLE editions
    ADD COLUMN dnb_record_idn text COLLATE "C" NULL
        CHECK (char_length(dnb_record_idn) BETWEEN 1 AND 32);

-- Rows written before this migration: the edition that carries an IDN was delivered by it.
UPDATE editions SET dnb_record_idn = dnb_idn WHERE dnb_idn IS NOT NULL;

CREATE INDEX editions_dnb_record_idn_idx ON editions (dnb_record_idn)
    WHERE dnb_record_idn IS NOT NULL;

COMMENT ON COLUMN editions.dnb_record_idn IS
    'IDN (MARC 001) of the newest DNB record that delivered this edition, on every edition of a multi-ISBN record (dnb_idn is on the first only). Not unique. A deleted OAI header withdraws every edition with this IDN (ingest::withdraw_dnb_edition).';
COMMENT ON COLUMN editions.withdrawn_at IS
    'DNB delivered an OAI header with status="deleted" for the newest record that delivered this edition (dnb_record_idn): a withdrawn announcement, with all its ISBNs. The row stays (shelves may point at it) and still resolves by ISBN; new-releases skips it. A later live record with this edition (by ISBN or IDN) clears it.';
