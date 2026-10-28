import { ApiError } from "./api/client";

/** The server's detail for a mail link that is unknown, expired or already used. */
export const LINK_EXPIRED = "That link has expired or was already used";

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

/**
 * True when the server refused the mail link itself, so another try with the
 * same link cannot help and only a new link does.
 */
export function isDeadLink(err: unknown): boolean {
  return err instanceof ApiError && err.status === 400 && err.detail === LINK_EXPIRED;
}
