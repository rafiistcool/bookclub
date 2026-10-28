import { describe, expect, it } from "vitest";
import { postTimeZone } from "./timezone";

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
