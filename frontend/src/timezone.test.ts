import { afterEach, describe, expect, it, vi } from "vitest";
import { browserTimeZone, currentZoneName, postTimeZone, timeZoneOptions } from "./timezone";

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

  it("offers current names in the picker, each once", () => {
    const intl = Intl as unknown as { supportedValuesOf: (key: string) => string[] };
    vi.spyOn(intl, "supportedValuesOf").mockReturnValue(["Asia/Calcutta", "Europe/Kiev", "Europe/Berlin"]);
    expect(timeZoneOptions("Asia/Kolkata")).toEqual(["Asia/Kolkata", "Europe/Berlin", "Europe/Kyiv", "UTC"]);
  });
});
