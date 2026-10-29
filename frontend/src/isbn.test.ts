import { describe, expect, it } from "vitest";
import { isbnFromScan, normalizeIsbnInput, parseIsbn } from "./isbn";

// The vectors of server/src/catalog/shared/isbn.rs (its tests), so the SPA
// asks for exactly the ISBNs the server accepts.

describe("parseIsbn", () => {
  it("turns ISBN-10 input into the 978 ISBN-13", () => {
    expect(parseIsbn("0-8044-2957-X")).toBe("9780804429573");
    expect(parseIsbn("080442957x")).toBe("9780804429573");
    expect(parseIsbn(" 3 89561 419 X ")).toBe("9783895614194");
    expect(parseIsbn("3518368818")).toBe("9783518368817");
    // The legacy parity fixture: all zeros is checksum-valid.
    expect(parseIsbn("0000000000")).toBe("9780000000002");
  });

  it("keeps a valid ISBN-13, 978 and 979", () => {
    expect(parseIsbn("978-0-316-76948-8")).toBe("9780316769488");
    expect(parseIsbn("9780316556347")).toBe("9780316556347");
    expect(parseIsbn("979-10-90636-07-1")).toBe("9791090636071");
  });

  it("passes an ISMN (9790, sheet music), as the server does", () => {
    expect(parseIsbn("979-0-2600-0043-8")).toBe("9790260000438");
  });

  it("refuses bad check digits and shapes", () => {
    for (const raw of [
      "0804429578", // wrong ISBN-10 check digit
      "9780316769487", // wrong ISBN-13 check digit
      "9771234567003", // a valid EAN, not an ISBN prefix
      "X804429573", // X only as the last ISBN-10 character
      "08044X9573",
      "97803167694888", // 14 digits
      "978031676948", // 12 digits
      "",
      "isbn",
    ]) {
      expect(parseIsbn(raw), raw).toBeNull();
    }
  });

  it("reads full-width digits, hyphens and X as ASCII", () => {
    // The server refuses "978031676948８"; the SPA normalises it and sends ASCII.
    expect(parseIsbn("978031676948８")).toBe("9780316769488");
    expect(parseIsbn("９７８－０－３１６－７６９４８－８")).toBe("9780316769488");
    expect(parseIsbn("080442957Ｘ")).toBe("9780804429573");
    expect(parseIsbn("978　0316769488")).toBe("9780316769488");
  });
});

describe("normalizeIsbnInput", () => {
  it("drops whitespace and hyphens and uppercases x", () => {
    expect(normalizeIsbnInput(" 0-8044\t2957-x\n")).toBe("080442957X");
    expect(normalizeIsbnInput("０８０４４２９５７ｘ")).toBe("080442957X");
  });
});

describe("isbnFromScan", () => {
  it("takes a 978 or 979 EAN-13 with its check digit", () => {
    expect(isbnFromScan("9780316769488")).toBe("9780316769488");
    expect(isbnFromScan("9791090636071")).toBe("9791090636071");
    expect(isbnFromScan("9790260000438")).toBe("9790260000438");
  });

  it("ignores other EANs, bad check digits and every ISBN-10", () => {
    for (const text of [
      "9771234567003", // a magazine's EAN
      "4006381333931", // a grocery EAN
      "9780316769487", // wrong check digit
      "3518368818", // an ISBN-10 that parseIsbn takes
      "080442957X",
      "978-0-316-76948-8", // never hyphenated in a barcode
      " 9780316769488",
      "97803167694888",
      "",
    ]) {
      expect(isbnFromScan(text), text).toBeNull();
    }
    expect(parseIsbn("3518368818")).toBe("9783518368817");
  });
});
