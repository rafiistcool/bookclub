/**
 * ISBNs by the server's rules (`server/src/catalog/shared/isbn.rs`, `parse_isbn`),
 * so the scanner and its manual entry ask only for ISBNs the server accepts:
 * ISBN-10 or ISBN-13 with a valid check digit, an ISBN-13 starting with 978 or
 * 979, and ISBN-10 input converted to its 978 ISBN-13.
 *
 * One addition: typed input is NFKC-normalised first, so full-width digits (a
 * Japanese or Chinese keyboard) become ASCII. The server never sees them: what
 * goes out is the ISBN-13 these functions return.
 *
 * `extractIsbn` in `constants.ts` is something else: Discover's shape-only
 * test of whether a search looks like an ISBN.
 */

/** Typed input as the server compacts it: NFKC, whitespace and `-` removed, uppercased. */
export function normalizeIsbnInput(raw: string): string {
  // `\s` plus the separators Python's isspace() (the legacy app) also counts.
  return raw.normalize("NFKC").replace(/[\s\x1c-\x1f\x85-]/g, "").toUpperCase();
}

function isbn10Check(nine: string): string {
  let sum = 0;
  for (let i = 0; i < 9; i += 1) sum += Number(nine[i]) * (10 - i);
  const check = (11 - (sum % 11)) % 11;
  return check === 10 ? "X" : String(check);
}

function isbn13Check(twelve: string): string {
  let sum = 0;
  for (let i = 0; i < 12; i += 1) sum += Number(twelve[i]) * (i % 2 === 0 ? 1 : 3);
  return String((10 - (sum % 10)) % 10);
}

/** 13 ASCII digits, 978 or 979, check digit. */
function isIsbn13(s: string): boolean {
  return /^97[89]\d{10}$/.test(s) && isbn13Check(s.slice(0, 12)) === s[12];
}

/**
 * Manual entry: an ISBN-10 (`X` allowed as its last character) or ISBN-13 as
 * typed, with spaces and hyphens, returned as the ISBN-13; null when the
 * server would refuse it.
 */
export function parseIsbn(raw: string): string | null {
  const s = normalizeIsbnInput(raw);
  if (/^\d{9}[\dX]$/.test(s)) {
    if (isbn10Check(s.slice(0, 9)) !== s[9]) return null;
    const twelve = `978${s.slice(0, 9)}`;
    return twelve + isbn13Check(twelve);
  }
  return isIsbn13(s) ? s : null;
}

/**
 * A decoded barcode: exactly 13 digits, 978 or 979, check digit. No ISBN-10
 * (a book's barcode is an EAN-13) and no other EAN, such as a magazine's 977.
 */
export function isbnFromScan(text: string): string | null {
  return isIsbn13(text) ? text : null;
}
