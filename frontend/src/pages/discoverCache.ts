import type { SearchHit } from "../types";

export type CachedBrowseRow = {
  subject: string;
  label: string;
  items: SearchHit[];
  error: string;
};

export type BrowseSnapshot = {
  trending: SearchHit[];
  /** The DNB new-releases row; null until it has loaded once. */
  releases: SearchHit[] | null;
  rows: CachedBrowseRow[];
  browseError: string;
};

let snapshot: BrowseSnapshot | null = null;

export function readBrowseSnapshot(): BrowseSnapshot | null {
  return snapshot;
}

export function writeBrowseSnapshot(next: BrowseSnapshot) {
  snapshot = {
    trending: [...next.trending],
    releases: next.releases ? [...next.releases] : null,
    browseError: next.browseError,
    rows: next.rows.map((row) => ({
      ...row,
      items: [...row.items],
    })),
  };
}

export function resetDiscoverBrowseCache() {
  snapshot = null;
}
