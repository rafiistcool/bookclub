import { afterEach, describe, expect, it, vi } from "vitest";
import {
  SEARCH_HISTORY_LIMIT,
  clearSearchHistory,
  forgetSearch,
  readSearchHistory,
  rememberSearch,
  resetSearchHistory,
  searchHistoryKey,
  writeSearchHistory,
} from "./searchHistory";

const ADA = 1;
const LENA = 2;

describe("searchHistory", () => {
  afterEach(() => {
    resetSearchHistory();
    vi.unstubAllGlobals();
  });

  it("starts empty and ignores blank queries", () => {
    expect(readSearchHistory(ADA)).toEqual([]);
    expect(rememberSearch(ADA, "   ")).toEqual([]);
    expect(readSearchHistory(ADA)).toEqual([]);
  });

  it("puts the latest query first and dedupes case-insensitively", () => {
    rememberSearch(ADA, "Circe");
    rememberSearch(ADA, "Dune");
    expect(rememberSearch(ADA, "circe")).toEqual(["circe", "Dune"]);
  });

  it("caps the list and drops the oldest", () => {
    for (let i = 0; i < SEARCH_HISTORY_LIMIT + 3; i += 1) {
      rememberSearch(ADA, `query-${i}`);
    }
    const items = readSearchHistory(ADA);
    expect(items).toHaveLength(SEARCH_HISTORY_LIMIT);
    expect(items[0]).toBe(`query-${SEARCH_HISTORY_LIMIT + 2}`);
    expect(items).not.toContain("query-0");
  });

  it("forgets one query and can clear the rest", () => {
    rememberSearch(ADA, "Circe");
    rememberSearch(ADA, "Dune");
    expect(forgetSearch(ADA, "circe")).toEqual(["Dune"]);
    expect(clearSearchHistory(ADA)).toEqual([]);
    expect(readSearchHistory(ADA)).toEqual([]);
  });

  it("keeps recents in a per-member key", () => {
    rememberSearch(ADA, "Dune");
    rememberSearch(LENA, "Circe");
    expect(readSearchHistory(ADA)).toEqual(["Dune"]);
    expect(readSearchHistory(LENA)).toEqual(["Circe"]);
    expect(localStorage.getItem(searchHistoryKey(ADA))).toContain("Dune");
    expect(localStorage.getItem(searchHistoryKey(LENA))).toContain("Circe");
    expect(readSearchHistory(null)).toEqual([]);
    expect(rememberSearch(null, "Neuromancer")).toEqual([]);
    expect(localStorage.getItem("bookclub.searchHistory")).toBeNull();
  });

  it("skips corrupt JSON and non-string entries", () => {
    localStorage.setItem(searchHistoryKey(ADA), "{not json");
    expect(readSearchHistory(ADA)).toEqual([]);
    writeSearchHistory(ADA, ["ok", "  ", 12 as unknown as string, "ok"]);
    expect(readSearchHistory(ADA)).toEqual(["ok"]);
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
    expect(readSearchHistory(ADA)).toEqual([]);
    expect(rememberSearch(ADA, "Circe")).toEqual(["Circe"]);
    expect(() => resetSearchHistory()).not.toThrow();
  });
});
