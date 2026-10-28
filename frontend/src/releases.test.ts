import { afterEach, describe, expect, it } from "vitest";
import { i18n } from "./i18n";
import { localDay, releaseNote } from "./releases";

describe("release notes", () => {
  afterEach(() => {
    i18n.global.locale.value = "en";
  });

  it("says when an upcoming title comes out, and since when a recent one is out", () => {
    expect(releaseNote("2026-11-12", "en", "2026-10-28")).toBe("Out 12 Nov 2026");
    expect(releaseNote("2026-10-03", "en", "2026-10-28")).toBe("Out since 3 Oct 2026");
    expect(releaseNote("2026-10-28", "en", "2026-10-28")).toBe("Out since 28 Oct 2026");
    i18n.global.locale.value = "de";
    expect(releaseNote("2026-11-12", "de", "2026-10-28")).toBe("Erscheint am 12. Nov. 2026");
  });

  it("says nothing without a usable date", () => {
    expect(releaseNote(null, "en")).toBe("");
    expect(releaseNote("2026-13-45", "en")).toBe("");
    expect(releaseNote("12.11.2026", "en")).toBe("");
  });

  it("reads today in the device's zone", () => {
    expect(localDay(new Date(2026, 0, 5, 23, 30))).toBe("2026-01-05");
  });
});
