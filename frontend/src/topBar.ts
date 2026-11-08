/** The phone's top bar while the page scrolls: hidden or not, and the scroll offset of the last decision. */
export type BarState = { hidden: boolean; anchor: number };

/**
 * The phone's top bar (the wordmark) gives its 56 px back while the reader
 * scrolls down and returns on the way up. Near the top of the page it always
 * shows. Until the page has moved `jitter` px from the last decision (a slow
 * drag, a finger resting, a rubber band) the bar stays as it is; the anchor
 * only moves with a decision, so a slow scroll still adds up.
 */
export function nextBar(state: BarState, y: number, barHeight: number, jitter = 8): BarState {
  if (y <= barHeight) return { hidden: false, anchor: y };
  const delta = y - state.anchor;
  if (Math.abs(delta) < jitter) return state;
  return { hidden: delta > 0, anchor: y };
}

/** Where the bar may hide: phones and small tablets. The desktop has no top bar at all. */
export const HIDING_BAR_QUERY = "(max-width: 720px)";
