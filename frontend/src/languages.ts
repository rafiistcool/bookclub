/**
 * Languages an edition can be in. The catalog stores ISO 639-2/B codes
 * (`ger`, `fre`); the browser names them from the two-letter code.
 */
const LANGUAGES: ReadonlyArray<readonly [code: string, iso1: string]> = [
  ["ger", "de"],
  ["eng", "en"],
  ["fre", "fr"],
  ["spa", "es"],
  ["ita", "it"],
  ["dut", "nl"],
  ["por", "pt"],
  ["swe", "sv"],
  ["dan", "da"],
  ["nor", "no"],
  ["fin", "fi"],
  ["pol", "pl"],
  ["cze", "cs"],
  ["hun", "hu"],
  ["tur", "tr"],
  ["gre", "el"],
  ["rus", "ru"],
  ["ukr", "uk"],
  ["ara", "ar"],
  ["heb", "he"],
  ["jpn", "ja"],
  ["chi", "zh"],
  ["kor", "ko"],
  ["lat", "la"],
];

/** "Deutsch" for `ger` in German, "German" in English; the code itself when the browser cannot say. */
export function languageName(code: string, locale: string): string {
  const iso1 = LANGUAGES.find(([known]) => known === code)?.[1];
  if (!iso1) return code;
  try {
    return new Intl.DisplayNames([locale], { type: "language" }).of(iso1) ?? code;
  } catch {
    return code;
  }
}

/** The picker's options, by name in the UI language; `current` is kept even when it is not listed. */
export function languageOptions(locale: string, current?: string | null): { code: string; name: string }[] {
  const codes = LANGUAGES.map(([code]) => code);
  if (current && !codes.includes(current)) codes.push(current);
  return codes
    .map((code) => ({ code, name: languageName(code, locale) }))
    .sort((a, b) => a.name.localeCompare(b.name, locale));
}
