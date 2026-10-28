import { ApiError } from "./api/client";

/**
 * True when a failed request may succeed if the reader tries again: the
 * network or the server failed (no answer, a body that is not ours, a 5xx,
 * 408 or 429). Any other refusal, such as 404 `No such book.`, answers the
 * same every time, so a page offers a way on instead of "Try again".
 */
export function worthRetrying(err: unknown): boolean {
  if (!(err instanceof ApiError)) return true;
  return err.status >= 500 || err.status === 408 || err.status === 429;
}
