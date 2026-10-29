import type { CoverSuggestionOut, EditRequestOut, MissingCoverOut } from "./api/generated";
import { parseBookParam } from "./constants";
import { EDITION_FORMATS } from "./editions";
import { t, tp } from "./i18n";
import { languageName } from "./languages";

/**
 * Reading the admin queue. `EditRequestOut.payload` is an object without a
 * schema: the proposed fields for an edit, `{into_work_id}` for a merge,
 * `{}` for a cover photo. `EditRequestOut.current` holds the record's value
 * now under the same keys.
 */
export const REQUEST_KINDS = ["work_edit", "edition_edit", "cover_replace", "work_merge"] as const;

export function requestKindLabel(kind: string): string {
  return (REQUEST_KINDS as readonly string[]).includes(kind) ? t(`admin.kind.${kind}`) : kind;
}

const FIELD_KEYS = [
  "title",
  "subtitle",
  "authors",
  "year",
  "pages",
  "description",
  "subjects",
  "publisher",
  "format",
  "language",
  "publication_year",
  "planned_publication_date",
];

export type ProposedField = {
  key: string;
  label: string;
  /** The proposed value, labelled; "(clear it)" when the request clears the field. */
  value: string;
  cleared: boolean;
  /** What the record holds now (`EditRequestOut.current`); null when that is not known. */
  before: string | null;
  /** The record holds nothing in this field now. */
  wasEmpty: boolean;
  /** The proposal is what the record holds already. */
  same: boolean;
};

function show(key: string, value: unknown, locale: string): string {
  if (Array.isArray(value)) return value.map(String).join(", ");
  if (key === "format" && typeof value === "string" && (EDITION_FORMATS as readonly string[]).includes(value)) {
    return t(`editions.format.${value}`);
  }
  if (key === "language" && typeof value === "string") return languageName(value, locale);
  return typeof value === "string" ? value : JSON.stringify(value);
}

function isEmpty(value: unknown): boolean {
  return value == null || value === "" || (Array.isArray(value) && value.length === 0);
}

function sameValue(a: unknown, b: unknown): boolean {
  return JSON.stringify(isEmpty(a) ? null : a) === JSON.stringify(isEmpty(b) ? null : b);
}

type Proposal = Pick<EditRequestOut, "kind" | "payload"> & Partial<Pick<EditRequestOut, "current">>;

/**
 * The fields an edit request proposes, labelled, in the order they were sent,
 * each with the record's value now; null means "clear it".
 */
export function proposedFields(request: Proposal, locale: string): ProposedField[] {
  if (request.kind !== "work_edit" && request.kind !== "edition_edit") return [];
  const payload = request.payload as Record<string, unknown>;
  const current = (request.current ?? {}) as Record<string, unknown>;
  return Object.entries(payload).map(([key, value]) => {
    const cleared = isEmpty(value);
    const known = key in current;
    const wasEmpty = known && isEmpty(current[key]);
    return {
      key,
      label: FIELD_KEYS.includes(key) ? t(`editRequest.fields.${key}`) : key,
      value: cleared ? t("admin.cleared") : show(key, value, locale),
      cleared,
      before: !known ? null : wasEmpty ? t("admin.empty") : show(key, current[key], locale),
      wasEmpty,
      same: known && sameValue(current[key], value),
    };
  });
}

/** A merge request's target: the work this one would be merged into. */
export function mergeTarget(request: Pick<EditRequestOut, "kind" | "payload">): number | null {
  if (request.kind !== "work_merge") return null;
  const into = (request.payload as Record<string, unknown>).into_work_id;
  return typeof into === "number" ? into : null;
}

/** Items per page of either queue (the server's `admin::queue::PAGE_SIZE`). */
export const QUEUE_PAGE_SIZE = 50;

/**
 * The page to ask for next when `shown` items of a queue are listed. In the
 * open view a decision drops items, so the next page starts earlier than the
 * one after the last page read; asking from the first page not yet complete,
 * and dropping the ids already shown, skips nothing.
 */
export function nextQueuePage(shown: number): number {
  return Math.floor(shown / QUEUE_PAGE_SIZE) + 1;
}

const HOUR_MS = 3_600_000;

/** Hours from `fromIso` to `to` (default now); 0 for a time that does not parse. */
export function hoursBetween(fromIso: string, to: string | number = Date.now()): number {
  const from = new Date(fromIso).getTime();
  const until = typeof to === "number" ? to : new Date(to).getTime();
  if (Number.isNaN(from) || Number.isNaN(until)) return 0;
  return Math.max(0, (until - from) / HOUR_MS);
}

/**
 * How long something waited or took: "less than an hour", "71 hours",
 * "3 days" (whole hours below the 72-hour target, so an item close to it
 * reads so, then whole days). German reads in the dative, after "seit" and
 * "nach".
 */
export function formatAge(hours: number): string {
  if (hours < 1) return t("admin.age.underAnHour");
  if (hours < 72) return tp("admin.age.hours", Math.floor(hours));
  return tp("admin.age.days", Math.floor(hours / 24));
}

/** An open item that has waited `targetHours` or longer. */
export function isOverdue(createdAt: string, targetHours: number, now: number = Date.now()): boolean {
  return hoursBetween(createdAt, now) >= targetHours;
}

/** A picture's public id: 26 lowercase letters and digits. */
const PUBLIC_ID_RE = /^[a-z0-9]{26}$/;

/**
 * What the "take down by URL" form names: a picture (`avatar` when its
 * address is a profile picture's, `/media/{public_id}/a`), or a book page.
 */
export type PictureRef = { kind: "media"; publicId: string; avatar?: true } | { kind: "book"; book: number | string };

/**
 * Reads the "take down by URL" form: a picture's address
 * (`…/media/{public_id}/l` for a cover, `…/a` for a profile picture), a bare
 * public id, or a book page (`…/book/{id}`), which means that book's cover.
 * Null when the text names none of these.
 */
export function pictureRef(input: string): PictureRef | null {
  const text = input.trim();
  if (!text) return null;
  if (PUBLIC_ID_RE.test(text)) return { kind: "media", publicId: text };
  let path: string;
  try {
    path = new URL(text, "https://example.invalid").pathname;
  } catch {
    return null;
  }
  const media = /^\/media\/([a-z0-9]{26})(?:\/([^/]*)|$)/.exec(path);
  if (media) return media[2] === "a" ? { kind: "media", publicId: media[1], avatar: true } : { kind: "media", publicId: media[1] };
  const book = /^\/book\/([^/]+)\/?$/.exec(path);
  if (!book) return null;
  let raw: string;
  try {
    raw = decodeURIComponent(book[1]);
  } catch {
    return null;
  }
  const param = parseBookParam(raw);
  if (param.kind === "id") return { kind: "book", book: param.id };
  if (param.kind === "legacy") return { kind: "book", book: param.key };
  return null;
}

/** `date` as a `datetime-local` input value ("2026-10-28T14:05"), in the browser's zone. */
export function toLocalInput(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
    `T${pad(date.getHours())}:${pad(date.getMinutes())}`
  );
}

/**
 * The 400 of `DELETE /api/admin/media/{public_id}` with a notice, byte for
 * byte as `media::reports` sends it. The takedown form answers it with its
 * own copy; `apiErrors.ts` maps it too (`errors.receivedInFuture`) for any
 * other caller. A notice about a profile picture is recorded like one about a
 * cover (#54), so there is no second refusal any more.
 */
export const TAKEDOWN_DETAILS = {
  /** `received_at` is more than 5 minutes ahead of the server's clock. */
  receivedInFuture: "Input should be in the past",
} as const;

/** A `datetime-local` value as an RFC 3339 instant; null when empty or invalid. */
export function fromLocalInput(value: string): string | null {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date.toISOString();
}

/**
 * The cover search (#74). A clear match attaches on its own; an uncertain one
 * waits for an admin as a suggestion with the reason it was not attached
 * (`CoverSuggestionOut.reason`).
 */
export const COVER_REASONS = [
  "title_differs",
  "number_differs",
  "several_works",
  "generic_title",
  "no_author",
  "after_takedown",
] as const;

/** Why the search left a candidate for an admin, in words; an unknown reason as it is. */
export function coverReasonLabel(reason: string): string {
  return (COVER_REASONS as readonly string[]).includes(reason) ? t(`admin.covers.reason.${reason}`) : reason;
}

export const COVER_SUGGESTION_STATUSES = ["open", "approved", "rejected", "closed"] as const;

/** A suggestion's status as a badge; an unknown status as it is. */
export function coverStatusLabel(status: string): string {
  return (COVER_SUGGESTION_STATUSES as readonly string[]).includes(status) ? t(`admin.covers.status.${status}`) : status;
}

const OL_RECORD_RE = /^OL[0-9]+[WM]$/;

/**
 * The candidate's page on Open Library, for the admin to compare: its edition
 * when the picture is an edition's, else its work. Null for an id that is not
 * one (the server checks them; this keeps the link to openlibrary.org).
 */
export function coverCandidateUrl(suggestion: Pick<CoverSuggestionOut, "ol_work_id" | "ol_edition_id">): string | null {
  const edition = suggestion.ol_edition_id;
  if (edition && OL_RECORD_RE.test(edition) && edition.endsWith("M")) return `https://openlibrary.org/books/${edition}`;
  const work = suggestion.ol_work_id;
  if (OL_RECORD_RE.test(work) && work.endsWith("W")) return `https://openlibrary.org/works/${work}`;
  return null;
}

/** The outcomes of "Search now" (`CoverSearchOut.outcome`). */
export const COVER_SEARCH_OUTCOMES = ["attached", "inherited", "isbn", "suggested", "no_match"] as const;

/** What "Search now" came to, in words. */
export function coverSearchMessage(outcome: string): string {
  return (COVER_SEARCH_OUTCOMES as readonly string[]).includes(outcome)
    ? t(`admin.covers.result.${outcome}`)
    : t("admin.covers.result.no_match");
}

/** "Search now" put a cover on the book: it leaves the list of books without one. */
export function coverFound(outcome: string): boolean {
  return outcome === "attached" || outcome === "inherited" || outcome === "isbn";
}

/** Outcomes that recorded a search without a cover or a suggestion. */
const MISSES = ["no_match", "uncertain", "blocked"];

/**
 * A book of the "Without a cover" list in words, one line each: what keeps
 * its cover away (a reader's photo waiting, Open Library's picture failing to
 * load), a suggestion waiting, how the search went and when it tries next,
 * and whether a takedown left it to suggestions only. `day` formats a date.
 */
export function missingCoverNotes(
  item: Pick<
    MissingCoverOut,
    | "state"
    | "failing_edition_id"
    | "failed_at"
    | "searched_at"
    | "outcome"
    | "next_search_at"
    | "failures"
    | "after_takedown"
    | "suggestion_id"
  >,
  day: (iso: string) => string,
  now: number = Date.now(),
): string[] {
  const notes: string[] = [];
  if (item.state === "photo_waiting") notes.push(t("admin.covers.state.photoWaiting"));
  if (item.state === "failing") {
    const date = item.failed_at ? day(item.failed_at) : "";
    if (item.failing_edition_id != null) {
      notes.push(
        date
          ? t("admin.covers.state.failingEdition", { id: item.failing_edition_id, date })
          : t("admin.covers.state.failingEditionUndated", { id: item.failing_edition_id }),
      );
    } else {
      notes.push(date ? t("admin.covers.state.failing", { date }) : t("admin.covers.state.failingUndated"));
    }
  }
  if (item.suggestion_id != null) {
    notes.push(t("admin.covers.state.suggestion"));
  } else if (item.failures > 0) {
    notes.push(tp("admin.covers.state.failures", item.failures));
  } else if (!item.searched_at) {
    notes.push(t("admin.covers.state.notSearched"));
  } else if (item.outcome && MISSES.includes(item.outcome)) {
    notes.push(t("admin.covers.state.noMatch", { date: day(item.searched_at) }));
  } else {
    notes.push(t("admin.covers.state.searched", { date: day(item.searched_at) }));
  }
  if (item.suggestion_id == null && item.next_search_at) {
    const next = new Date(item.next_search_at).getTime();
    if (!Number.isNaN(next) && next > now) notes.push(t("admin.covers.state.nextTry", { date: day(item.next_search_at) }));
  }
  if (item.after_takedown) notes.push(t("admin.covers.state.afterTakedown"));
  return notes;
}
