/** Recent Discover queries, kept per signed-in member in this browser. */

export const SEARCH_HISTORY_KEY = "bookclub.searchHistory";
export const SEARCH_HISTORY_LIMIT = 8;
const QUERY_MAX = 200;

export type HistoryOwner = string | number;

export function searchHistoryKey(owner: HistoryOwner): string {
  return `${SEARCH_HISTORY_KEY}.${owner}`;
}

function hasOwner(owner: HistoryOwner | null | undefined): owner is HistoryOwner {
  if (owner == null) return false;
  return String(owner).trim() !== "";
}

function sanitize(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!trimmed) return null;
  return trimmed.length > QUERY_MAX ? trimmed.slice(0, QUERY_MAX) : trimmed;
}

function readRaw(owner: HistoryOwner): unknown {
  try {
    const raw = localStorage.getItem(searchHistoryKey(owner));
    if (!raw) return [];
    return JSON.parse(raw);
  } catch {
    return [];
  }
}

export function readSearchHistory(owner: HistoryOwner | null | undefined): string[] {
  if (!hasOwner(owner)) return [];
  const parsed = readRaw(owner);
  if (!Array.isArray(parsed)) return [];
  const seen = new Set<string>();
  const items: string[] = [];
  for (const entry of parsed) {
    const query = sanitize(entry);
    if (!query) continue;
    const key = query.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    items.push(query);
    if (items.length >= SEARCH_HISTORY_LIMIT) break;
  }
  return items;
}

export function writeSearchHistory(owner: HistoryOwner | null | undefined, items: string[]) {
  if (!hasOwner(owner)) return;
  const next = items
    .map((item) => sanitize(item))
    .filter((item): item is string => item !== null)
    .slice(0, SEARCH_HISTORY_LIMIT);
  try {
    localStorage.setItem(searchHistoryKey(owner), JSON.stringify(next));
  } catch {
    /* Private mode or a full quota. History is a convenience, not a hard requirement. */
  }
}

export function rememberSearch(
  owner: HistoryOwner | null | undefined,
  query: string,
): string[] {
  if (!hasOwner(owner)) return [];
  const trimmed = sanitize(query);
  if (!trimmed) return readSearchHistory(owner);
  const rest = readSearchHistory(owner).filter(
    (item) => item.toLowerCase() !== trimmed.toLowerCase(),
  );
  const next = [trimmed, ...rest].slice(0, SEARCH_HISTORY_LIMIT);
  writeSearchHistory(owner, next);
  return next;
}

export function forgetSearch(owner: HistoryOwner | null | undefined, query: string): string[] {
  if (!hasOwner(owner)) return [];
  const needle = query.trim().toLowerCase();
  const next = readSearchHistory(owner).filter((item) => item.toLowerCase() !== needle);
  writeSearchHistory(owner, next);
  return next;
}

export function clearSearchHistory(owner: HistoryOwner | null | undefined): string[] {
  if (!hasOwner(owner)) return [];
  writeSearchHistory(owner, []);
  return [];
}

function historyKeys(): string[] {
  const keys: string[] = [];
  try {
    for (let i = 0; i < localStorage.length; i += 1) {
      const key = localStorage.key(i);
      if (!key) continue;
      if (key === SEARCH_HISTORY_KEY || key.startsWith(`${SEARCH_HISTORY_KEY}.`)) {
        keys.push(key);
      }
    }
  } catch {
    /* jsdom / private mode. */
  }
  return keys;
}

export function resetSearchHistory(owner?: HistoryOwner | null) {
  try {
    if (hasOwner(owner)) {
      localStorage.removeItem(searchHistoryKey(owner));
      return;
    }
    for (const key of historyKeys()) localStorage.removeItem(key);
  } catch {
    /* jsdom / private mode. */
  }
}
