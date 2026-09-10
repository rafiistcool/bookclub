import { describe, expect, it } from "vitest";
import de from "./de.json";
import en from "./en.json";

function keysOf(value: unknown, prefix = ""): string[] {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return prefix ? [prefix] : [];
  }
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) =>
    keysOf(child, prefix ? `${prefix}.${key}` : key),
  );
}

function atPath(root: unknown, path: string): unknown {
  return path.split(".").reduce<unknown>((acc, key) => {
    if (acc && typeof acc === "object" && !Array.isArray(acc)) {
      return (acc as Record<string, unknown>)[key];
    }
    return undefined;
  }, root);
}

function placeholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();
}

describe("locale catalogs", () => {
  it("keeps the German catalog in lockstep with English", () => {
    expect(keysOf(de).sort()).toEqual(keysOf(en).sort());
  });

  it("does not keep the unused favorites.marked key", () => {
    expect(keysOf(en)).not.toContain("favorites.marked");
    expect(keysOf(de)).not.toContain("favorites.marked");
    expect(keysOf(en)).toContain("favorites.markedAt");
  });

  it("prefers Favoriten over Lieblinge in German favourites copy", () => {
    const flagged = [
      de.favorites.emptyOwn,
      de.favorites.added,
      de.favorites.removed,
      de.favorites.full,
      de.favorites.settingsBlurb,
      de.favorites.cleared,
      de.errors.tooManyFavourites,
      de.errors.favouritesFull,
    ];
    for (const text of flagged) {
      expect(text).toMatch(/Favorit/);
      expect(text).not.toMatch(/Lieblinge/);
    }
    expect(de.favorites.heading).toBe("Lieblingsbücher");
  });

  it("keeps {placeholder} sets and | plural arity in lockstep", () => {
    for (const key of keysOf(en)) {
      const left = atPath(en, key);
      const right = atPath(de, key);
      expect(typeof left, key).toBe("string");
      expect(typeof right, key).toBe("string");
      expect(placeholders(left as string), key).toEqual(placeholders(right as string));
      expect((left as string).split("|").length, key).toBe((right as string).split("|").length);
    }
  });
});
