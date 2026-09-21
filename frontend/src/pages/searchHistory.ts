/** Recent Discover queries, kept in this browser so titles do not have to be retyped. */

export const SEARCH_HISTORY_KEY = "bookclub.searchHistory";
export const SEARCH_HISTORY_LIMIT = 8;
const QUERY_MAX = 200;

function sanitize(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!trimmed) return null;
  return trimmed.length > QUERY_MAX ? trimmed.slice(0, QUERY_MAX) : trimmed;
}

function readRaw(): unknown {
  try {
    const raw = localStorage.getItem(SEARCH_HISTORY_KEY);
    if (!raw) return [];
    return JSON.parse(raw);
  } catch {
    return [];
  }
}

export function readSearchHistory(): string[] {
  const parsed = readRaw();
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

export function writeSearchHistory(items: string[]) {
  const next = items
    .map((item) => sanitize(item))
    .filter((item): item is string => item !== null)
    .slice(0, SEARCH_HISTORY_LIMIT);
  try {
    localStorage.setItem(SEARCH_HISTORY_KEY, JSON.stringify(next));
  } catch {
    /* Private mode or a full quota. History is a convenience, not a hard requirement. */
  }
}

export function rememberSearch(query: string): string[] {
  const trimmed = sanitize(query);
  if (!trimmed) return readSearchHistory();
  const rest = readSearchHistory().filter(
    (item) => item.toLowerCase() !== trimmed.toLowerCase(),
  );
  const next = [trimmed, ...rest].slice(0, SEARCH_HISTORY_LIMIT);
  writeSearchHistory(next);
  return next;
}

export function forgetSearch(query: string): string[] {
  const needle = query.trim().toLowerCase();
  const next = readSearchHistory().filter((item) => item.toLowerCase() !== needle);
  writeSearchHistory(next);
  return next;
}

export function clearSearchHistory(): string[] {
  writeSearchHistory([]);
  return [];
}

export function resetSearchHistory() {
  try {
    localStorage.removeItem(SEARCH_HISTORY_KEY);
  } catch {
    /* jsdom / private mode. */
  }
}
