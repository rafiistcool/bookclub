import { afterEach, describe, expect, it } from "vitest";
import { translateApiError } from "./apiErrors";
import { i18n } from "./index";

describe("translateApiError", () => {
  afterEach(() => {
    i18n.global.locale.value = "en";
  });

  it("translates the server's exact English details", () => {
    i18n.global.locale.value = "de";
    expect(translateApiError("No such club")).toBe("Diesen Club gibt es nicht, oder du bist kein Mitglied");
    expect(translateApiError("That book is not in the catalog")).toBe("Dieses Buch ist nicht im Katalog");
    expect(translateApiError("Slow down a little. Try again later.")).toBe(
      "Etwas langsamer, bitte. Versuch es später nochmal.",
    );
  });

  it("fills the wait into the busy-library template", () => {
    i18n.global.locale.value = "de";
    expect(translateApiError("The library is busy. Try again in 20 seconds.")).toBe(
      "Die Bibliothek ist gerade ausgelastet. Versuch es in 20 Sekunden nochmal.",
    );
    i18n.global.locale.value = "en";
    expect(translateApiError("The library is busy. Try again in 7 seconds.")).toBe(
      "The library is busy. Try again in 7 seconds.",
    );
  });

  it("names the field of a refused correction by its label", () => {
    expect(translateApiError("Invalid value for publication_year")).toBe("That value of “Year published” cannot be used");
    i18n.global.locale.value = "de";
    expect(translateApiError("Invalid value for pages")).toBe("Dieser Wert für „Seiten“ geht nicht");
    expect(translateApiError("Invalid value for shelf_mark")).toBe("Dieser Wert für „shelf_mark“ geht nicht");
    expect(translateApiError("This request was already decided")).toBe("Über diese Anfrage wurde schon entschieden");
    expect(translateApiError("This book has a cover now")).toBe("Dieses Buch hat inzwischen ein Cover.");
  });

  it("shows details it does not know as the server wrote them", () => {
    i18n.global.locale.value = "de";
    expect(translateApiError("Keep the take to 140 characters")).toBe("Keep the take to 140 characters");
    expect(translateApiError("No such club ")).toBe("No such club ");
  });
});
