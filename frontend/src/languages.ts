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

/** Codes the browser names but that say nothing about an edition's language. */
const NOT_A_LANGUAGE = new Set(["und", "mul", "zxx", "mis"]);

/** False for the codes that say nothing about a language (`und`, `mul`, `zxx`, `mis`): a label leaves them out. */
export function isRealLanguage(code: string): boolean {
  return !NOT_A_LANGUAGE.has(code);
}

/**
 * "Deutsch" for `ger` in German, "German" in English. A code the list does not
 * have (Open Library's `rum`, `nob`) is named by the browser when it knows it;
 * otherwise the code itself.
 */
export function languageName(code: string, locale: string): string {
  const iso1 = LANGUAGES.find(([known]) => known === code)?.[1];
  if (!iso1 && (NOT_A_LANGUAGE.has(code) || !/^[a-z]{3}$/.test(code))) return code;
  try {
    return new Intl.DisplayNames([locale], { type: "language" }).of(iso1 ?? code) ?? code;
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
