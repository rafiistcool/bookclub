import { t } from "./i18n";

/**
 * The vote suggestions explain themselves in English (`server/src/votes/suggestions.rs`).
 * Each known shape is translated; anything else shows as the server wrote it.
 */
const REASONS: ReadonlyArray<readonly [RegExp, (match: RegExpExecArray) => string]> = [
  [/^On (\d+) shelves: (.+)$/, (m) => t("vote.reasonShelves", { n: m[1], names: m[2] })],
  [/^(.+) wants to read it$/, (m) => t("vote.reasonWants", { name: m[1] })],
  [/^Like past picks: (.+)$/, (m) => t("vote.reasonLikePicks", { subjects: m[1] })],
  [/^Open Library rating (\d+(?:\.\d+)?)$/, (m) => t("vote.reasonRating", { rating: m[1] })],
];

export function suggestionReason(reason: string): string {
  for (const [pattern, translate] of REASONS) {
    const match = pattern.exec(reason);
    if (match) return translate(match);
  }
  return reason;
}
