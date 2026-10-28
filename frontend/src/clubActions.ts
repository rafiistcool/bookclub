import type { ClubSummary } from "./types";

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
