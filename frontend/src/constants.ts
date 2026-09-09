export const STATUSES = [
  "want_to_read",
  "currently_reading",
  "finished",
  "did_not_finish",
] as const;

export type Status = (typeof STATUSES)[number];

export const STATUS_LABEL: Record<Status, string> = {
  want_to_read: "Want to read",
  currently_reading: "Reading",
  finished: "Finished",
  did_not_finish: "Did not finish",
};

export const STATUS_SHORT: Record<Status, string> = {
  want_to_read: "Want",
  currently_reading: "Reading",
  finished: "Done",
  did_not_finish: "DNF",
};

export type CoverSize = "xs" | "sm" | "md" | "lg" | "fluid" | "tile";

/** Open Library's image tiers, mapped to the sizes we actually render. */
const COVER_TIER: Record<CoverSize, "S" | "M" | "L"> = {
  xs: "S",
  sm: "S",
  md: "M",
  lg: "M",
  tile: "M",
  fluid: "L",
};

const ISBN_RE = /^(?:\d{9}[\dXx]|\d{13})$/;
const CLUB_WORK_ID_RE = /^BC[a-f0-9]{10}$/;
const OL_WORK_ID_RE = /^OL\d+W$/;
const ISBN_WORK_ID_RE = /^ISBN(?:\d{9}[\dXx]|\d{13})$/;
const GOOGLE_WORK_ID_RE = /^GB[A-Za-z0-9_-]{1,40}$/;

export function extractIsbn(raw: string): string | null {
  const cleaned = raw.replace(/[\s-]/g, "").toUpperCase();
  return ISBN_RE.test(cleaned) ? cleaned : null;
}

function coverSrc(
  kind: "id" | "olid" | "isbn",
  value: string | number,
  size: CoverSize,
): string {
  return `https://covers.openlibrary.org/b/${kind}/${value}-${COVER_TIER[size]}.jpg?default=false`;
}

export function coverUrl(
  coverId: number | null | undefined,
  size: CoverSize = "fluid",
): string | null {
  if (coverId == null || coverId <= 0) return null;
  return coverSrc("id", coverId, size);
}

export function editionCoverUrl(
  olid: string | null | undefined,
  size: CoverSize = "fluid",
): string | null {
  const key = olid?.trim();
  if (!key) return null;
  return coverSrc("olid", key, size);
}

export function isbnCoverUrl(
  isbn: string | null | undefined,
  size: CoverSize = "fluid",
): string | null {
  const cleaned = extractIsbn(isbn || "");
  if (!cleaned) return null;
  return coverSrc("isbn", cleaned, size);
}

/** Google Books (or any https) cover. Rewrite http→https; reject non-URLs. */
export function remoteCoverUrl(url: string | null | undefined): string | null {
  const text = (url || "").trim();
  if (!text) return null;
  const https = text.startsWith("http://") ? `https://${text.slice(7)}` : text;
  if (!https.startsWith("https://")) return null;
  try {
    const parsed = new URL(https);
    if (parsed.protocol !== "https:") return null;
    return https;
  } catch {
    return null;
  }
}

/** "/works/OL1W" -> "OL1W", the form the detail route and API path take. */
export function workId(olWorkKey: string): string {
  return olWorkKey.replace(/^\/works\//, "");
}

export function isClubWorkId(id: string): boolean {
  return CLUB_WORK_ID_RE.test(id);
}

export function isClubWorkKey(olWorkKey: string): boolean {
  return isClubWorkId(workId(olWorkKey));
}

export function isOpenLibraryWorkId(id: string): boolean {
  return OL_WORK_ID_RE.test(id);
}

export function isIsbnWorkId(id: string): boolean {
  return ISBN_WORK_ID_RE.test(id);
}

export function isGoogleWorkId(id: string): boolean {
  return GOOGLE_WORK_ID_RE.test(id);
}

export function isGoogleCatalogWorkKey(olWorkKey: string): boolean {
  const id = workId(olWorkKey);
  return isIsbnWorkId(id) || isGoogleWorkId(id);
}

export function isbnFromWorkKey(olWorkKey: string): string | null {
  const id = workId(olWorkKey);
  const canonical = id.startsWith("ISBN") || id.startsWith("isbn")
    ? `ISBN${id.slice(4).toUpperCase()}`
    : id;
  return isIsbnWorkId(canonical) ? extractIsbn(canonical.slice(4)) : null;
}

export function bookPath(olWorkKey: string): string {
  return `/book/${workId(olWorkKey)}`;
}

export function openLibraryUrl(olWorkKey: string): string | null {
  const id = workId(olWorkKey);
  if (!isOpenLibraryWorkId(id)) return null;
  return `https://openlibrary.org/works/${id}`;
}

export function googleBooksUrl(olWorkKey: string): string | null {
  const id = workId(olWorkKey);
  if (isGoogleWorkId(id)) {
    return `https://books.google.com/books?id=${id.slice(2)}`;
  }
  const isbn = isbnFromWorkKey(olWorkKey);
  if (isbn) {
    return `https://books.google.com/books?vid=ISBN${isbn}`;
  }
  return null;
}

export function catalogUrl(olWorkKey: string): string | null {
  return openLibraryUrl(olWorkKey) || googleBooksUrl(olWorkKey);
}

export type FinishNote = {
  rating?: number | null;
  take?: string;
  dnf_reason?: string;
  progress?: number | null;
};

export function starLabel(rating: number | null | undefined): string {
  if (!rating) return "";
  return "★".repeat(rating) + "☆".repeat(5 - rating);
}

export function monogram(title: string): string {
  const letter = title.trim().charAt(0);
  return letter ? letter.toUpperCase() : "?";
}

/** "in 3 days" / "today" / "2 weeks ago", for meeting dates. */
export function relativeDay(iso: string | null | undefined): string {
  if (!iso) return "";
  const target = new Date(iso);
  if (Number.isNaN(target.getTime())) return "";
  const startOfDay = (date: Date) =>
    new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  const days = Math.round(
    (startOfDay(target) - startOfDay(new Date())) / 86_400_000,
  );
  if (days === 0) return "today";
  if (days === 1) return "tomorrow";
  if (days === -1) return "yesterday";
  if (days > 0) return days < 14 ? `in ${days} days` : `in ${Math.round(days / 7)} weeks`;
  const ago = Math.abs(days);
  return ago < 14 ? `${ago} days ago` : `${Math.round(ago / 7)} weeks ago`;
}
