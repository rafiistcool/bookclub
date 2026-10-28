# Notices about content: intake, decision and removal

How the operator handles notices that content on the service is illegal, infringes someone's rights or breaks the terms: notice and action under Art. 16 and 17 DSA, and the operator's own moderation. It is built from the code; "to confirm" marks Rafael's open decisions, and "planned" marks what ADM (#54) and ADM2 (#54, #55) build next. Have #53's legal source confirm the DSA wording (work plan Q12).

`server/tests/legal_docs.rs` checks that every endpoint this file names exists in the server's OpenAPI document, or is in its list of planned ADM and ADM2 endpoints, and that every app file it names exists.

## 1. Targets (to confirm, Q12)

| Step | Target (recommended option A) |
|---|---|
| Acknowledge a notice received by mail | within 24 hours |
| Decide a notice | within 72 hours of receipt |
| Tell the reporter and the uploader | when deciding; the app sends the mails (planned, ADM2) |

The admin page flags items open longer than 72 hours (default D4, planned in ADM). The 90-day statistics (median and maximum hours to a decision, count over the target) come from `GET /api/admin/summary` (planned, ADM).

## 2. How notices come in

1. **In the app.** A signed-in user reports a cover from the book page: "Report cover" (`frontend/src/components/ReportCoverSheet.vue`) calls `POST /api/media/{public_id}/reports` with a reason (`copyright`, `wrong_book`, `offensive`, `other`) and details (at most 2,000 characters; 20 reports a day per account; `server/src/media/reports.rs:120-200`). The report lands in the admin queue. Planned (ADM2, Q11 option B): reports on profile pictures through the same endpoint, and on books and editions readers added through `POST /api/books/works/{work_id}/reports` and `POST /api/books/editions/{edition_id}/reports`.
2. **By mail.** Anyone, with or without an account, writes to `CONTACT_EMAIL`, the one public contact and takedown address (default D3). The app shows it in `GET /api/config` (`contact_email`), in the legal section of Settings, and in the imprint. Planned (ADM2): the report sheet adds "No account? Write to {contact_email}".
3. **Mail into the queue.** There is no public unauthenticated form (default D2). An instance admin enters a mailed notice about a picture with the "Take down a picture by URL" form on the admin page (planned, ADM), which calls `DELETE /api/admin/media/{public_id}` with the optional body `{details, received_at}` and so records a resolved report with the time the mail arrived. Notices about anything else are recorded in the mailbox (see §6).

Each notice needs, per Art. 16 (2) DSA: why the content is illegal, where it is (the URL), the name and address of the reporter (except for some offences against children), and a statement of good faith. A notice that lacks them is answered with a request for what is missing.

## 3. Triage

| Reason | Typical content | First question |
|---|---|---|
| Copyright (`copyright`) | a cover photo, a long quote, a pasted review | Is the reporter the rights holder or acting for them? Is the use covered (cover thumbnails of the book itself, short quotes, § 51 UrhG)? |
| Wrong book (`wrong_book`) | a cover or book data that belongs to another book | A catalog error, not illegal content: fix the record or remove the cover, no statement of reasons needed beyond the admin note |
| Offensive (`offensive`) | insults, harassment, hate, pictures of people | Is it illegal (for example §§ 130, 185–187 StGB, § 201a StGB, § 22 KUG) or only against the terms? |
| Other (`other`) | personal data of others, spam, anything else | Which law or which clause of the terms does it break? |

Content inside a club is visible to its members only, and personal content to the author's audience (`privacy-facts.md` §10). Pictures and books are visible to every signed-in user, so most notices will be about them.

## 4. Removal per kind of content

| Content | Who can remove it today | How |
|---|---|---|
| Cover photo a user uploaded | instance admin | From the report: "Remove" in the reports tab of the admin page (`frontend/src/components/admin/AdminReports.vue`), which calls `POST /api/admin/reports/{report_id}/resolve` with `{"action": "remove"}`. Without a report: `DELETE /api/admin/media/{public_id}`. Either way the files are deleted, the hashes of the original and every variant are blocked so the same picture cannot come back, every book that showed it loses the cover, the picture's open reports are resolved, and `/media/{public_id}/…` answers 410 (`server/src/media/reports.rs:333-425`). |
| Cover fetched from Open Library | instance admin | The same. The hash block also stops the server from fetching it again. |
| Profile picture | instance admin; the user | `DELETE /api/admin/media/{public_id}` with the `public_id` from the picture's URL (`/media/{public_id}/a`), which works for every picture. The user removes their own with `DELETE /api/auth/me/avatar`. Planned (ADM2): reports on profile pictures, then "Remove" from the queue. |
| Book or edition a reader added (title, authors, description, details) | instance admin | Planned (ADM): "Edit as admin" on the book page and in the queue, `PATCH /api/admin/works/{work_id}` and `PATCH /api/admin/editions/{edition_id}`. Today: a reader's edit request approved with `POST /api/admin/edit-requests/{request_id}/approve`, or a duplicate merged away with `POST /api/admin/works/{work_id}/merge` (`frontend/src/components/admin/AdminRequests.vue`). |
| Description from Open Library | instance admin | Planned (ADM, Q18 option A): `PATCH /api/admin/works/{work_id}` clears or replaces it, and a later refresh leaves the admin's value alone. |
| Club post or reply | the club's admins and owner; the author | The delete action on the post, `DELETE /api/posts/{post_id}` (`server/src/posts/edit.rs:178-190`). A post others replied to becomes a tombstone without text. The instance admin has no in-app way (Q11 option C adds reports on posts with #57); until then, the operator asks the club's admins, or deletes the text by hand in the database (to confirm the procedure). |
| Personal diary entry, quote, rating take | the author | `DELETE /api/posts/{post_id}`, `DELETE /api/quotes/{quote_id}`, `PATCH /api/shelf/{entry_id}`. The instance admin has no in-app way (as above). |
| Club name | the club's owner and admins | `PATCH /api/clubs/{slug}`. The instance admin: by hand (to confirm). |
| Username | nobody in the app | Usernames cannot be changed. The operator renames by hand, or suspends the account (planned, ADM2: `POST /api/admin/users/{username}/suspend`). |
| Account | the user; the operator | The user deletes it in Settings (`frontend/src/components/DeleteAccount.vue`, `DELETE /api/auth/me`). Planned (ADM2, Q15): the instance admin suspends it (`POST /api/admin/users/{username}/suspend`, undone with `POST /api/admin/users/{username}/unsuspend`): login refused, sessions ended, content stays. |

"By hand" means the operator changes the database directly. Every such change is written down in the record (§6).

## 5. Decision, statement of reasons, complaints

- **Decision.** The admin decides "remove" or "dismiss" (`POST /api/admin/reports/{report_id}/resolve`). "Dismiss" resolves every open report of the picture (planned fix, ADM).
- **Reporter.** Gets the decision (Art. 16 (5) DSA). Planned (ADM2, Q12 option A): the app mails it after the commit, in the reporter's language. Until then, and for notices by mail: by hand, from the contact mailbox.
- **Uploader.** Gets a statement of reasons when their content is removed or restricted (Art. 17 DSA): what was removed, why (the law or the clause of the terms), whether it was found by a notice, and how to object. Planned (ADM2): the app mails it to the uploader or the owner of the profile picture.
- **Objection.** The uploader or the reporter writes to `CONTACT_EMAIL`; the operator reviews the decision and answers within the same target (to confirm). Whether the internal complaint system of Art. 20 DSA applies (it applies to online platforms, and content inside closed clubs is not disseminated to the public) is for the lawyer to confirm.
- **Repeat abuse.** Suspension (planned, ADM2, Q15) after a warning, per the terms (Art. 23 DSA, if it applies; to confirm).

## 6. Records

- **In the database.** Every report keeps the target, reason, details, reporter, time, the deciding admin, the time and the outcome (`reports`, `server/migrations/0001_init.sql`). A takedown also sets `media.removed_at` and keeps the blocked hashes (`media_removed_hashes`). A direct takedown with a body records a resolved report with the time the notice arrived (planned, ADM). Edit requests keep the deciding admin, the time and the note (`catalog_edit_requests`).
- **In the mailbox.** Notices received by mail, acknowledgements, decisions sent by hand, objections and answers, and every change made by hand in the database (what, when, why). Keep them in one folder of the contact mailbox.
- **Retention.** 3 years after the decision (to confirm, Q14; LEG2 deletes resolved reports and decided requests after the period).
- **Export.** Reports a user filed and resolved are part of their export (`reports` section, `docs/phase1/conventions.md` §5.7).

## 7. Accounts that cannot sign in

A suspended account (planned, ADM2) loses the in-app export and deletion. Its owner writes to `CONTACT_EMAIL`; the operator checks that the request comes from the account's verified address and then runs the export or the deletion for them. ADM2 adds the exact steps here (work plan §0 item 20).
