import type { CoverSize } from "./constants";

/** What a cover needs to know about a book. `BookOut`, `SearchHit` and `BookDetailOut` all fit. */
export type CoverBook = {
  id?: number;
  title: string;
  authors?: string;
  ol_work_key?: string | null;
  cover_url: string | null;
  cover_thumb_url: string | null;
};

/** How long a failed cover waits before its one retry (the server may still be fetching it). */
export const COVER_RETRY_MS = 1500;

/**
 * Covers come from our own `/media/…` URLs only. Anything that would make the
 * browser ask another host (an absolute URL, `//host`, `/\host`, or a tab or
 * newline the URL parser strips, as in `/<tab>/host`) is dropped, as in
 * `safeNextPath`.
 */
export function sameOriginPath(url: string | null | undefined): string | null {
  const text = (url || "").trim();
  if (!text.startsWith("/") || text.startsWith("//") || text.includes("\\")) return null;
  if (/[\u0000-\u001f\u007f]/.test(text)) return null;
  return text;
}

/**
 * The cover URLs to try, best first. Small renders take the thumbnail
 * (`/s`, at most 240 px wide) and fall back to the large picture; the
 * full-width cover takes the large one (`/l`, at most 800 px).
 */
export function coverSources(book: CoverBook, size: CoverSize): string[] {
  const thumb = sameOriginPath(book.cover_thumb_url);
  const large = sameOriginPath(book.cover_url);
  const order = size === "fluid" ? [large, thumb] : [thumb, large];
  return [...new Set(order.filter((url): url is string => Boolean(url)))];
}

/**
 * The key for the cloth colour a book gets without a cover, the same on its
 * spine on the bookshelf page. Open Library works keep the key the legacy app
 * hashed; other works hash their id, so every edition looks alike.
 */
export function bookColorKey(book: { id?: number; ol_work_key?: string | null; title: string; authors?: string }): string {
  if (book.ol_work_key) return book.ol_work_key;
  if (book.id != null) return `work:${book.id}`;
  return `${book.title}:${book.authors ?? ""}`;
}

/**
 * The book page asks for a photo of the reader's copy (#74): a catalog or
 * imported book (not one a reader added, which may keep its cloth) that has no
 * cover and is on the viewer's shelf, in any status.
 */
export function asksForCoverPhoto(book: {
  custom: boolean;
  on_shelf: string | null;
  cover_url: string | null;
  cover_thumb_url: string | null;
}): boolean {
  return !book.custom && book.on_shelf != null && !book.cover_url && !book.cover_thumb_url;
}
