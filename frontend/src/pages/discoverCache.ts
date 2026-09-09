import type { SearchHit } from "../types";

export type CachedBrowseRow = {
  subject: string;
  label: string;
  items: SearchHit[];
  error: string;
};

export type BrowseSnapshot = {
  trending: SearchHit[];
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
