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

-- Rows written before this migration. The edition that carries an IDN was delivered by it.
UPDATE editions SET dnb_record_idn = dnb_idn WHERE dnb_idn IS NOT NULL;

-- The other ISBNs of a multi-ISBN record were stored without an IDN, with the record's planned
-- date (263). Until this migration, new-releases recognised them by that date (HAR decision 15):
-- an edition without an IDN whose planned date came from the DNB and equals that of a withdrawn
-- edition of its work was left out. By the same heuristic, such an edition gets the record IDN of
-- the edition of its work that carries an IDN and has the same planned date, when exactly one
-- does. With several, its record is unknown, and the record's next delivery sets it.
UPDATE editions e
   SET dnb_record_idn = src.idn
  FROM (SELECT work_id, planned_publication_date, min(dnb_idn) AS idn
          FROM editions
         WHERE dnb_idn IS NOT NULL AND planned_publication_date IS NOT NULL
         GROUP BY work_id, planned_publication_date
        HAVING count(*) = 1) src
 WHERE e.dnb_idn IS NULL
   AND e.dnb_record_idn IS NULL
   AND e.field_sources->>'planned_publication_date' = 'dnb'
   AND e.work_id = src.work_id
   AND e.planned_publication_date = src.planned_publication_date;

-- The editions that new-releases left out that way, the other ISBNs of a record deleted before
-- this migration, are withdrawn here: new-releases now reads only withdrawn_at, so it lists the
-- same editions after this migration as before it. A later delivery of the record brings them
-- back, as it does any withdrawn edition.
UPDATE editions e
   SET withdrawn_at = gone.withdrawn_at, updated_at = now()
  FROM (SELECT work_id, planned_publication_date, max(withdrawn_at) AS withdrawn_at
          FROM editions
         WHERE withdrawn_at IS NOT NULL AND planned_publication_date IS NOT NULL
         GROUP BY work_id, planned_publication_date) gone
 WHERE e.dnb_idn IS NULL
   AND e.withdrawn_at IS NULL
   AND e.field_sources->>'planned_publication_date' = 'dnb'
   AND e.work_id = gone.work_id
   AND e.planned_publication_date = gone.planned_publication_date;

CREATE INDEX editions_dnb_record_idn_idx ON editions (dnb_record_idn)
    WHERE dnb_record_idn IS NOT NULL;

COMMENT ON COLUMN editions.dnb_record_idn IS
    'IDN (MARC 001) of the newest DNB record that delivered this edition, on every edition of a multi-ISBN record (dnb_idn is on the first only). Not unique. A deleted OAI header withdraws every edition with this IDN (ingest::withdraw_dnb_edition).';
COMMENT ON COLUMN editions.withdrawn_at IS
    'DNB delivered an OAI header with status="deleted" for the newest record that delivered this edition (dnb_record_idn): a withdrawn announcement, with all its ISBNs. The row stays (shelves may point at it) and still resolves by ISBN; new-releases skips it. A later live record with this edition (by ISBN or IDN) clears it.';
