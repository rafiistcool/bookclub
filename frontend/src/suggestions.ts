import { t, type Locale } from "./i18n";
import { formatRating } from "./stats";
import type { VoteReasonOut } from "./types";

/**
 * One reason of a vote suggestion, worded from its code (`VoteSuggestionOut.reason_codes`;
 * `reasons` has the same in English).
 */
export function suggestionReason(reason: VoteReasonOut, locale: Locale): string {
  switch (reason.code) {
    case "wanted":
      return reason.usernames.length >= 2
        ? t("vote.reasonShelves", { n: reason.usernames.length, names: reason.usernames.join(", ") })
        : t("vote.reasonWants", { name: reason.usernames[0] ?? "" });
    case "like_past_picks":
      return t("vote.reasonLikePicks", { subjects: reason.subjects.join(", ") });
    case "ol_rating":
      return t("vote.reasonRating", { rating: formatRating(reason.rating, locale) });
  }
}
