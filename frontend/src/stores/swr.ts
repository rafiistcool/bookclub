/**
 * Tiny stale-while-revalidate helper for Pinia stores.
 *
 * `load()` returns cached data immediately when present and refreshes in the
 * background; `load(true)` always awaits the network. Pages call `load()` on
 * mount so switching tabs renders instantly, and mutations call `invalidate()`
 * (or set the data directly) so the next reader is fresh.
 */

import { ApiError } from "../api/client";

export type SwrState<T> = {
  data: T | null;
  loadedAt: number;
  pending: boolean;
  error: string;
};

export function swrState<T>(): SwrState<T> {
  return { data: null, loadedAt: 0, pending: false, error: "" };
}

export const STALE_AFTER = 20_000;

export async function swrLoad<T>(
  state: SwrState<T>,
  fetcher: () => Promise<T>,
  force = false,
  fallback = "Could not load",
): Promise<T | null> {
  const fresh = state.data !== null && Date.now() - state.loadedAt < STALE_AFTER;
  if (!force && fresh) return state.data;
  const run = async () => {
    state.pending = true;
    try {
      const data = await fetcher();
      state.data = data;
      state.loadedAt = Date.now();
      state.error = "";
      return data;
    } catch (err) {
      state.error = err instanceof ApiError ? err.message : fallback;
      if (state.data === null) throw err;
      return state.data;
    } finally {
      state.pending = false;
    }
  };
  if (!force && state.data !== null) {
    void run().catch(() => undefined);
    return state.data;
  }
  return run();
}
