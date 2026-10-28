# LEG (#53) implementation notes

Owner: LEG. These notes record what the legal package built, where it deviates from the work plan, and what the integrator must move into `conventions.md`, `endpoint-map.md`, `CLAUDE.md`, `README.md` and DEP's deployment files. The design documents are unchanged, except the one add-only line in `endpoint-map.md` §0 (work plan §0 item 2).

## Status

**Part 1 (done): the legal paperwork and the server.**

- `docs/legal/privacy-facts.md`, `vvt.md`, `processors.md`, `takedown.md`: built from the code with file references, Rafael's open decisions (Q1, Q4, Q9, Q10, Q12, Q14, the log retention) marked "to confirm". `server/tests/legal_docs.rs` keeps them true to the code (below). After the review: pictures are public by URL and cached for a year (`privacy-facts.md` §2, §7, §10; `vvt.md` activities 7 and 8 and the access-control measure; `takedown.md` §4), imported avatars stay JPEG, and the in-memory rate-limit keys include session ids and recipient email addresses (`privacy-facts.md` §4, §7; `vvt.md` activity 10).
- `server/src/legal.rs`: `LEGAL_DIR`, `GET /api/legal/{doc}`, the startup rules, the terms helpers.
- `server/legal-placeholder/`: the six marked DEV_MODE placeholders and a `manifest.json` (`terms_version` `placeholder`).
- `server/migrations/2200_users_terms.sql`: `users.terms_version`, `users.terms_accepted_at`.
- Terms acceptance on the server: `RegisterIn.terms_version`, `UserOut.needs_terms`, `POST /api/auth/terms`.

**Part 2 (done): the SPA.** The public pages `/imprint`, `/privacy` and `/terms` outside the AppShell with the German aliases, `LegalPage.vue` with an HTML sanitiser, `LegalLinks.vue` on the auth pages and the AppShell, the Legal section in Settings, the privacy link in `EmailPrompt.vue`, the terms sentence and `terms_version` on sign-up, `TermsPrompt.vue`, the service and session stores, and the `apiErrors.ts` mapping of `TERMS_CHANGED`. Checked in a browser against the worktree's server (below).

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

- `msg::TERMS_CHANGED` = `The terms of use have changed. Reload the page and try again.` (`server/src/error.rs`, under `// LEG (#53)`). `apiErrors.ts` maps it to `errors.termsChanged`: "The terms of use have changed. Have a look at them and try again." / "Die Nutzungsbedingungen haben sich geändert. Sieh sie dir an und versuch es nochmal." The app fetches the new version itself on this error (below), so its copy does not ask for a reload, which would empty the sign-up form.

### Configuration (conventions §11)

| Variable | Default | Meaning |
|---|---|---|
| `LEGAL_DIR` | unset | The operator's legal texts: `{imprint,privacy,terms}.{de,en}.html` (HTML fragments as the generator exports them, UTF-8, at most 512 KiB each) plus `manifest.json`. Read once at startup; a change needs a restart (`docker compose restart app`). Unset: DEV_MODE serves the placeholders, production serves none. Set to a directory that does not exist, or that holds neither `manifest.json` nor any of the six texts: no texts (in DEV_MODE too), with a warning in the log. |
| `CONTACT_EMAIL` | (unchanged: required unless DEV_MODE) | Now also the public contact and takedown address in `/api/config.contact_email` (default D3), not only the User-Agent contact. |

`manifest.json`:

```json
{
  "terms_version": "2026-11-01",
  "updated": {"imprint": "2026-11-01", "privacy": "2026-11-01", "terms": "2026-11-01"}
}
```

- Required once `LEGAL_DIR` holds a text file. Unknown keys are refused (a misspelt key fails at startup). A manifest without texts is valid (no texts, no terms).
- `terms_version`: 1 to 64 of `[A-Za-z0-9._-]` (the column check `users_terms_version_format` is the same). Required once a terms file exists. A new value makes every account accept again.
- `updated`: optional per document, `YYYY-MM-DD`; served as `LegalDocumentOut.updated`.

Startup rules (`LegalTexts::configure`, called from `Config::from_lookup`; not in `main.rs`, work plan §0 item 18):

- **An absent or empty `LEGAL_DIR` is no texts, not an error** (`LegalTexts::load`, review fix): a path that does not exist, or a directory with neither `manifest.json` nor any of the six texts (other files are ignored), starts with no texts and logs a warning (`LEGAL_DIR does not exist: …` / `LEGAL_DIR holds no legal texts yet: …`). Invite mode starts; `/api/config.legal` is `[]` and `terms_version` null; open sign-up is refused by the next rule, which then names all six files. DEP's committed `env.example` sets `LEGAL_DIR=/app/legal` before any text exists, and its compose file mounts the empty `/srv/bookclub/legal` there (work plan §0 item 8), so this rule is what lets the first deploy start. **DEP depends on it:** `server/tests/deploy_env.rs::env_example_filled_in_is_a_production_configuration` (and any later `ci_env_is_a_production_configuration`) calls `Config::from_lookup(...).unwrap()` with `LEGAL_DIR=/app/legal`, a path that does not exist on the CI or test host, and DEP's image smoke test mounts an empty directory there with `SIGNUP_MODE=invite`. Both pass only with this rule; keep it when merging DEP. Tests: `legal::tests::an_empty_or_absent_directory_serves_nothing`, `config::tests::an_empty_or_absent_legal_dir_serves_no_texts`, `tests/legal.rs::an_empty_or_absent_legal_dir_starts_invite_mode_without_texts`.
- Still errors: a `LEGAL_DIR` path that is not a directory; text files without `manifest.json`; an invalid manifest; a text file that is not a UTF-8, non-empty file of at most 512 KiB. Errors name `LEGAL_DIR` and the file.
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
- `frontend/src/i18n/messages.test.ts` (owned by no package): one import (`LEGAL_DOCS` from `../legal`) and one added test, `names every legal document, and the languages a text may fall back to, in both catalogs`. No existing test changed. ADM's branch does not touch the file today; if another package adds a test there, both merge as separate hunks.
- Generated: `server/.sqlx/` and `frontend/src/api/generated.ts`. `cargo sqlx prepare` on a fresh database also rewrote the `nullable` array of one unrelated query (`query-fb32f4f8…`, the reactions loader: three overridden `!` columns from `true` to `false`); it changes no code, and the integrator's regeneration settles it.

### Frontend (Part 2: the SPA)

**Routes** (`frontend/src/router.ts`; endpoint-map's SPA route list and CLAUDE.md "Frontend" can name them):

- `/imprint`, `/privacy`, `/terms`: `LegalPage.vue` with the prop `doc`. Top-level records, declared before the AppShell, with no `meta`: public signed in and signed out, and not guest pages (a signed-in reader stays). Inside the AppShell its catch-all would send a signed-out visitor to `/login`.
- `/impressum` → `/imprint`, `/datenschutz` → `/privacy`, `/nutzungsbedingungen` → `/terms` (redirect records).
- The server's SPA fallback (`server/src/spa.rs`) already answers these paths with the shell; no server change.

**Rendering the operator's HTML: a sanitiser, not a sandboxed iframe.** Decision (asked for in the Part 2 task):

- `frontend/src/legal.ts` `sanitizeLegalHtml` parses the HTML with `DOMParser` (no script runs, nothing loads), keeps an allow-list of text elements (headings, paragraphs, lists, tables, links, emphasis, `section`/`aside`/`div`/`span`, `address`, `time`, …) and unwraps any other element, keeping its text. The allow-lists are a `Map` and `Set`s, never a plain object, so `Object.prototype` names (`<constructor>`, which the parser accepts as a tag) are unknown tags like any other. Script, style, frames, embeds, forms, `img`/`picture`/`video`/`audio`, SVG, MathML, `template`, `noscript` and comments are removed with their content. Attributes are an allow-list too: `id`, `lang`, `dir`, `title`, `data-legal-placeholder` everywhere, `href` on links, spans and `scope` on table cells, `start`/`type`/`reversed` on lists, `datetime` on `time`. So no `on*` handler, `style`, `class`, `target` or `src` survives. A link keeps its `href` only if it resolves to http, https, mailto or tel (`javascript:`, `data:`, `vbscript:`, `file:` and their obfuscated spellings go); a link to another origin gets `target="_blank" rel="noopener noreferrer"`. The result has no raw-text element, so re-parsing it in `v-html` builds the same tree (tested with the usual mutation-XSS payloads).
- Why not the iframe: a `sandbox` iframe without `allow-same-origin` cannot be sized to its content or styled from the app (fonts, the four palettes, dark mode), links inside it navigate the frame, and every text would need its own stylesheet; with `allow-same-origin` and no scripts it is safe but still needs measuring and link plumbing. The texts are the operator's own files, so the threat is a careless template or a pasted snippet, not a hostile author; an allow-list covers that and keeps the page in the app's type. Pictures are dropped on purpose: the privacy facts say the browser contacts no third party (`docs/legal/privacy-facts.md` §6 now names this).
- GATE's CSP (`script-src 'self'`, no inline handlers) is a second layer on top.
- Links inside a text: `#section` scrolls within the text (the router's `scrollBehavior` would jump to the top), a path of this app goes through the router, anything else is left to the browser.

**Components and pages:**

- `LegalPage.vue`: wordmark, EN|DE switch, back link, the sanitised text in an `<article lang>` (the served locale), a note when the server fell back to the other language (`legal.onlyIn.*`), "Last updated …" from `updated`, and `LegalLinks` at the bottom. It reloads when the reader switches the language. A 404 (the operator has no such text) shows the title and "This text is not online yet." plus the contact address when `contact_email` is set; any other failure offers "Try again". It adds no heading of its own above the text: generator exports bring theirs.
- `LegalLinks.vue`: "Imprint · Privacy · Terms" (`legal.short.*`), only the documents in `/api/config.legal`; nothing when there are none. On Login, Signup, ForgotPassword, ResetPassword, VerifyEmail and Join (below the language switch), and as a footer at the end of the AppShell's `<main>`, hidden on `/settings`, whose Legal section lists the same texts.
- `TermsPrompt.vue`: in the AppShell above `EmailPrompt`, shown while `session.user.needs_terms`: "The terms of use of {name} are new or have changed. Please read them and accept them." plus the privacy sentence, and an "Accept the terms" button. It cannot be dismissed without accepting (EmailPrompt's "Not now" has no meaning here); the app is not blocked meanwhile. The button sends `service.terms_version` (fetched first if the store has none) through `session.acceptTerms`, which stores the returned `UserOut`.
- Sign-up (Q13 option A): above the button, "By creating an account, you accept the {terms of use}. The {privacy policy} explains what we store about you." Each half shows only when the server has that text. The links open in a new tab, so the form keeps what was typed. `session.register` sends `terms_version: service.terms_version` (null without terms; the server ignores it then).
- `TERMS_CHANGED` on sign-up or in the prompt: the app reloads the legal settings (`service.reloadLegal()`) and shows the error, so the next tap sends the new version without a page reload.
- `EmailPrompt.vue`: the privacy sentence with a link after its body, when the server has a privacy policy.
- `SettingsPage.vue`: a Legal section at the very end, after "Log out": a blurb, one link per served text (`legal.title.*`), and "Questions about your data, or content to report? Write to {email}." as a `mailto:` link when `CONTACT_EMAIL` is set. The section is hidden when there are neither texts nor a contact.

**Stores and modules:**

- `stores/service.ts`: the state is the whole `ServiceConfig` (`legal`, `terms_version`, `contact_email` added; `LEGAL_DEFAULTS` before the config arrives or when it fails), a getter `hasLegal(doc)`, and `reloadLegal()`, which patches only the three legal fields and keeps them when the request fails.
- `stores/session.ts`: `register` takes an optional `termsVersion`; new `acceptTerms(version)`.
- `api/client.ts` (add-only): `api.legal(doc, locale)` → `GET /api/legal/{doc}?locale=`, `api.acceptTerms(version)` → `POST /api/auth/terms`; two type imports.
- `types.ts`: `LegalDoc` and `LegalDocumentOut` re-exported.
- New `frontend/src/legal.ts`: `LEGAL_DOCS`, `legalPath`, `TERMS_CHANGED`, `isTermsChanged`, `sanitizeLegalHtml`.

**i18n** (`en.json`, `de.json`): new top-level `legal` (`nav`, `short.*`, `title.*`, `inline.*`, `privacyNote`, `updated`, `onlyIn.{de,en}`, `missing`, `failed`, `back`, `contact`, `settingsTitle`, `settingsBlurb`) and `terms` (`signup`, `promptTitle`, `promptBody`, `accept`, `accepting`, `accepted`, `failed`), and `errors.termsChanged`.

### Frontend (Part 1 only what the generated types forced)

- `frontend/src/types.ts`: `ServiceBrand` is now `Pick<ServiceConfig, "name" | "theme" | "theme_dark" | "public_url" | "signup">`, the branding part, so `brand.ts`'s `BRAND_DEFAULTS` stays valid without the legal fields. Part 2 gives the service store the legal fields.
- `frontend/src/test/fixtures.ts`: `needs_terms: false` in `user()` (one added line; the file is not in LEG's list, see deviations).

## Tests

- `server/tests/legal.rs` (7): every document and locale served anonymously with `no-store`, the default locale, case-insensitive locales; the fallback reported both ways; unknown documents (`impressum`, `Terms`, `terms.de.html`, `manifest.json`, `..%2Fmanifest.json`, `%2E%2E`) 404, an unknown locale 400, POST 405; the manifest version and the contact address in `/api/config`; production without texts serves none; an empty or absent `LEGAL_DIR` starts production in invite mode with no texts, `needs_terms` false, and open sign-up with it is refused with all six files named (`an_empty_or_absent_legal_dir_starts_invite_mode_without_texts`); DEV_MODE serves the placeholders.
- `server/tests/legal_docs.rs` (6): every table with a foreign key to `users` (and `users`) is in `vvt.md`; every outbound host (`config.rs` defaults, `outbound.rs` redirect domains) has a verdict line in `processors.md`; every `METHOD /path` in `takedown.md` is an operation of the OpenAPI document or in its `PLANNED` list (ADM's and ADM2's endpoints: the integrator removes each once its package merges), and every `.vue` it names exists; every `server/…`, `frontend/…` and `docs/…` file the four documents cite exists, with its line numbers inside the file; every name cited as `` `path` (`name`, …) `` is still in that file (`every_cited_name_exists`: a snake_case name in a Rust file must be a `fn`, a SCREAMING_CASE one a `const` or `static`, anything else a word; for a directory, an entry); and no `path:line` citation points into a file another Phase 2 package edits (`files_other_packages_edit_are_cited_by_name`, list `EDITED_ELSEWHERE`).
- `server/tests/auth_config.rs`: the exact `/api/config` body gains the three fields; `config_follows_the_service_settings` now needs `LEGAL_DIR` for open sign-up and checks the three fields; new `open_sign_up_without_the_legal_texts_is_refused_outside_dev_mode` (missing files listed, invite mode and DEV_MODE start, placeholders refused) and `dev_mode_serves_placeholders_and_the_contact_is_public`.
- `server/tests/auth_required.rs`: `GET /api/legal/{doc}` in the public allow-list.
- `server/tests/auth_register.rs`: `needs_terms` in the full `UserOut`; new `a_sign_up_accepts_the_current_terms` (missing, old, empty and null versions are 400 and create nothing; the current one is stored with the time) and `without_terms_a_sign_up_stores_no_acceptance`.
- `server/tests/auth_account.rs`: `needs_terms` in the full `UserOut`; new `needs_terms_flips_after_accepting_the_terms` (also on `PATCH /api/auth/me/preferences` and MED's `DELETE /api/auth/me/avatar`; a stale or empty version 400, `{}` 400 `Field required`; accepting again keeps the first time; an older accepted version asks again) and `without_terms_nobody_is_asked`.
- `server/tests/auth_export.rs`: new `the_export_holds_the_accepted_terms`.
- Unit tests: `legal::tests` (11: loading, fallback, the production rules, an empty or absent directory, the manifest checks, bad files, exact document names, the version format, the task-local), `config::tests` (2 new: open sign-up needs the texts; an empty or absent `LEGAL_DIR` serves no texts while a file in its place or texts without a manifest do not start; `rejects_bad_enums_and_bools` now uses invite mode for its valid case).

Frontend (Part 2), `npx vitest run src/router.test.ts src/pages/LegalPage.test.ts src/pages/AuthPages.test.ts src/i18n/messages.test.ts src/legal.test.ts src/components/TermsPrompt.test.ts src/layouts/AppShell.test.ts`:

- `src/router.test.ts` (3 new): the three pages open signed out as top-level records with their `doc` prop, and signed in; the German aliases redirect, signed out and in.
- `src/pages/LegalPage.test.ts` (6, new): the text in the reader's language without its script, handler, picture or `javascript:` link, the updated date and the links; the fallback note and `lang`; reloading on EN|DE; the 404 state with the contact; retry after a failure; `#section` scrolls, an own path goes through the router, an external link gets a new tab.
- `src/pages/AuthPages.test.ts` (5 new): every auth page (Login, Signup, ForgotPassword, ResetPassword, VerifyEmail, Join) links the imprint, the privacy policy and the terms; only the served texts are linked; the sign-up sentence names both texts right above the button and sends the version; no version without terms; `TERMS_CHANGED` fetches the new version and the next try sends it.
- `src/i18n/messages.test.ts` (1 new): every `LegalDoc` has `legal.title.*` and `legal.short.*`, and both `legal.onlyIn.*` exist, in both catalogs.
- `src/legal.test.ts` (10, new): the sanitiser (kept structure and links, removed elements with their content, attributes, every `javascript:` spelling, unwrapped unknown elements, `<constructor class=…>` unwrapped like any unknown tag, mutation-XSS payloads stay inert and the output is stable, own-origin links), `isTermsChanged`, and the en/de translation of `TERMS_CHANGED`.
- `src/components/TermsPrompt.test.ts` (4, new): hidden without `needs_terms`; accepting sends the current version and hides it; `TERMS_CHANGED` fetches the new version, which the next tap accepts; a missing version is fetched first.
- `src/layouts/AppShell.test.ts` (6, new): mounts the real AppShell under a router: `TermsPrompt` comes first in `<main>`, above `EmailPrompt` and the page; neither prompt for an account that needs neither; the footer's `LegalLinks` list only the served texts and the footer is gone without any; on `/settings` (the real `SettingsPage`, its heavy sections stubbed) the footer is gone and the Legal section lists the texts and the `mailto:` contact, shows for the contact alone, and is gone without both; `EmailPrompt` links `/privacy` only when the server serves it.

Browser check (Part 2), with `playwright-cli` against the worktree's server (`STATIC_DIR=../frontend/dist BIND_ADDR=0.0.0.0:8084 PUBLIC_URL=http://agents.lan:8084 DEV_MODE=true`):

- A fresh browser without cookies opens `http://agents.lan:8084/imprint`: the placeholder imprint renders (`/api/auth/me` 401, no redirect), with the legal links; `/datenschutz`, `/nutzungsbedingungen` and `/impressum` land on `/privacy`, `/terms` and `/imprint`; EN|DE reloads the text in German; dark mode and a 390 px viewport render.
- With `LEGAL_DIR=legal-placeholder` (terms version `placeholder`): the sign-up sentence shows above the button; a sign-up with a `beta-invite` code stored `terms_version = placeholder` with a time; the AppShell footer shows the links; Settings ends with the Legal section.
- Restarted with a copy of the placeholders whose manifest says `placeholder-2`: the same account sees `TermsPrompt` on Home; "Accept the terms" stored `placeholder-2`, showed the toast, and the prompt stayed gone after a reload.

## For the docs and the deployment

- **README / SELFHOST (hosted):** how to generate the texts (eRecht24 or a lawyer), export them as HTML fragments, name the six files, write `manifest.json`, put them into `/srv/bookclub/legal` (DEP's README creates it, owned by uid 1000, work plan §0 item 8), restart, and check `/imprint`, `/privacy` and `/terms` signed out (work plan §5 step 19). Bumping `terms_version` makes everyone accept again.
- **DEP:** mount `/srv/bookclub/legal` read-only and set `LEGAL_DIR` to the mount point in the compose file; add `LEGAL_DIR` to `deploy/hosted/env.example` (commented: required before `SIGNUP_MODE=open`). DEP's branch already does both (`LEGAL_DIR=/app/legal`). Its `deploy_env.rs` production-configuration tests and the image smoke test rely on an absent or empty `LEGAL_DIR` starting invite mode (Configuration, startup rules); once LEG is merged, DEP's `READ_BY_A_LATER_PACKAGE` entry for `LEGAL_DIR` is redundant and may go, as its own comment says.
- **Citations in `docs/legal/`, after each merge.** Files another Phase 2 package edits are cited by name (`` `server/src/media/reports.rs` (`take_down_locked`, `record_removed_hashes`) ``), not by line, and `legal_docs.rs` fails when a cited name disappears. Line citations remain only into files no other package edits. After merging:
  - **ADM:** `legal_docs.rs` keeps the names honest, but check that they still describe the behaviour: `takedown.md` §2 (`reports.rs` `report`, `ReportIn`), §4 (`take_down_locked`, `record_removed_hashes`, the admin screens), `privacy-facts.md` §5 (the four `templates.rs` functions: add ADM's new templates, the admin notification, to the mail-provider row) and §7 (`jobs.rs` `CLEANUP_EVERY`, `auth_cleanup_loop`; add ADM's notify job if it holds personal data), §6 (`BookDetailPage.vue` `book.viewOpenLibrary`). Remove ADM's entries from `PLANNED`.
  - **GATE:** the palette script moves from `frontend/index.html` into `frontend/public/theme-init.js`. `privacy-facts.md` §3 cites `frontend/index.html` without a line, which stays true (it loads the script); add `` `frontend/public/theme-init.js` `` to that row. §6's sentence "GATE (#55) adds a Content-Security-Policy" becomes present tense.
  - **`EDITED_ELSEWHERE`** in `legal_docs.rs` lists the files other packages edit (ADM, ADM2, GATE, DEP, CUT, LEG2). It only forbids line citations into them; trim or extend it when ownership changes.
- **CLAUDE.md:** DEV_MODE serves placeholder legal pages; `LEGAL_DIR=legal-placeholder` (from `server/`) tries the terms acceptance; production refuses open sign-up without the six files.
- **CLAUDE.md, Frontend:** the legal pages are top-level routes outside the AppShell (public); the operator's HTML goes through `sanitizeLegalHtml` (`src/legal.ts`) before `v-html`, and any new place that renders server HTML must use it too.
- **endpoint-map, SPA routes:** `/imprint`, `/privacy`, `/terms` and the three German redirects.
- **Work plan §5 step 26:** startup now enforces "Outside `DEV_MODE`, startup refuses this until the legal texts are present".

## Deviations

- **A layer in `lib.rs`** besides the `legal::routes()` line: `UserOut.needs_terms` needs the current terms version inside the frozen `user_out`, which has no configuration and is also called by MED's avatar endpoints (not LEG's). See "`needs_terms` and the frozen `user_out`" for the alternative.
- **`frontend/src/test/fixtures.ts`** (not in LEG's list) gained one line, `needs_terms: false`: without it `npm run typecheck` fails as soon as the generated `UserOut` has the field.
- **`/api/config.legal`** is a list of the served documents, not a boolean, so the SPA can link only what exists while `SIGNUP_MODE=invite` runs with a partial set.
- **`LegalDocumentOut.fallback`** is an addition to "the response says which one it served" (`locale`): it saves the SPA the comparison.
- **The DEV_MODE placeholders have no terms version** (see Behaviour): the test harness runs in DEV_MODE, and other packages' tests register without one.
- **No `LegalLinks` inside `NotFoundPage.vue`.** The not-found page renders inside the AppShell (it is the shell's catch-all, signed in only), and the shell's footer already shows the legal links under it; a second copy in the page would show them twice. `NotFoundPage.vue` is unchanged.
- **New frontend files** not named in LEG's list, all new paths only LEG uses: `src/legal.ts` (the sanitiser and the terms helpers) with `src/legal.test.ts`, `src/components/TermsPrompt.test.ts`, and `src/layouts/AppShell.test.ts` (the shell is LEG's; the test also mounts the shared `SettingsPage.vue` to check LEG's Legal section, without changing it).
- **`frontend/src/i18n/messages.test.ts`** (owned by no package, though the work plan's acceptance command runs it) gained one import and one test (see Shared files touched).
- **An absent or empty `LEGAL_DIR` is no texts** rather than a startup error (Configuration): the work plan mounts the directory empty before the texts exist, and DEP's tests and smoke test start the server that way.
- **The AppShell's legal footer is hidden on `/settings`**, which lists the same texts in its Legal section.
- **The en/de copy of `TERMS_CHANGED`** does not say "Reload the page" like the server's English: the app fetches the new version itself, and a reload would empty the sign-up form.
