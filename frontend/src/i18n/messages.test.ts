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

describe("locale catalogs", () => {
  it("keeps the German catalog in lockstep with English", () => {
    expect(keysOf(de).sort()).toEqual(keysOf(en).sort());
  });
});
