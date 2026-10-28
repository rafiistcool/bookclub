import type { InviteOut } from "./api/generated";

/** An invite code waiting for its reader to sign in or sign up (per tab). */
const PENDING_JOIN_KEY = "bookclub.join";

/** A pasted code or a whole invite link (`…/join/<code>`) → the code. */
export function parseInviteCode(raw: string): string {
  const trimmed = raw.trim();
  const match = /\/join\/([^/?#\s]+)/.exec(trimmed);
  let code = match ? match[1]! : trimmed;
  try {
    code = decodeURIComponent(code);
  } catch {
    /* Not percent-encoded after all. */
  }
  return code.replace(/\s+/g, "");
}

export type InviteState = "active" | "revoked" | "used_up" | "expired";

/** Why an invite no longer works, or that it does. */
export function inviteState(invite: Pick<InviteOut, "active" | "revoked" | "max_uses" | "use_count">): InviteState {
  if (invite.revoked) return "revoked";
  if (invite.active) return "active";
  if (invite.max_uses != null && invite.use_count >= invite.max_uses) return "used_up";
  return "expired";
}

export function rememberJoinCode(code: string) {
  try {
    sessionStorage.setItem(PENDING_JOIN_KEY, code);
  } catch {
    /* The code still travels in the link. */
  }
}

export function pendingJoinCode(): string | null {
  try {
    return sessionStorage.getItem(PENDING_JOIN_KEY) || null;
  } catch {
    return null;
  }
}

export function clearJoinCode() {
  try {
    sessionStorage.removeItem(PENDING_JOIN_KEY);
  } catch {
    /* Nothing to clear. */
  }
}

/** The code inside a `?next=/join/<code>` target, if that is where login leads. */
export function joinCodeFromNext(next: unknown): string | null {
  if (typeof next !== "string") return null;
  const match = /^\/join\/([^/?#]+)$/.exec(next);
  return match ? parseInviteCode(match[1]!) : null;
}
