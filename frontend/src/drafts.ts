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

/** An empty draft is forgotten rather than stored. */
export function saveDraft(key: string, text: string): void {
  try {
    if (text.trim()) localStorage.setItem(key, text);
    else localStorage.removeItem(key);
  } catch {
    // Private mode or a full quota: the draft just does not persist.
  }
}
