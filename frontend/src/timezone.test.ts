import { afterEach, describe, expect, it, vi } from "vitest";
import { browserTimeZone, currentZoneName, postTimeZone, timeZoneOptions } from "./timezone";

/** The server's zone module, from this file (a variable: Vite rewrites `new URL` of a literal). */
const TZ_RS = "../../server/src/tz.rs";

/**
 * `LINKS` in server/src/tz.rs: the old names the server accepts and stores as
 * the current name. Read from disk, as Vite serves nothing outside frontend/.
 */
async function serverLinks(): Promise<Map<string, string>> {
  const fs = await vi.importActual<{ readFileSync(path: URL, encoding: "utf8"): string }>("node:fs");
  const source = fs.readFileSync(new URL(TZ_RS, import.meta.url), "utf8");
  const start = source.indexOf("static LINKS");
  const table = source.slice(start, source.indexOf("];", start));
  return new Map([...table.matchAll(/\(\s*"([^"]+)",\s*"([^"]+)",?\s*\)/g)].map((m) => [m[1], m[2]]));
}

const viewer = {
  timezone: "America/New_York",
  clubs: [{ slug: "berlin-ab12", timezone: "Europe/Berlin" }],
};

describe("post times", () => {
  it("shows a club post in its club's zone and a diary entry in the viewer's", () => {
    expect(postTimeZone({ club: { slug: "berlin-ab12" } }, viewer)).toBe("Europe/Berlin");
    expect(postTimeZone({ club: null }, viewer)).toBe("America/New_York");
    expect(postTimeZone({ club: { slug: "gone-zz99" } }, viewer)).toBe("America/New_York");
    expect(postTimeZone({ club: null }, null)).toBe("UTC");
  });
});

describe("zone names", () => {
  afterEach(() => vi.restoreAllMocks());

  it("sends the current IANA name for the old spellings ICU browsers report", () => {
    expect(currentZoneName("Asia/Calcutta")).toBe("Asia/Kolkata");
    expect(currentZoneName("Europe/Kiev")).toBe("Europe/Kyiv");
    expect(currentZoneName("America/Buenos_Aires")).toBe("America/Argentina/Buenos_Aires");
    expect(currentZoneName("Europe/Berlin")).toBe("Europe/Berlin");

    vi.spyOn(Intl.DateTimeFormat.prototype, "resolvedOptions").mockReturnValue({
      ...new Intl.DateTimeFormat().resolvedOptions(),
      timeZone: "Asia/Calcutta",
    });
    expect(browserTimeZone()).toBe("Asia/Kolkata");
  });

  it("sends the current name for every old one this ICU reports and the server links", async () => {
    const links = await serverLinks();
    expect(links.size).toBe(150);
    const intl = Intl as unknown as { supportedValuesOf: (key: string) => string[] };
    const reported = new Set(intl.supportedValuesOf("timeZone"));
    // The device's zone is ICU's name for it, which need not be in that list (CST6CDT).
    for (const name of [...links.keys(), ...links.values()]) {
      try {
        reported.add(new Intl.DateTimeFormat("en", { timeZone: name }).resolvedOptions().timeZone);
      } catch {
        // A name this ICU does not know.
      }
    }
    const wrong = [...reported].filter((name) => currentZoneName(name) !== (links.get(name) ?? name));
    expect(wrong).toEqual([]);
    expect(currentZoneName("America/Coral_Harbour")).toBe("America/Atikokan");
    expect(currentZoneName("CST6CDT")).toBe("America/Chicago");
  });

  it("offers current names in the picker, each once", () => {
    const intl = Intl as unknown as { supportedValuesOf: (key: string) => string[] };
    vi.spyOn(intl, "supportedValuesOf").mockReturnValue(["Asia/Calcutta", "Europe/Kiev", "Europe/Berlin"]);
    expect(timeZoneOptions("Asia/Kolkata")).toEqual(["Asia/Kolkata", "Europe/Berlin", "Europe/Kyiv", "UTC"]);
  });
});
