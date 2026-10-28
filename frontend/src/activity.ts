import { STATUSES, starLabel, statusLabel, type Status } from "./constants";
import { t, tp } from "./i18n";
import type { ActivityItem } from "./types";

/**
 * One feed line: an i18n key whose `{actor}` and `{book}` slots the feed
 * fills with links, plus plain values for the other placeholders, and an
 * optional second line (a take, a DNF reason, a quote excerpt).
 */
export type ActivityLine = {
  key: string;
  params: Record<string, string>;
  detail: string;
};

const KINDS = new Set([
  "member_joined",
  "pick_set",
  "pick_cleared",
  "note_posted",
  "reaction",
  "nominated",
  "voted",
  "vote_closed",
  "milestone_added",
  "shelf_added",
  "shelf_moved",
  "shelf_finished",
  "shelf_dnf",
  "shelf_removed",
  "progress",
  "quote_added",
]);

// The generated type cannot describe a per-kind JSON object.
function payloadOf(item: ActivityItem): Record<string, unknown> {
  const payload = item.payload as unknown;
  return payload !== null && typeof payload === "object" ? (payload as Record<string, unknown>) : {};
}

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function count(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function status(value: unknown): string {
  return STATUSES.includes(value as Status) ? statusLabel(value as Status) : "";
}

export function activityLine(item: ActivityItem): ActivityLine {
  const payload = payloadOf(item);
  const params: Record<string, string> = {};
  let key = KINDS.has(item.kind) ? `activity.${item.kind}` : "activity.other";
  let detail = "";

  switch (item.kind) {
    case "member_joined":
      params.club = item.club?.name ?? "";
      break;
    case "reaction": {
      params.emoji = text(payload.emoji);
      const author = text(payload.author);
      if (author) params.author = author;
      else key = "activity.reactionNoAuthor";
      break;
    }
    case "vote_closed": {
      if (payload.auto === true) key = "activity.vote_closed_auto";
      const votes = count(payload.votes);
      if (votes != null) detail = tp("activity.votes", votes);
      break;
    }
    case "milestone_added":
      params.title = text(payload.title);
      break;
    case "shelf_added":
    case "shelf_moved":
      params.status = status(payload.status);
      break;
    case "shelf_finished":
      detail = [starLabel(count(payload.rating)), text(payload.take)].filter(Boolean).join(" · ");
      break;
    case "shelf_dnf":
      detail = text(payload.reason);
      break;
    case "progress":
      params.n = String(count(payload.progress) ?? 0);
      break;
    case "quote_added": {
      const excerpt = text(payload.excerpt);
      if (excerpt) detail = t("activity.quoted", { text: excerpt });
      break;
    }
  }
  return { key, params, detail };
}

/** The line as plain text, for labels and tests. */
export function activityText(item: ActivityItem): string {
  const line = activityLine(item);
  return t(line.key, {
    ...line.params,
    actor: item.actor ?? t("activity.someone"),
    book: item.book?.title ?? "",
  });
}
