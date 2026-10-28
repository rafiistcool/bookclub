import type { EditFields } from "./api/client";
import type { BookDetailOut, EditionOut } from "./api/generated";

/**
 * Corrections of catalog facts go to the admin queue as edit requests
 * (`POST …/edit-requests`). The form shows every field the request may
 * change, filled with what the catalog has now, and sends only what the
 * reader changed. Keys, ranges and lengths are `apply_admin_fields`'s.
 */
export type FieldKind = "text" | "textarea" | "number" | "subjects" | "format" | "language";

export type FieldDef = {
  key: string;
  kind: FieldKind;
  /** Inclusive bounds of a number. */
  min?: number;
  max?: number;
  /** Characters of a text, or of each subject. */
  maxlength?: number;
  /** Cannot be cleared (a work's title). */
  required?: boolean;
};

/**
 * What the form edits: every field as its input holds it (a number input's
 * `v-model` gives a number, or "" when empty).
 */
export type Draft = Record<string, string | number>;

/** The years `apply_admin_fields` accepts (antiquity included). */
const YEARS = { min: -5000, max: 3000 };

export const WORK_FIELDS: readonly FieldDef[] = [
  { key: "title", kind: "text", maxlength: 1000, required: true },
  { key: "subtitle", kind: "text", maxlength: 1000 },
  { key: "authors", kind: "text", maxlength: 2000 },
  { key: "year", kind: "number", ...YEARS },
  { key: "pages", kind: "number", min: 1, max: 100_000 },
  { key: "description", kind: "textarea", maxlength: 50_000 },
  { key: "subjects", kind: "subjects", maxlength: 200 },
];

export const EDITION_FIELDS: readonly FieldDef[] = [
  { key: "title", kind: "text", maxlength: 1000 },
  { key: "subtitle", kind: "text", maxlength: 1000 },
  { key: "publisher", kind: "text", maxlength: 300 },
  { key: "format", kind: "format" },
  { key: "language", kind: "language" },
  { key: "pages", kind: "number", min: 1, max: 100_000 },
  { key: "publication_year", kind: "number", ...YEARS },
];

/** At most this many subjects per work. */
export const MAX_SUBJECTS = 50;

function text(value: string | number | null | undefined): string {
  return value == null ? "" : String(value);
}

export function workDraft(book: BookDetailOut): Draft {
  return {
    title: book.title,
    subtitle: book.subtitle,
    authors: book.authors,
    year: text(book.year),
    pages: text(book.pages),
    description: book.description,
    // One per line: a subject may hold a comma ("Women, history").
    subjects: book.subjects.join("\n"),
  };
}

export function editionDraft(edition: EditionOut): Draft {
  return {
    title: text(edition.title),
    subtitle: text(edition.subtitle),
    publisher: edition.publisher,
    format: text(edition.format),
    language: text(edition.language),
    pages: text(edition.pages),
    publication_year: text(edition.publication_year),
  };
}

function subjects(raw: string): string[] {
  return raw
    .split("\n")
    .map((item) => item.trim())
    .filter(Boolean);
}

type Value = string | number | string[] | null;

/** One field's value as the request sends it; `undefined` when it is not valid. */
function parse(def: FieldDef, input: string | number | undefined): Value | undefined {
  const raw = String(input ?? "");
  const trimmed = raw.trim();
  switch (def.kind) {
    case "number": {
      if (!trimmed) return null;
      const n = Number(trimmed);
      if (!Number.isInteger(n)) return undefined;
      if ((def.min != null && n < def.min) || (def.max != null && n > def.max)) return undefined;
      return n;
    }
    case "subjects": {
      const list = subjects(raw);
      if (list.length > MAX_SUBJECTS) return undefined;
      if (def.maxlength && list.some((item) => item.length > def.maxlength!)) return undefined;
      return list;
    }
    default:
      if (!trimmed) return def.required ? undefined : null;
      if (def.maxlength && trimmed.length > def.maxlength) return undefined;
      return trimmed;
  }
}

function same(a: Value, b: Value): boolean {
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((item, i) => item === b[i]);
  return a === b;
}

export type Changes = { fields: EditFields; invalid: string[] };

/**
 * The fields the reader changed, as the request sends them (a cleared field
 * is null), and the keys whose new value the server would refuse.
 */
export function changedFields(defs: readonly FieldDef[], before: Draft, after: Draft): Changes {
  const fields: EditFields = {};
  const invalid: string[] = [];
  for (const def of defs) {
    const next = parse(def, after[def.key]);
    if (next === undefined) {
      invalid.push(def.key);
      continue;
    }
    const previous = parse({ ...def, required: false, min: undefined, max: undefined, maxlength: undefined }, before[def.key]);
    if (previous !== undefined && same(previous, next)) continue;
    fields[def.key] = next;
  }
  return { fields, invalid };
}
