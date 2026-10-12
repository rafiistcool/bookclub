import { afterEach, describe, expect, it, vi } from "vitest";
import { draftKey, loadDraft, saveDraft } from "./drafts";

function memoryStorage(): Storage {
  const data = new Map<string, string>();
  return {
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => void data.set(key, String(value)),
    removeItem: (key) => void data.delete(key),
    clear: () => data.clear(),
    key: (index) => [...data.keys()][index] ?? null,
    get length() { return data.size; },
  };
}

describe("note drafts", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("keeps drafts apart per member, book and reply", () => {
    expect(draftKey("ada", "OL1W")).toBe("bookclub.draft.ada.OL1W");
    expect(draftKey("ada", "OL1W", "reply", 7)).toBe("bookclub.draft.ada.OL1W.reply.7");
    expect(draftKey("grace", "OL1W")).not.toBe(draftKey("ada", "OL1W"));
    expect(draftKey(null, "OL1W")).toBe("bookclub.draft.anon.OL1W");
  });

  it("restores a saved draft and forgets an emptied one", () => {
    vi.stubGlobal("localStorage", memoryStorage());
    const key = draftKey("ada", "OL1W");
    expect(loadDraft(key)).toBe("");
    saveDraft(key, "Half a thought");
    expect(loadDraft(key)).toBe("Half a thought");
    saveDraft(key, "   ");
    expect(loadDraft(key)).toBe("");
    expect(localStorage.length).toBe(0);
  });

  it("does not break writing when storage is unavailable", () => {
    vi.stubGlobal("localStorage", {
      getItem: () => { throw new Error("blocked"); },
      setItem: () => { throw new Error("blocked"); },
      removeItem: () => { throw new Error("blocked"); },
    });
    expect(() => saveDraft("key", "text")).not.toThrow();
    expect(loadDraft("key")).toBe("");
  });
});
