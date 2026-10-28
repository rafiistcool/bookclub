/**
 * The token of a mail link. Links carry it in the fragment (`#token=…`), so
 * it never reaches a server log or a Referer header; a `?token=` query is
 * accepted too.
 */
export function linkToken(hash: string, query?: unknown): string | null {
  const fragment = new URLSearchParams(hash.replace(/^#/, ""));
  const token = fragment.get("token") ?? (typeof query === "string" ? query : null);
  const trimmed = token?.trim();
  return trimmed ? trimmed : null;
}
