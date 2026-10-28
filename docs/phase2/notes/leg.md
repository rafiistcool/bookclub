# LEG (#53) implementation notes

Owner: LEG. These notes record what the legal package built, where it deviates from the work plan, and what the integrator must move into `conventions.md`, `endpoint-map.md`, `CLAUDE.md`, `README.md` and DEP's deployment files. The design documents are unchanged, except the one add-only line in `endpoint-map.md` §0 (work plan §0 item 2).

## Status

**Part 1 (done): the legal paperwork and the server.**

- `docs/legal/privacy-facts.md`, `vvt.md`, `processors.md`, `takedown.md`: built from the code with file references, Rafael's open decisions (Q1, Q4, Q9, Q10, Q12, Q14, the log retention) marked "to confirm". `server/tests/legal_docs.rs` keeps them true to the code (below).
- `server/src/legal.rs`: `LEGAL_DIR`, `GET /api/legal/{doc}`, the startup rules, the terms helpers.
- `server/legal-placeholder/`: the six marked DEV_MODE placeholders and a `manifest.json` (`terms_version` `placeholder`).
- `server/migrations/2200_users_terms.sql`: `users.terms_version`, `users.terms_accepted_at`.
- Terms acceptance on the server: `RegisterIn.terms_version`, `UserOut.needs_terms`, `POST /api/auth/terms`.

**Part 2 (open): the SPA.** Routes `/imprint`, `/privacy`, `/terms` outside the AppShell with the German aliases, `LegalPage.vue`, `LegalLinks.vue` on the auth pages, NotFound and the AppShell, the Legal section in Settings, the privacy link in `EmailPrompt.vue`, the terms sentence on sign-up, `TermsPrompt.vue`, the service and session stores, the `apiErrors.ts` mapping of `TERMS_CHANGED` with its en/de strings, and the browser check at `http://agents.lan:8084/imprint`.

## Contract changes for the integrator

### Routes (endpoint-map §1 and §2, conventions §17.7)

| Route | Auth | Request → Response | Errors |
|---|---|---|---|
| `GET /api/legal/{doc}` (new, tag `legal`, operation `legal_get_document`) | none (public) | `?locale=de\|en` (optional, default `en`; case and spaces ignored) → `LegalDocumentOut {doc: LegalDoc, locale: "de"\|"en", fallback: bool, updated: "YYYY-MM-DD"\|null, html: string}` | 404 `Not found` (unknown `doc`, or a document the operator has not provided in either language); 400 `locale must be one of: en, de` |
| `POST /api/auth/terms` (new, operation `auth_accept_terms`) | user | `{terms_version}` (`TermsIn`) → 200 `UserOut` (`needs_terms` false) | 400 `The terms of use have changed. Reload the page and try again.` (not the current version, or no terms configured); 400 `Field required`; 401 |
| `POST /api/auth/register` (changed) | none | new optional field `terms_version: string\|null` | 400 `The terms of use have changed. Reload the page and try again.` when the operator's terms are configured and `terms_version` is missing or not the current version. Checked after the 409 `That username is already taken` and before the invite checks; counts against the sign-up limit like every validated attempt |
| `GET /api/config` (changed) | none | three new fields: `legal: LegalDoc[]` (the documents `/api/legal/{doc}` serves), `terms_version: string\|null` (the current terms version; null without terms), `contact_email: string\|null` (`CONTACT_EMAIL`) | |

- `{doc}` is a new path parameter name: add it to the parameter table at the top of `endpoint-map.md` (`imprint`, `privacy` or `terms`).
- `LegalDoc = "imprint"|"privacy"|"terms"` is a schema of its own; `LegalDocumentOut` and `TermsIn` too.
- conventions §17.7: the anonymous allow-list gains `GET /api/legal/{doc}` (`tests/auth_required.rs`, now 14 entries).
- `UserOut.needs_terms: bool` (endpoint-map §0, line added): true while the operator's terms are configured and the account has not accepted their current version (imported accounts, sign-ups from before the terms, everyone after a new version). conventions §20 needs the field in the `UserOut` block, and §5.3's `user_out` description its rule.

### Error strings (endpoint-map §11.3, `error::msg`)

- `msg::TERMS_CHANGED` = `The terms of use have changed. Reload the page and try again.` (`server/src/error.rs`, under `// LEG (#53)`). Part 2 maps it in `apiErrors.ts` with en and de strings.

### Configuration (conventions §11)

| Variable | Default | Meaning |
|---|---|---|
| `LEGAL_DIR` | unset | The operator's legal texts: `{imprint,privacy,terms}.{de,en}.html` (HTML fragments as the generator exports them, UTF-8, at most 512 KiB each) plus `manifest.json`. Read once at startup; a change needs a restart (`docker compose restart app`). Unset: DEV_MODE serves the placeholders, production serves none. |
| `CONTACT_EMAIL` | (unchanged: required unless DEV_MODE) | Now also the public contact and takedown address in `/api/config.contact_email` (default D3), not only the User-Agent contact. |

`manifest.json`:

```json
{
  "terms_version": "2026-11-01",
  "updated": {"imprint": "2026-11-01", "privacy": "2026-11-01", "terms": "2026-11-01"}
}
```

- Required whenever `LEGAL_DIR` is set. Unknown keys are refused (a misspelt key fails at startup).
- `terms_version`: 1 to 64 of `[A-Za-z0-9._-]` (the column check `users_terms_version_format` is the same). Required once a terms file exists. A new value makes every account accept again.
- `updated`: optional per document, `YYYY-MM-DD`; served as `LegalDocumentOut.updated`.

Startup rules (`LegalTexts::configure`, called from `Config::from_lookup`; not in `main.rs`, work plan §0 item 18):

- `LEGAL_DIR` must be a directory with a valid `manifest.json`; a text file that exists must be a UTF-8, non-empty file of at most 512 KiB. Errors name `LEGAL_DIR` and the file.
- Outside DEV_MODE, a file that carries the placeholder marker (`data-legal-placeholder`) is refused.
- Outside DEV_MODE, `SIGNUP_MODE=open` is refused unless all six files are present; the error lists the missing ones. `SIGNUP_MODE=invite` starts with any subset (Q3's alternatives: imprint and privacy first, the terms at #55, cost nothing).
- A missing language of a document falls back to the other one.

### Behaviour (conventions §5, new section for the legal texts)

- **Placeholders.** DEV_MODE without `LEGAL_DIR` serves the six files of `server/legal-placeholder/`, compiled into the binary (`include_str!`, so the image needs no extra files; `.html`, because the root `.dockerignore` drops `*.md`). They carry **no** terms version: `terms_version` is null, so a development sign-up needs no acceptance and every existing test that registers keeps working. To try the acceptance in development, start the server with `LEGAL_DIR=legal-placeholder` (from `server/`): the same files with their `manifest.json`, terms version `placeholder`.
- **Terms acceptance.** `legal::accepted_terms(config, given)` decides for sign-up and `POST /api/auth/terms`: no terms configured → nothing is stored, a given version is ignored (sign-up) or refused (accept); terms configured → `given` must equal the current version. Sign-up stores the version with `terms_accepted_at = now()` in the same insert. Accepting the version already accepted keeps the first time.
- **`needs_terms` and the frozen `user_out`.** `auth::user_out(conn, user_id)` keeps its frozen signature, which has no configuration, and MED's avatar endpoints call it too. A new layer, `legal::terms_scope`, runs every request with the configuration's terms version in a tokio task-local; `user_out` reads it through `legal::needs_terms`. Outside a request it is false. conventions §3's layer list gains it, between `DefaultBodyLimit` and `maintain_cookie` (it wraps the handlers; its position among the inner layers does not matter). The cheaper alternative, if the integrator prefers it: give `user_out` a `&Config` argument, update its two calls in `server/src/media/avatar.rs`, and drop the layer.
- **Export.** The profile section is `to_jsonb(u) - 'password_hash'` (`server/src/auth/export.rs:151-160`), so `terms_version` and `terms_accepted_at` are exported without a code change (work plan §0 item 17); `tests/auth_export.rs::the_export_holds_the_accepted_terms` pins it. conventions §5.7's profile row can say so.
- **Caching.** `GET /api/legal/{doc}` carries the API's `Cache-Control: no-store` like every API answer.

### Schema

- `server/migrations/2200_users_terms.sql`: `users.terms_version text NULL` (check `^[A-Za-z0-9._-]{1,64}$`), `users.terms_accepted_at timestamptz NULL`, and `users_terms_accepted_together` (both NULL or both set). No foreign key, so `auth_export.rs`'s pinned count is unchanged.

### Shared files touched

- `server/src/lib.rs`: three separate hunks: `pub mod legal;`, `.merge(legal::routes())` in `api_router`, and the `legal::terms_scope` layer after `DefaultBodyLimit` (the layer is a deviation, below).
- `server/src/error.rs`: one constant added.
- `server/tests/common/mod.rs`: one helper added at the end, `legal_dir(terms_version) -> tempfile::TempDir` (all six texts `<p>{doc} {locale}</p>` and a manifest with every document updated 2026-11-01). No existing helper changed.
- `docs/phase1/endpoint-map.md` §0: the `needs_terms: bool` line in `UserOut`.
- Generated: `server/.sqlx/` and `frontend/src/api/generated.ts`. `cargo sqlx prepare` on a fresh database also rewrote the `nullable` array of one unrelated query (`query-fb32f4f8…`, the reactions loader: three overridden `!` columns from `true` to `false`); it changes no code, and the integrator's regeneration settles it.

### Frontend (Part 1 only what the generated types forced)

- `frontend/src/types.ts`: `ServiceBrand` is now `Pick<ServiceConfig, "name" | "theme" | "theme_dark" | "public_url" | "signup">`, the branding part, so `brand.ts`'s `BRAND_DEFAULTS` stays valid without the legal fields. Part 2 gives the service store the legal fields.
- `frontend/src/test/fixtures.ts`: `needs_terms: false` in `user()` (one added line; the file is not in LEG's list, see deviations).

## Tests

- `server/tests/legal.rs` (6): every document and locale served anonymously with `no-store`, the default locale, case-insensitive locales; the fallback reported both ways; unknown documents (`impressum`, `Terms`, `terms.de.html`, `manifest.json`, `..%2Fmanifest.json`, `%2E%2E`) 404, an unknown locale 400, POST 405; the manifest version and the contact address in `/api/config`; production without texts serves none; DEV_MODE serves the placeholders.
- `server/tests/legal_docs.rs` (4): every table with a foreign key to `users` (and `users`) is in `vvt.md`; every outbound host (`config.rs` defaults, `outbound.rs` redirect domains) has a verdict line in `processors.md`; every `METHOD /path` in `takedown.md` is an operation of the OpenAPI document or in its `PLANNED` list (ADM's and ADM2's endpoints: the integrator removes each once its package merges), and every `.vue` it names exists; every `server/…`, `frontend/…` and `docs/…` file the four documents cite exists, with its line numbers inside the file.
- `server/tests/auth_config.rs`: the exact `/api/config` body gains the three fields; `config_follows_the_service_settings` now needs `LEGAL_DIR` for open sign-up and checks the three fields; new `open_sign_up_without_the_legal_texts_is_refused_outside_dev_mode` (missing files listed, invite mode and DEV_MODE start, placeholders refused) and `dev_mode_serves_placeholders_and_the_contact_is_public`.
- `server/tests/auth_required.rs`: `GET /api/legal/{doc}` in the public allow-list.
- `server/tests/auth_register.rs`: `needs_terms` in the full `UserOut`; new `a_sign_up_accepts_the_current_terms` (missing, old, empty and null versions are 400 and create nothing; the current one is stored with the time) and `without_terms_a_sign_up_stores_no_acceptance`.
- `server/tests/auth_account.rs`: `needs_terms` in the full `UserOut`; new `needs_terms_flips_after_accepting_the_terms` (also on `PATCH /api/auth/me/preferences` and MED's `DELETE /api/auth/me/avatar`; a stale or empty version 400, `{}` 400 `Field required`; accepting again keeps the first time; an older accepted version asks again) and `without_terms_nobody_is_asked`.
- `server/tests/auth_export.rs`: new `the_export_holds_the_accepted_terms`.
- Unit tests: `legal::tests` (10: loading, fallback, the production rules, the manifest checks, bad files, exact document names, the version format, the task-local), `config::tests` (2 new: open sign-up needs the texts, `LEGAL_DIR` must exist; `rejects_bad_enums_and_bools` now uses invite mode for its valid case).

## For the docs and the deployment

- **README / SELFHOST (hosted):** how to generate the texts (eRecht24 or a lawyer), export them as HTML fragments, name the six files, write `manifest.json`, put them into `/srv/bookclub/legal` (DEP's README creates it, owned by uid 1000, work plan §0 item 8), restart, and check `/imprint`, `/privacy` and `/terms` signed out (work plan §5 step 19). Bumping `terms_version` makes everyone accept again.
- **DEP:** mount `/srv/bookclub/legal` read-only and set `LEGAL_DIR` to the mount point in the compose file; add `LEGAL_DIR` to `deploy/hosted/env.example` (commented: required before `SIGNUP_MODE=open`).
- **CLAUDE.md:** DEV_MODE serves placeholder legal pages; `LEGAL_DIR=legal-placeholder` (from `server/`) tries the terms acceptance; production refuses open sign-up without the six files.
- **Work plan §5 step 26:** startup now enforces "Outside `DEV_MODE`, startup refuses this until the legal texts are present".

## Deviations

- **A layer in `lib.rs`** besides the `legal::routes()` line: `UserOut.needs_terms` needs the current terms version inside the frozen `user_out`, which has no configuration and is also called by MED's avatar endpoints (not LEG's). See "`needs_terms` and the frozen `user_out`" for the alternative.
- **`frontend/src/test/fixtures.ts`** (not in LEG's list) gained one line, `needs_terms: false`: without it `npm run typecheck` fails as soon as the generated `UserOut` has the field.
- **`/api/config.legal`** is a list of the served documents, not a boolean, so the SPA can link only what exists while `SIGNUP_MODE=invite` runs with a partial set.
- **`LegalDocumentOut.fallback`** is an addition to "the response says which one it served" (`locale`): it saves the SPA the comparison.
- **The DEV_MODE placeholders have no terms version** (see Behaviour): the test harness runs in DEV_MODE, and other packages' tests register without one.
