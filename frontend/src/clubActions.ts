import type { ClubRef, ClubSummary } from "./types";

export type ClubAction = "pick" | "nominate";

/**
 * The clubs where the viewer may do this with a book. Setting the pick is for
 * admins and the owner; every member can nominate. No clubs, no club actions.
 */
export function clubsFor(action: ClubAction, clubs: readonly ClubSummary[] | null | undefined): ClubSummary[] {
  const all = [...(clubs ?? [])];
  if (action === "nominate") return all;
  return all.filter((club) => club.role === "owner" || club.role === "admin");
}

/**
 * The clubs where the viewer could still make this book the pick: those they
 * may set it in, less those whose pick it already is. None left, and the
 * "set as pick" row goes; the pick badge says the rest.
 */
export function pickTargets(
  clubs: readonly ClubSummary[] | null | undefined,
  currentIn: readonly ClubRef[] | null | undefined,
): ClubSummary[] {
  const current = new Set((currentIn ?? []).map((club) => club.slug));
  return clubsFor("pick", clubs).filter((club) => !current.has(club.slug));
}
