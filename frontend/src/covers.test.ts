import { describe, expect, it } from "vitest";
import { bookColorKey, coverSources, sameOriginPath } from "./covers";

const book = (cover_url: string | null, cover_thumb_url: string | null) => ({
  id: 4,
  title: "Circe",
  cover_url,
  cover_thumb_url,
});

describe("cover sources", () => {
  it("takes the thumbnail for small covers and the large picture full width", () => {
    const both = book("/media/abc/l", "/media/abc/s");
    expect(coverSources(both, "tile")).toEqual(["/media/abc/s", "/media/abc/l"]);
    expect(coverSources(both, "xs")).toEqual(["/media/abc/s", "/media/abc/l"]);
    expect(coverSources(both, "fluid")).toEqual(["/media/abc/l", "/media/abc/s"]);
    expect(coverSources(book("/media/abc/l", null), "md")).toEqual(["/media/abc/l"]);
    expect(coverSources(book(null, null), "md")).toEqual([]);
  });

  it("never lets the browser ask another host for a cover", () => {
    for (const url of [
      "https://covers.openlibrary.org/b/id/1-L.jpg",
      "http://books.google.com/books/content?id=x",
      "//attacker.example/pixel.gif",
      "/\\attacker.example/pixel.gif",
      // The URL parser drops tabs and newlines, so each of these reads as //attacker.example.
      "/\t/attacker.example/x.gif",
      "/\n/attacker.example/x.gif",
      "/\r/attacker.example/x.gif",
      "/media/\u0000x",
      "javascript:alert(1)",
      "media/abc/l",
      "",
    ]) {
      expect(sameOriginPath(url)).toBeNull();
      expect(coverSources(book(url, url), "md")).toEqual([]);
    }
    expect(sameOriginPath(" /media/abc/s ")).toBe("/media/abc/s");
  });

  it("colours a cover-less book by its Open Library key, else by its work id", () => {
    expect(bookColorKey({ id: 4, ol_work_key: "/works/OL1W", title: "Circe" })).toBe("/works/OL1W");
    expect(bookColorKey({ id: 4, ol_work_key: null, title: "Circe" })).toBe("work:4");
    expect(bookColorKey({ title: "Circe", authors: "Madeline Miller" })).toBe("Circe:Madeline Miller");
  });
});
