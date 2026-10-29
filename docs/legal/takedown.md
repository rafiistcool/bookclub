# Notices about content: intake, decision and removal

How the operator handles notices that content on the service is illegal, infringes someone's rights or breaks the terms: notice and action under Art. 16 and 17 DSA, and the operator's own moderation. It is built from the code; "to confirm" marks Rafael's open decisions, and "planned" marks the admin and reader screens ADM2 (#54, #55) adds in its second part. Have #53's legal source confirm the DSA wording (work plan Q12).

`server/tests/legal_docs.rs` checks that every endpoint this file names exists in the server's OpenAPI document, or is in its list of planned endpoints, and that every app file it names exists.

## 1. Targets (to confirm, Q12)

| Step | Target (recommended option A) |
|---|---|
| Acknowledge a notice received by mail | within 24 hours |
| Decide a notice | within 72 hours of receipt |
| Tell the reporter and the uploader | when deciding: the app mails both after the decision (`server/src/admin/notices.rs` (`decided`)) |

The admin page flags items open longer than 72 hours (default D4). The 90-day statistics (median and maximum hours to a decision, count over the target) come from `GET /api/admin/summary`; reports closed on arrival (the picture was down already) are left out.

## 2. How notices come in

1. **In the app.** A signed-in user reports:
   - a cover, from the book page: "Report cover" (`frontend/src/components/ReportCoverSheet.vue`) calls `POST /api/media/{public_id}/reports`;
   - a profile picture, through the same endpoint with the picture's public id, when the reporter can see its owner (they share a club; otherwise the answer is 404 like an unknown picture);
   - a book or an edition a reader added (never one from Open Library, the DNB or the old app: `server/src/catalog/reports.rs` (`ONLY_READER_BOOKS`)), through `POST /api/books/works/{work_id}/reports` and `POST /api/books/editions/{edition_id}/reports` (`server/src/catalog/reports.rs` (`report_work`, `report_edition`)).

   Each report has a reason (`copyright`, `wrong_book`, `offensive`, `other`) and details (at most 2,000 characters; 20 new reports a day per account, shared by all kinds; `server/src/media/reports.rs` (`report`, `ReportIn`)). A repeat by the same reader answers with their open report. The report lands in the admin queue. Planned (ADM2, second part): the "Report" action on a member's shelf (profile picture) and on books readers added.
2. **By mail.** Anyone, with or without an account, writes to `CONTACT_EMAIL`, the one public contact and takedown address (default D3). The app shows it in `GET /api/config` (`contact_email`), in the legal section of Settings, and in the imprint. Planned (ADM2, second part): the report sheet adds "No account? Write to {contact_email}".
3. **Mail into the queue.** There is no public unauthenticated form (default D2). An instance admin enters a mailed notice about a picture, a cover or a profile picture, with the "Take down a picture by URL" form on the admin page (`frontend/src/components/admin/AdminTakeDown.vue`), which calls `DELETE /api/admin/media/{public_id}` with the optional body `{details, received_at, reason}`. That takes the picture down and records a resolved report that came in by mail (`via` `notice`), with the time the mail arrived. A mailed notice about a book or edition a reader added: the admin files a report of it in the app with the notice in its details (the arrival time too), then decides it; the mail itself goes into the record (§6).

Each notice needs, per Art. 16 (2) DSA: why the content is illegal, where it is (the URL), the name and address of the reporter (except for some offences against children), and a statement of good faith. A notice that lacks them is answered with a request for what is missing.

## 3. Triage

| Reason | Typical content | First question |
|---|---|---|
| Copyright (`copyright`) | a cover photo, a long quote, a pasted review or blurb | Is the reporter the rights holder or acting for them? Is the use covered (cover thumbnails of the book itself, short quotes, § 51 UrhG)? |
| Wrong book (`wrong_book`) | a cover or book data that belongs to another book | A catalog error, not illegal content: fix the record ("Edit as admin"), merge a duplicate, or remove the cover |
| Offensive (`offensive`) | insults, harassment, hate, pictures of people | Is it illegal (for example §§ 130, 185–187 StGB, § 201a StGB, § 22 KUG) or only against the terms? |
| Other (`other`) | personal data of others, spam, anything else | Which law or which clause of the terms does it break? |

Content inside a club is visible to its members only, and personal content to the author's audience (`privacy-facts.md` §10). Covers and books are visible to every signed-in user, and a profile picture to everyone who shares a club with its owner, so most notices will be about them.

## 4. Removal per kind of content

| Content | Who can remove it today | How |
|---|---|---|
| Cover photo a user uploaded | instance admin | From the report: "Remove" in the reports tab of the admin page (`frontend/src/components/admin/AdminReports.vue`), which calls `POST /api/admin/reports/{report_id}/resolve` with `{"action": "remove"}`. Without a report: `DELETE /api/admin/media/{public_id}`. Either way the files are deleted, the hashes of the original and every variant are blocked so the same picture cannot come back, every book that showed it loses the cover, the picture's open reports are resolved, and `/media/{public_id}/…` answers 410 (`server/src/media/reports.rs` (`take_down_locked`, `record_removed_hashes`)). A browser that loaded the picture before can still show its cached copy for up to a year (`server/src/media/serve.rs` (`IMMUTABLE`), `privacy-facts.md` §2); the takedown cannot reach it. |
| Cover fetched from Open Library | instance admin | The same. The hash block also stops the server from fetching it again. |
| Profile picture | instance admin; the user | From the report: "Remove", as for a cover. Without a report: `DELETE /api/admin/media/{public_id}` with the `public_id` from the picture's URL (`/media/{public_id}/a`), with or without a notice. The picture then shows nowhere (every place that shows a profile picture skips a removed one), `/media/{public_id}/a` answers 410, and the same file cannot be uploaded again. The user removes their own with `DELETE /api/auth/me/avatar`. |
| Book or edition a reader added (title, authors, description, details) | instance admin | From the report: "Remove" withdraws what the reader typed and keeps the record, so the shelves, posts and picks that point at it keep working: a book gets the title `[removed]` and loses its subtitle, authors, description and subjects; an edition loses its own title, subtitle and publisher and shows its book's title (`server/src/catalog/reports.rs` (`decide`, `REMOVED_TITLE`)). The fields get provenance `admin`, so no source fills them again. To correct rather than remove: "Edit as admin" (`frontend/src/components/EditRequestSheet.vue`), `PATCH /api/admin/works/{work_id}` and `PATCH /api/admin/editions/{edition_id}`; a reader's edit request approved with `POST /api/admin/edit-requests/{request_id}/approve`; a duplicate merged away with `POST /api/admin/works/{work_id}/merge` (`frontend/src/components/admin/AdminRequests.vue`). The book's cover is a picture of its own: report and remove it as above. |
| Description from Open Library | instance admin | Q18 option A: `PATCH /api/admin/works/{work_id}` clears or replaces it, and a later refresh leaves the admin's value alone. |
| Club post or reply | the club's admins and owner; the author | The delete action on the post, `DELETE /api/posts/{post_id}` (`server/src/posts/edit.rs:178-190`). A post others replied to becomes a tombstone without text. The instance admin has no in-app way (Q11 option C adds reports on posts with #57); until then, the operator asks the club's admins, or deletes the text by hand in the database (to confirm the procedure). |
| Personal diary entry, quote, rating take | the author | `DELETE /api/posts/{post_id}`, `DELETE /api/quotes/{quote_id}`, `PATCH /api/shelf/{entry_id}`. The instance admin has no in-app way (as above). |
| Club name | the club's owner and admins | `PATCH /api/clubs/{slug}`. The instance admin: by hand (to confirm). |
| Username | nobody in the app | Usernames cannot be changed. The operator renames by hand, or suspends the account (`POST /api/admin/users/{username}/suspend`). |
| Account | the user; the operator | The user deletes it in Settings (`frontend/src/components/DeleteAccount.vue`, `DELETE /api/auth/me`). The instance admin suspends it (Q15): `POST /api/admin/users/{username}/suspend`, undone with `POST /api/admin/users/{username}/unsuspend`, and `GET /api/admin/users/suspended` lists the suspended accounts (`server/src/admin/users.rs` (`suspend`, `unsuspend`, `list_suspended`)). A suspended account cannot sign in (login answers "This account is suspended" once the password matched), every session and app token it had ends at once, and its content stays. An admin cannot suspend their own account. Planned (ADM2, second part): the accounts section on the admin page. |

"By hand" means the operator changes the database directly. Every such change is written down in the record (§6).

## 5. Decision, statement of reasons, complaints

- **Decision.** The admin decides "remove" or "dismiss" (`POST /api/admin/reports/{report_id}/resolve`). Either way every open report of the same picture, book or edition is resolved with it.
- **Reporter.** Gets the decision (Art. 16 (5) DSA): after the decision commits, the app mails each reporter of the resolved reports whether the content was removed or kept, in their language, to their verified address (`server/src/email/templates.rs` (`report_decision`)). The mail names nothing about the uploader. A kept report tells the reporter where to object (`CONTACT_EMAIL`). A decision that fails or is rolled back sends nothing. An account without a verified address gets no mail; notices by mail are answered by hand, from the contact mailbox.
- **Uploader.** Gets a statement of reasons when their content is removed (Art. 17 DSA): the uploader of a cover, the owner of a profile picture, the reader who added a book or edition (`server/src/email/templates.rs` (`statement_of_reasons`)). It says what was removed and what changes, the ground (the reason the report or notice gave, or that an admin found it unlawful or against the terms when it was removed without one), that a person decided and no automated system, how to object (`CONTACT_EMAIL`), and links the terms. It never names the reporter or quotes the report. A dismissal, or a "remove" of something removed before, sends no statement. Covers from Open Library have no uploader. To a verified address only; for an account without one, the operator decides whether to write by hand.
- **Objection.** The uploader or the reporter writes to `CONTACT_EMAIL`; the operator reviews the decision and answers within the same target (to confirm). Whether the internal complaint system of Art. 20 DSA applies (it applies to online platforms, and content inside closed clubs is not disseminated to the public) is for the lawyer to confirm.
- **Repeat abuse.** Suspension (Q15, §4) after a warning, per the terms (Art. 23 DSA, if it applies; to confirm). The app sends no mail on a suspension: the operator writes the statement of reasons for it by hand from the contact mailbox (Art. 17 DSA also covers suspensions; to confirm).

## 6. Records

- **In the database.** Every report keeps what was reported (a cover, a profile picture with whose it was, a book or an edition), the reason, details, reporter, how it came in (`via`: `app` or `notice`), the time, the deciding admin, the time and the outcome, and whether it was closed on arrival (`reports`, `server/migrations/0001_init.sql`, `server/migrations/2101_reports_targets.sql`). A takedown also sets `media.removed_at` and keeps the blocked hashes (`media_removed_hashes`). A direct takedown with a body records a resolved report with the time the notice arrived. Edit requests keep the deciding admin, the time and the note (`catalog_edit_requests`). A suspension keeps its time (`users.suspended_at`, `server/migrations/2102_users_suspended_at.sql`); who suspended and why go into the mailbox record.
- **In the mailbox.** Notices received by mail, acknowledgements, decisions sent by hand, objections and answers, suspensions with their reasons, and every change made by hand in the database (what, when, why). Keep them in one folder of the contact mailbox.
- **Retention.** 3 years after the decision (to confirm, Q14; LEG2 deletes resolved reports and decided requests after the period).
- **Export.** Reports a user filed and resolved, and the reports of their own content (their profile picture, pictures they uploaded, books and editions they added; without who filed them or what they wrote), are part of their export (`reports` section with `filed`, `resolved` and `about_me`, `docs/phase1/conventions.md` §5.7).

## 7. Accounts that cannot sign in

A suspended account loses the in-app export and deletion (DSGVO Art. 15, 17 and 20). Its owner writes to `CONTACT_EMAIL`. The operator:

1. Checks that the request comes from the account's verified address: in `psql` on the instance (`docker compose exec postgres psql -U bookclub bookclub`), `SELECT username, email, email_verified_at, suspended_at FROM users WHERE username = '<name>';`. A request from another address is answered with a request to write from that one.
2. **Export.** Downloads it for them, so the account is never usable in between:
   1. Unsuspend it: `POST /api/admin/users/{username}/unsuspend` (the account still has no session: the suspension ended them all).
   2. In `psql`, make a one-off app token that lives ten minutes and print it:
      ```sql
      WITH s AS (SELECT decode(replace(gen_random_uuid()::text || gen_random_uuid()::text, '-', ''), 'hex') AS secret)
      INSERT INTO sessions (user_id, kind, token_hash, label, expires_at)
      SELECT u.id, 'bearer', sha256(s.secret), 'operator export', now() + interval '10 minutes'
      FROM users u, s WHERE u.username = '<name>'
      RETURNING 'bc_' || rtrim(translate(encode((SELECT secret FROM s), 'base64'), '+/', '-_'), '=') AS token;
      ```
   3. `curl -fsS -H "Authorization: Bearer <token>" -o export.zip https://<host>/api/auth/me/export`
   4. Suspend it again: `POST /api/admin/users/{username}/suspend`, which deletes the token.
   5. Send `export.zip` to the verified address (as an encrypted archive where possible), delete the local copy, and note the request and the answer in the record (§6).
3. **Deletion.** The deletion asks for the account's password, which only its owner knows: unsuspend the account, tell the owner (at the verified address) to delete it in Settings within 48 hours, and suspend it again if it still exists then. An owner of clubs first transfers or deletes them (the deletion answers "Transfer or delete your clubs first"). Note it in the record.
