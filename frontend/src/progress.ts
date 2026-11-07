import { t } from "./i18n";
import type { ShelfPatchIn } from "./api/generated";

/**
 * Where a reader is in a book: the percent everyone sees, and the page when
 * they count in pages (the book has a page count). The server keeps the page
 * (`current_page`) and derives the percent from it, so one page more is a
 * change even when the percent stays (207 → 208 of 334 is 62 % both times).
 */
export type ReadingPosition = { progress: number; page: number | null };

/** A shelf entry's (or a reader row's) position; no progress is 0 %. */
export function positionOf(entry: { progress: number | null; current_page?: number | null } | null | undefined): ReadingPosition {
  return { progress: entry?.progress ?? 0, page: entry?.current_page ?? null };
}

/** The PATCH that saves a position: the page when there is one, else the percent. */
export function positionPatch(position: ReadingPosition): ShelfPatchIn {
  return position.page != null ? { current_page: position.page } : { progress: position.progress };
}

/** The toast after a save: "Seite 208 · 62 %" or "Fortschritt: 62 %". */
export function positionSaved(position: ReadingPosition): string {
  return position.page != null
    ? t("progress.pageSaved", { page: position.page, n: position.progress })
    : t("book.progressSaved", { n: position.progress });
}

/**
 * The percent of `page` in a book of `pages` pages, as the server derives it
 * (`shelf::ordering::page_percent`): the nearest whole percent, but 100 only
 * on the last page and at least 1 from the first page on.
 */
export function pagePercent(page: number, pages: number): number {
  const count = Math.max(1, Math.round(pages));
  const at = Math.min(count, Math.max(0, Math.round(page)));
  if (at === count) return 100;
  if (at === 0) return 0;
  return Math.min(99, Math.max(1, Math.floor((at * 100 + Math.floor(count / 2)) / count)));
}

/** The page a stored percent stands for, in a book of `pages` pages (an old save in percent). */
export function pageOfPercent(percent: number, pages: number): number {
  return Math.round((percent / 100) * pages);
}
