import type { EditionOut } from "./api/generated";
import { t } from "./i18n";

/** `EditionOut.format`'s values. */
export const EDITION_FORMATS = ["hardcover", "paperback", "ebook", "audiobook", "other"] as const;

/** One line that tells editions apart: "Paperback · Penguin · 2019 · ISBN 9780…". */
export function editionLine(edition: EditionOut): string {
  const parts: string[] = [];
  if (edition.format && (EDITION_FORMATS as readonly string[]).includes(edition.format)) {
    parts.push(t(`editions.format.${edition.format}`));
  }
  if (edition.publisher) parts.push(edition.publisher);
  if (edition.publication_year) parts.push(String(edition.publication_year));
  if (edition.pages) parts.push(t("editions.pages", { n: edition.pages }));
  if (edition.isbn13) parts.push(`ISBN ${edition.isbn13}`);
  return parts.join(" · ") || t("editions.unnamed", { id: edition.id });
}
