import { describe, expect, it } from "vitest";
import { i18n } from "./i18n";
import { suggestionReason } from "./suggestions";

describe("vote suggestion reasons", () => {
  it("words each reason code with its values", () => {
    i18n.global.locale.value = "de";
    try {
      expect(suggestionReason({ code: "wanted", usernames: ["ada", "grace"] }, "de")).toBe("In 2 Regalen: ada, grace");
      expect(suggestionReason({ code: "wanted", usernames: ["ada"] }, "de")).toBe("ada will es lesen");
      expect(suggestionReason({ code: "like_past_picks", subjects: ["fantasy", "myth"] }, "de")).toBe(
        "Wie frühere Club-Bücher: fantasy, myth",
      );
      expect(suggestionReason({ code: "ol_rating", rating: 4.2 }, "de")).toBe("Open-Library-Bewertung 4,2");
    } finally {
      i18n.global.locale.value = "en";
    }
    expect(suggestionReason({ code: "wanted", usernames: ["ada", "grace", "tom"] }, "en")).toBe(
      "On 3 shelves: ada, grace, tom",
    );
    expect(suggestionReason({ code: "ol_rating", rating: 4.44 }, "en")).toBe("Open Library rating 4.4");
  });
});
