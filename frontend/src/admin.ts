import type { EditRequestOut } from "./api/generated";
import { EDITION_FORMATS } from "./editions";
import { t } from "./i18n";
import { languageName } from "./languages";

/**
 * Reading the admin queue. `EditRequestOut.payload` is an object without a
 * schema: the proposed fields for an edit, `{into_work_id}` for a merge,
 * `{}` for a cover photo.
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

export type ProposedField = { key: string; label: string; value: string; cleared: boolean };

function show(key: string, value: unknown, locale: string): string {
  if (Array.isArray(value)) return value.map(String).join(", ");
  if (key === "format" && typeof value === "string" && (EDITION_FORMATS as readonly string[]).includes(value)) {
    return t(`editions.format.${value}`);
  }
  if (key === "language" && typeof value === "string") return languageName(value, locale);
  return typeof value === "string" ? value : JSON.stringify(value);
}

/** The fields an edit request proposes, labelled, in the order they were sent; null means "clear it". */
export function proposedFields(request: Pick<EditRequestOut, "kind" | "payload">, locale: string): ProposedField[] {
  if (request.kind !== "work_edit" && request.kind !== "edition_edit") return [];
  const payload = request.payload as Record<string, unknown>;
  return Object.entries(payload).map(([key, value]) => {
    const cleared = value === null || (Array.isArray(value) && value.length === 0);
    return {
      key,
      label: FIELD_KEYS.includes(key) ? t(`editRequest.fields.${key}`) : key,
      value: cleared ? t("admin.cleared") : show(key, value, locale),
      cleared,
    };
  });
}

/** A merge request's target: the work this one would be merged into. */
export function mergeTarget(request: Pick<EditRequestOut, "kind" | "payload">): number | null {
  if (request.kind !== "work_merge") return null;
  const into = (request.payload as Record<string, unknown>).into_work_id;
  return typeof into === "number" ? into : null;
}
