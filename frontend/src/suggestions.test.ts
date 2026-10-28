import { describe, expect, it } from "vitest";
import { i18n } from "./i18n";
import { suggestionReason } from "./suggestions";

describe("vote suggestion reasons", () => {
  it("translates each shape the server writes and passes others through", () => {
    i18n.global.locale.value = "de";
    try {
      expect(suggestionReason("On 2 shelves: ada, grace")).toBe("In 2 Regalen: ada, grace");
      expect(suggestionReason("ada wants to read it")).toBe("ada will es lesen");
      expect(suggestionReason("Like past picks: fantasy, myth")).toBe("Wie frühere Club-Bücher: fantasy, myth");
      expect(suggestionReason("Open Library rating 4.2")).toBe("Open-Library-Bewertung 4.2");
      expect(suggestionReason("Something new")).toBe("Something new");
    } finally {
      i18n.global.locale.value = "en";
    }
  });
});
