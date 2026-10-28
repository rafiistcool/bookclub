// Half-written notes survive an app switch or a reload. They stay on this
// device, under the member's own key.
const PREFIX = "bookclub.draft";

export function draftKey(username: string | null | undefined, ...parts: (string | number)[]): string {
  return [PREFIX, username || "anon", ...parts].join(".");
}

export function loadDraft(key: string): string {
  try {
    return localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}

/**
 * Forgets every unsent note on this device, when its reader signs out here
 * or deletes the account: a shared or borrowed browser must not keep text
 * that may have been meant as only-me. A session that merely expired keeps
 * them, so a reader who signs in again finds their half-written note.
 */
export function forgetDrafts(): void {
  try {
    const keys: string[] = [];
    for (let index = 0; index < localStorage.length; index += 1) {
      const key = localStorage.key(index);
      if (key?.startsWith(`${PREFIX}.`)) keys.push(key);
    }
    for (const key of keys) localStorage.removeItem(key);
  } catch {
    // Private mode or blocked storage: nothing was stored either.
  }
}

/** An empty draft is forgotten rather than stored. */
export function saveDraft(key: string, text: string): void {
  try {
    if (text.trim()) localStorage.setItem(key, text);
    else localStorage.removeItem(key);
  } catch {
    // Private mode or a full quota: the draft just does not persist.
  }
}
