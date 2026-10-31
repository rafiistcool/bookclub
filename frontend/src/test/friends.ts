// Complete `GET /api/friends` objects for tests (FRIENDS, #57).
import type {
  BlockedOut,
  FriendClubInviteOut,
  FriendOut,
  FriendReadingOut,
  FriendRequestOut,
  FriendsOut,
} from "../api/generated";
import { bookOut } from "./fixtures";

/** A stable id per name, so lists keyed by `user.id` stay apart. */
function idFor(username: string): number {
  return [...username].reduce((sum, char, at) => sum + char.charCodeAt(0) * (at + 1), 100);
}

export function friendsOut(overrides: Partial<FriendsOut> = {}): FriendsOut {
  return { friends: [], incoming: [], outgoing: [], blocked: [], club_invites: [], ...overrides };
}

export function reading(id: number, progress: number | null = null, title = `Book ${id}`): FriendReadingOut {
  return { book: bookOut({ id, title }), progress };
}

export function friendOut(username: string, overrides: Partial<FriendOut> = {}): FriendOut {
  return {
    user: { id: idFor(username), username, avatar_url: null },
    since: "2026-10-01T09:00:00Z",
    currently_reading_count: 0,
    currently_reading: [],
    ...overrides,
  };
}

export function friendRequest(username: string, overrides: Partial<FriendRequestOut> = {}): FriendRequestOut {
  return {
    user: { id: idFor(username), username, avatar_url: null },
    created_at: "2026-10-20T09:00:00Z",
    ...overrides,
  };
}

export function blockedOut(username: string, overrides: Partial<BlockedOut> = {}): BlockedOut {
  return { username, created_at: "2026-10-21T09:00:00Z", ...overrides };
}

export function friendClubInvite(overrides: Partial<FriendClubInviteOut> = {}): FriendClubInviteOut {
  return {
    id: 1,
    club: { slug: "readers-ab12", name: "Readers" },
    invited_by: "grace",
    code: "7K3M9QX2HD",
    created_at: "2026-10-22T09:00:00Z",
    expires_at: "2026-11-21T09:00:00Z",
    ...overrides,
  };
}
