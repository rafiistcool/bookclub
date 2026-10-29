import type { EditionOut } from "./api/generated";
import { t } from "./i18n";

/** `EditionOut.format`'s values. */
export const EDITION_FORMATS = ["hardcover", "paperback", "ebook", "audiobook", "other"] as const;

/**
 * An edition as a report names it: its own title, when it has one, before
 * its line ("Sonderausgabe · Paperback · Penguin · 2019"). The admin queue
 * and the decision mails name it by that title too.
 */
export function editionName(edition: EditionOut): string {
  return edition.title ? `${edition.title} · ${editionLine(edition)}` : editionLine(edition);
}

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
