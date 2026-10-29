import { afterEach, describe, expect, it } from "vitest";
import type { MissingCoverOut } from "./api/generated";
import {
  coverCandidateUrl,
  coverFound,
  coverReasonLabel,
  coverSearchMessage,
  coverStatusLabel,
  formatAge,
  fromLocalInput,
  hoursBetween,
  isOverdue,
  mergeTarget,
  missingCoverNotes,
  nextQueuePage,
  pictureRef,
  proposedFields,
  requestKindLabel,
  toLocalInput,
} from "./admin";
import { i18n } from "./i18n";
import { languageName, languageOptions } from "./languages";

const request = (kind: string, payload: Record<string, unknown>) => ({
  kind,
  payload: payload as Record<string, never>,
});

describe("the admin queue", () => {
  afterEach(() => {
    i18n.global.locale.value = "en";
  });

  it("labels each proposed field and marks a cleared one", () => {
    const fields = proposedFields(
      request("edition_edit", { pages: 401, format: "paperback", language: "ger", subtitle: null, subjects: ["A", "B"] }),
      "en",
    );
    const unknown = { before: null, wasEmpty: false, same: false };
    expect(fields).toEqual([
      { key: "pages", label: "Pages", value: "401", cleared: false, ...unknown },
      { key: "format", label: "Format", value: "Paperback", cleared: false, ...unknown },
      { key: "language", label: "Language", value: "German", cleared: false, ...unknown },
      { key: "subtitle", label: "Subtitle", value: "(clear it)", cleared: true, ...unknown },
      { key: "subjects", label: "Subjects", value: "A, B", cleared: false, ...unknown },
    ]);
    expect(proposedFields(request("cover_replace", {}), "en")).toEqual([]);
  });

  it("puts the record's value now beside each proposed one", () => {
    const fields = proposedFields(
      {
        ...request("edition_edit", { pages: 401, format: "paperback", language: "ger", subtitle: null, publisher: "Klett" }),
        current: { pages: 380, format: null, language: "eng", subtitle: "Roman", publisher: "Klett" } as never,
      },
      "en",
    );
    expect(fields.map(({ key, before, wasEmpty, same }) => ({ key, before, wasEmpty, same }))).toEqual([
      { key: "pages", before: "380", wasEmpty: false, same: false },
      { key: "format", before: "(empty)", wasEmpty: true, same: false },
      { key: "language", before: "English", wasEmpty: false, same: false },
      { key: "subtitle", before: "Roman", wasEmpty: false, same: false },
      { key: "publisher", before: "Klett", wasEmpty: false, same: true },
    ]);
    i18n.global.locale.value = "de";
    const [subjects] = proposedFields(
      { ...request("work_edit", { subjects: [] }), current: { subjects: [] } as never },
      "de",
    );
    expect(subjects).toMatchObject({ value: "(leeren)", before: "(leer)", wasEmpty: true, same: true });
  });

  it("names the request kinds in both languages, and the merge target", () => {
    expect(requestKindLabel("work_merge")).toBe("Same book");
    i18n.global.locale.value = "de";
    expect(requestKindLabel("cover_replace")).toBe("Neues Cover-Foto");
    expect(requestKindLabel("something_new")).toBe("something_new");
    expect(mergeTarget(request("work_merge", { into_work_id: 31 }))).toBe(31);
    expect(mergeTarget(request("work_edit", { into_work_id: 31 }))).toBeNull();
  });

  it("names languages by their catalog code in the UI language", () => {
    expect(languageName("ger", "de")).toBe("Deutsch");
    expect(languageName("fre", "en")).toBe("French");
    expect(languageName("xyz", "en")).toBe("xyz");
    const options = languageOptions("en", "xyz");
    expect(options.map((row) => row.code)).toContain("xyz");
    expect(options[0]!.name.localeCompare(options[1]!.name)).toBeLessThan(0);
  });

  it("reads a picture's address, a bare picture id or a book page", () => {
    const id = "abcdefghijklmnopqrstuvwxyz";
    expect(pictureRef(`https://books.example.com/media/${id}/l`)).toEqual({ kind: "media", publicId: id });
    expect(pictureRef(`/media/${id}/s?v=2`)).toEqual({ kind: "media", publicId: id });
    expect(pictureRef(`  ${id}  `)).toEqual({ kind: "media", publicId: id });
    expect(pictureRef(`https://books.example.com/media/${id}/a`)).toEqual({ kind: "media", publicId: id, avatar: true });
    expect(pictureRef(`/media/${id}`)).toEqual({ kind: "media", publicId: id });
    expect(pictureRef("https://books.example.com/book/42")).toEqual({ kind: "book", book: 42 });
    expect(pictureRef("https://books.example.com/book/OL1W/")).toEqual({ kind: "book", book: "OL1W" });
    expect(pictureRef("https://books.example.com/book/42/notes")).toBeNull();
    expect(pictureRef("https://books.example.com/media/short/l")).toBeNull();
    expect(pictureRef(`https://books.example.com/media/${id.toUpperCase()}/l`)).toBeNull();
    expect(pictureRef("https://books.example.com/club/readers")).toBeNull();
    expect(pictureRef("")).toBeNull();
    expect(pictureRef("not a link")).toBeNull();
  });

  it("tells ages in hours below 72, then in days, in both languages", () => {
    expect(formatAge(0.5)).toBe("less than an hour");
    expect(formatAge(1.2)).toBe("an hour");
    expect(formatAge(71.9)).toBe("71 hours");
    expect(formatAge(72)).toBe("3 days");
    i18n.global.locale.value = "de";
    expect(formatAge(0)).toBe("weniger als einer Stunde");
    expect(formatAge(1)).toBe("einer Stunde");
    expect(formatAge(5)).toBe("5 Stunden");
    expect(formatAge(100)).toBe("4 Tagen");
  });

  it("measures waits and flags an item once it reaches the target", () => {
    const now = Date.parse("2026-10-28T12:00:00Z");
    expect(hoursBetween("2026-10-28T09:30:00Z", now)).toBe(2.5);
    expect(hoursBetween("2026-10-26T12:00:00Z", "2026-10-28T12:00:00Z")).toBe(48);
    expect(hoursBetween("not a time", now)).toBe(0);
    expect(hoursBetween("2026-10-29T12:00:00Z", now)).toBe(0);
    expect(isOverdue("2026-10-25T12:00:01Z", 72, now)).toBe(false);
    expect(isOverdue("2026-10-25T12:00:00Z", 72, now)).toBe(true);
  });

  it("asks for the first page not yet complete, so a decision skips nothing", () => {
    expect(nextQueuePage(50)).toBe(2);
    expect(nextQueuePage(49)).toBe(1);
    expect(nextQueuePage(100)).toBe(3);
  });

  it("turns a datetime-local value into an instant and back", () => {
    const date = new Date(2026, 9, 28, 9, 5);
    expect(toLocalInput(date)).toBe("2026-10-28T09:05");
    expect(fromLocalInput("2026-10-28T09:05")).toBe(date.toISOString());
    expect(fromLocalInput("")).toBeNull();
    expect(fromLocalInput("soon")).toBeNull();
  });
});

describe("the cover search for an admin", () => {
  afterEach(() => {
    i18n.global.locale.value = "en";
  });

  type Notes = Parameters<typeof missingCoverNotes>[0];
  const NOW = Date.parse("2026-10-29T12:00:00Z");
  const day = (iso: string) => iso.slice(0, 10);
  const row = (overrides: Partial<MissingCoverOut> = {}): Notes => ({
    state: "none",
    failing_edition_id: null,
    failed_at: null,
    searched_at: null,
    outcome: null,
    next_search_at: null,
    failures: 0,
    after_takedown: false,
    suggestion_id: null,
    ...overrides,
  });

  it("says why a candidate waits for an admin, in both languages", () => {
    expect(coverReasonLabel("number_differs")).toBe("A number in the title differs: maybe another volume.");
    expect(coverReasonLabel("after_takedown")).toBe("This book's cover was taken down, so a person decides from now on.");
    expect(coverReasonLabel("something_new")).toBe("something_new");
    expect(coverStatusLabel("approved")).toBe("Used");
    expect(coverStatusLabel("rejected")).toBe("Not this book");
    expect(coverStatusLabel("later")).toBe("later");
    i18n.global.locale.value = "de";
    expect(coverReasonLabel("several_works")).toBe("Open Library hat mehr als ein Buch mit diesem Titel von dieser Person.");
    expect(coverStatusLabel("closed")).toBe("Geschlossen");
  });

  it("links the candidate's edition, else its work, and nothing that is not an Open Library id", () => {
    expect(coverCandidateUrl({ ol_work_id: "OL16809803W", ol_edition_id: "OL27198433M" })).toBe(
      "https://openlibrary.org/books/OL27198433M",
    );
    expect(coverCandidateUrl({ ol_work_id: "OL16809803W", ol_edition_id: null })).toBe(
      "https://openlibrary.org/works/OL16809803W",
    );
    expect(coverCandidateUrl({ ol_work_id: "OL1W", ol_edition_id: "OL2W" })).toBe("https://openlibrary.org/works/OL1W");
    expect(coverCandidateUrl({ ol_work_id: "javascript:alert(1)", ol_edition_id: "//evil.example" })).toBeNull();
  });

  it("words what Search now came to, and which outcomes gave the book a cover", () => {
    expect(coverSearchMessage("suggested")).toBe("Found a possible cover. It waits under Suggestions.");
    expect(coverSearchMessage("no_match")).toBe("No match on Open Library.");
    expect(coverSearchMessage("whatever")).toBe("No match on Open Library.");
    expect(coverSearchMessage("failing")).toBe("Open Library's picture still does not load. Upload a picture instead.");
    expect(["attached", "inherited", "isbn", "suggested", "no_match", "failing"].filter(coverFound)).toEqual([
      "attached",
      "inherited",
      "isbn",
    ]);
  });

  it("tells a book that was never searched, and one Open Library had no match for", () => {
    expect(missingCoverNotes(row(), day, NOW)).toEqual(["Not searched yet."]);
    expect(
      missingCoverNotes(
        row({ searched_at: "2026-10-20T08:00:00Z", outcome: "no_match", next_search_at: "2026-11-19T08:00:00Z" }),
        day,
        NOW,
      ),
    ).toEqual(["No match on Open Library on 2026-10-20.", "Next try on 2026-11-19."]);
    // Uncertain and blocked read as no match; a past retry date is no news.
    expect(
      missingCoverNotes(row({ searched_at: "2026-09-01T08:00:00Z", outcome: "uncertain", next_search_at: "2026-10-01T08:00:00Z" }), day, NOW),
    ).toEqual(["No match on Open Library on 2026-09-01."]);
    expect(missingCoverNotes(row({ searched_at: "2026-09-01T08:00:00Z", outcome: "attached" }), day, NOW)).toEqual([
      "Last searched on 2026-09-01.",
    ]);
  });

  it("names a waiting suggestion or photo, a failing picture, outages and a takedown", () => {
    expect(missingCoverNotes(row({ suggestion_id: 4, searched_at: "2026-10-20T08:00:00Z", outcome: "suggested" }), day, NOW)).toEqual([
      "A suggestion waits under Suggestions.",
    ]);
    expect(missingCoverNotes(row({ state: "photo_waiting" }), day, NOW)).toEqual([
      "A reader's photo waits in the Requests tab.",
      "Not searched yet.",
    ]);
    expect(missingCoverNotes(row({ state: "failing", failed_at: "2026-10-28T03:00:00Z" }), day, NOW)[0]).toBe(
      "Open Library's picture failed to load on 2026-10-28.",
    );
    expect(missingCoverNotes(row({ state: "failing", failing_edition_id: 13, failed_at: null }), day, NOW)[0]).toBe(
      "Open Library's picture of edition 13 failed to load.",
    );
    expect(
      missingCoverNotes(row({ failures: 3, next_search_at: "2026-10-29T16:00:00Z", after_takedown: true }), day, NOW),
    ).toEqual([
      "Open Library did not answer the last 3 searches.",
      "Next try on 2026-10-29.",
      "After a takedown, the search only suggests.",
    ]);
    i18n.global.locale.value = "de";
    expect(missingCoverNotes(row({ failures: 1 }), day, NOW)).toEqual([
      "Open Library hat bei der letzten Suche nicht geantwortet.",
    ]);
    expect(
      missingCoverNotes(row({ state: "failing", failing_edition_id: 13, failed_at: "2026-10-28T03:00:00Z" }), day, NOW)[0],
    ).toBe("Das Bild von Open Library zu Ausgabe 13 ließ sich am 2026-10-28 nicht laden.");
  });
});
