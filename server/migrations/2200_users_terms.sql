-- LEG (#53): which version of the operator's terms of use an account accepted, and when.
-- Sign-up stores the current version (`LEGAL_DIR/manifest.json`) whenever the terms are
-- configured; `POST /api/auth/terms` stores it for imported accounts and after a new version.
-- Both stay NULL for an account that never accepted any version. The export carries both
-- columns with the rest of the `users` row (docs/phase1/conventions.md §5.7).
ALTER TABLE users
    ADD COLUMN terms_version text NULL
        CONSTRAINT users_terms_version_format
        CHECK (terms_version IS NULL OR terms_version ~ '^[A-Za-z0-9._-]{1,64}$'),
    ADD COLUMN terms_accepted_at timestamptz NULL,
    ADD CONSTRAINT users_terms_accepted_together
        CHECK ((terms_version IS NULL) = (terms_accepted_at IS NULL));

COMMENT ON COLUMN users.terms_version IS
    'The terms version (LEGAL_DIR/manifest.json terms_version) the account accepted last; NULL if it never accepted one (imported accounts, sign-ups while no terms were configured). UserOut.needs_terms is true while it differs from the current version.';
COMMENT ON COLUMN users.terms_accepted_at IS
    'When the account accepted terms_version: at sign-up, or through POST /api/auth/terms. Kept as evidence of the acceptance; NULL exactly when terms_version is NULL.';
