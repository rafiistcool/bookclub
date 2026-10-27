# OAPI (#47) implementation notes

Owner: OAPI. These notes record what the package built, where it deviates from the work plan, and what FE, PAR and the integrator need to know. The design documents themselves are unchanged.

## What OAPI built

| File | Contents |
|---|---|
| `frontend/scripts/gen-api-types.mjs` | Runs `cargo run --quiet --locked --manifest-path ../server/Cargo.toml -- openapi` with `SQLX_OFFLINE=true` (no database), marks response fields required (below), and writes `src/api/generated.ts` through the openapi-typescript Node API. `--check` then runs `git diff --exit-code` on the file and fails with a hint when it changed. |
| `frontend/src/api/generated.ts` | The generated types: `paths`, `operations`, `components`, and one alias per schema (`export type BookOut = components['schemas']['BookOut']`). It is generated from phase1-green at 1d34355 (W1, CAT, SHELF, IMP: 78 operations, 91 schemas). |
| `frontend/package.json` | `gen:api` (`node scripts/gen-api-types.mjs`), `check:api` (`… --check`), and the devDependency `openapi-typescript` `^7.13.0`. |
| `frontend/package-lock.json` | The lockfile entries that devDependency brings (additions only; `npm ci` works). |
| `.forgejo/workflows/ci.yml`, `.github/workflows/ci.yml` | Three steps at the end of the `server` job: `actions/setup-node` (Node 22, npm cache), `npm ci` and `npm run check:api`, all in `frontend/`. |
| `server/tests/openapi_contract.rs` | 6 tests, below. |

### Commands

- `cd frontend && npm run gen:api` regenerates `src/api/generated.ts`. It builds the server with `cargo` if needed (the first build takes a few minutes; afterwards it takes seconds).
- `cd frontend && npm run check:api` regenerates and fails when the file differs from git's index. It needs a git checkout and a Rust toolchain.

### Why the CI step is in the server job

The work plan left two options: give the frontend job a Rust toolchain, or run the check in the server job after it has built. The server job is the cheaper one. By the time it reaches the check, `cargo test` has already built the dependency tree and the `bookclub-server` binary, so `cargo run -- openapi` only starts the binary. At most it recompiles the server crate itself, not the dependencies. Locally, after `cargo build` and `cargo test`, `npm run check:api` (with its `SQLX_OFFLINE=true`) found nothing to rebuild and took under a second. After a change to a DTO it recompiled only the server crate (about 8 s in all). The check therefore adds a Node setup and `npm ci` (seconds with the npm cache). In the frontend job it would need `dtolnay/rust-toolchain`, a second rust-cache entry and a full debug build of the dependency tree (about 370 crates in `Cargo.lock`). The check runs after `Test`, so a stale `generated.ts` never hides test results.

### Response fields are required in the generated types

utoipa 6 marks every `Option<T>` field as not required. openapi-typescript would turn that into `cover_url?: string | null`. But endpoint-map.md says "Every response field is always present; nullable fields are `null`", and serde writes `None` as `null`. W1 left the decision to OAPI (wave1-notes, step 1). Changing the frozen `dto.rs` to `#[schema(required)]` was not an option, and neither was asking every package to annotate each field.

So the generator edits the document in memory before handing it to openapi-typescript:
- Every property of a component schema that no request body or parameter references (directly or through nested `$ref`s) becomes required.
- So does every property of an inline schema in a response.
- The types then read `cover_url: string | null`.

Schemas that requests use keep their optional fields, because a client may leave them out. These are the `*In` bodies, and also the shelf layout types (`PhysicalShelfLayout`, `PhysicalSpine`, …), which are both the `PUT` body and the `GET` answer. So in responses they keep `rows?:`, which is weaker than the truth but never wrong.

`openapi_contract.rs::schema_types_never_skip_a_field` guards the rule: it fails if any `ToSchema` type in `server/src` uses `skip_serializing` or `skip_serializing_if`, because the types would then promise a field the server leaves out. **For every package:** a response field that should sometimes be absent needs a different design. Nullable is fine.

### `openapi_contract.rs`

- `every_section_0_dto_is_in_the_document`: parses the `text` block of endpoint-map.md §0 and checks that every DTO name it lists is in `components.schemas`, so a renamed DTO is caught.
- `section_0_objects_match_the_document`: every §0 object has exactly the fields §0 lists, in both directions. That includes the inline `[{name, role}]` of `EditionOut.contributors` (checked against `ContributorOut`). Fields given as string literals are compared with the document's enum values, as with `source: "openlibrary"|…` against `CatalogSource`. Literals are compared only when the document has an enum: `"YYYY-MM-DD"` is a format, and `ClubSummary.role` is a documented string.
- `section_0_enums_match_the_document`: `ShelfStatus` and `Visibility` have exactly the §0 values.
- `every_schema_conventions_14_registers_is_in_the_document`: the `components(schemas(…))` list of conventions.md §14 is in the document. That list also names `ContributorOut`, `CatalogSource` and `ErrorBody`, which §0 only describes inline.
- `schema_types_never_skip_a_field`: the guard above.
- `the_cli_prints_the_same_bytes_every_time`: four runs of `bookclub-server openapi` print identical bytes, and the output equals `openapi().to_pretty_json()` built in the test process. A map with a random order in the document would otherwise make `check:api` fail at random.

The parser is strict. A line in column 0 of the §0 block that is not `Name {…}`, `Name (note) {…}` or `Name = "a"|"b"` fails the test instead of being skipped. It also asserts that it found at least 11 DTOs, among them `BookOut`, `EditionOut`, `UserOut`, `ShelfStatus` and `ReactionOut`, so it cannot pass vacuously.

I checked by mutation that the tests catch drift. Each of these failed with a message naming the difference, and every change was reverted:
- renaming `ReactionOut` in §0
- dropping `spine_color` from §0 `BookOut`
- renaming `role` in `contributors`
- dropping a `ShelfStatus` value
- dropping `"legacy"` from `BookOut.source`
- a stray line in the block
- adding `skip_serializing_if` to `BookOut` (whereas the same attribute on the non-schema `UserSummaryRow` passed)

I also checked `check:api` the same way: changing a DTO doc comment in `dto.rs` made it print the diff and exit 1.

## Deviations from the work plan

1. **No `openapi.json` on disk.** The plan's `cargo run … -- openapi > openapi.json` would leave an untracked file in `frontend/`, and OAPI does not own `.gitignore`. The script keeps the document in memory. Its only output is `generated.ts`.
2. **`check:api` runs the git diff itself** (`node scripts/gen-api-types.mjs --check`), not as `npm run gen:api && git diff --exit-code`. It is the same check. It also fails if `generated.ts` is not tracked (`git ls-files --error-unmatch`), which a bare `git diff` would miss, and it prints a hint naming the fix.
3. **`cargo run` gets `--locked`**, as every other cargo command in CI does.
4. **Response fields are required** in the generated types (the section above). The document itself is unchanged: `GET /api/openapi.json` still marks `Option` fields optional, as utoipa emits them.
5. **Root type aliases.** openapi-typescript runs with `rootTypes` and `rootTypesNoSchemaPrefix`, so FE can `import type { BookOut } from "@/api/generated"`. Without them, FE would write `components["schemas"]["BookOut"]`. The names are the schema names, as conventions.md §14 intends.

## New detail strings

None.

## Python tests

No Python test belongs to OAPI. The OpenAPI-related assertions of the legacy suite (`/api/docs` is 404 in `test_selfhost.py`) are W1's and are already reasoned in parity-notes.md.

## For the integrator

- **After each merge:** run `cd frontend && npm run gen:api` and commit `src/api/generated.ts`, as work-plan ground rule 7 says.
  - CI's `check:api` fails until you do.
  - The script needs the server's `.sqlx/` to be current (`SQLX_OFFLINE=true`), which ground rule 7 step 1 already ensures.
  - On the agents VM the generation takes seconds once `server/target` is built.
- **`npm ci` before the first `gen:api`** in a checkout, so that `openapi-typescript` is installed.
- **Merging with another branch that touched `generated.ts`:** resolve the conflict by regenerating, never by hand.

## For FE (#48)

- **Imports:** `import type { BookOut, ShelfItemOut } from "@/api/generated"`. Every schema has an alias of its own name. Request and response shapes per route are under `paths["/api/…"]["get"]` and `operations["<operation_id>"]`.
- **Optional fields:** response fields are never optional (`x: T | null`). Request bodies keep `x?: T | null` where the server accepts an absent key.
- **Integers:** `int64` fields are `number`, as in `types.ts` today.
- **Never edit `generated.ts`.** CI regenerates it and fails when it is stale. To change a type, change the Rust DTO and run `npm run gen:api`.
- **`types.ts`** still stands alongside. Replacing it alias by alias is FE's job (work-plan FE).

## For PAR (#49)

Nothing to check off. OAPI ports no Python tests.
