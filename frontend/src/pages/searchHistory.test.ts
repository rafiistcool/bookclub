import { afterEach, describe, expect, it, vi } from "vitest";
import {
  SEARCH_HISTORY_KEY,
  SEARCH_HISTORY_LIMIT,
  clearSearchHistory,
  forgetSearch,
  readSearchHistory,
  rememberSearch,
  resetSearchHistory,
  writeSearchHistory,
} from "./searchHistory";

describe("searchHistory", () => {
  afterEach(() => {
    resetSearchHistory();
    vi.unstubAllGlobals();
  });

  it("starts empty and ignores blank queries", () => {
    expect(readSearchHistory()).toEqual([]);
    expect(rememberSearch("   ")).toEqual([]);
    expect(readSearchHistory()).toEqual([]);
  });

  it("puts the latest query first and dedupes case-insensitively", () => {
    rememberSearch("Circe");
    rememberSearch("Dune");
    expect(rememberSearch("circe")).toEqual(["circe", "Dune"]);
  });

  it("caps the list and drops the oldest", () => {
    for (let i = 0; i < SEARCH_HISTORY_LIMIT + 3; i += 1) {
      rememberSearch(`query-${i}`);
    }
    const items = readSearchHistory();
    expect(items).toHaveLength(SEARCH_HISTORY_LIMIT);
    expect(items[0]).toBe(`query-${SEARCH_HISTORY_LIMIT + 2}`);
    expect(items).not.toContain("query-0");
  });

  it("forgets one query and can clear the rest", () => {
    rememberSearch("Circe");
    rememberSearch("Dune");
    expect(forgetSearch("circe")).toEqual(["Dune"]);
    expect(clearSearchHistory()).toEqual([]);
    expect(readSearchHistory()).toEqual([]);
  });

  it("skips corrupt JSON and non-string entries", () => {
    localStorage.setItem(SEARCH_HISTORY_KEY, "{not json");
    expect(readSearchHistory()).toEqual([]);
    writeSearchHistory(["ok", "  ", 12 as unknown as string, "ok"]);
    expect(readSearchHistory()).toEqual(["ok"]);
  });

  it("survives localStorage throwing", () => {
    const boom = () => {
      throw new Error("private");
    };
    vi.stubGlobal("localStorage", {
      getItem: boom,
      setItem: boom,
      removeItem: boom,
    });
    expect(readSearchHistory()).toEqual([]);
    expect(rememberSearch("Circe")).toEqual(["Circe"]);
    expect(() => resetSearchHistory()).not.toThrow();
  });
});
