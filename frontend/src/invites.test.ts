import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearJoinCode,
  inviteState,
  joinCodeFromNext,
  parseInviteCode,
  pendingJoinCode,
  rememberJoinCode,
} from "./invites";
import { invite } from "./test/fixtures";
import { blockedStorage, memoryStorage } from "./test/storage";

describe("invite codes", () => {
  it("takes a bare code or a whole invite link", () => {
    expect(parseInviteCode(" 7K3M9QX2HD ")).toBe("7K3M9QX2HD");
    expect(parseInviteCode("https://books.example/join/7K3M9QX2HD")).toBe("7K3M9QX2HD");
    expect(parseInviteCode("https://books.example/join/7K3M9QX2HD?utm=x#y")).toBe("7K3M9QX2HD");
    expect(parseInviteCode("7K3M 9QX2HD")).toBe("7K3M9QX2HD");
  });

  it("reads the code from a login target that leads back to the invite", () => {
    expect(joinCodeFromNext("/join/7K3M9QX2HD")).toBe("7K3M9QX2HD");
    expect(joinCodeFromNext("/c/readers-ab12")).toBeNull();
    expect(joinCodeFromNext(undefined)).toBeNull();
  });

  it("says why an invite no longer works", () => {
    expect(inviteState(invite())).toBe("active");
    expect(inviteState(invite({ active: false, revoked: true }))).toBe("revoked");
    expect(inviteState(invite({ active: false, max_uses: 1, use_count: 1 }))).toBe("used_up");
    expect(inviteState(invite({ active: false, max_uses: null, use_count: 4 }))).toBe("expired");
  });
});

describe("a code waiting for sign-in", () => {
  afterEach(() => vi.unstubAllGlobals());

  beforeEach(() => vi.stubGlobal("sessionStorage", memoryStorage()));

  it("waits in this tab until it is used", () => {
    expect(pendingJoinCode()).toBeNull();
    rememberJoinCode("7K3M9QX2HD");
    expect(pendingJoinCode()).toBe("7K3M9QX2HD");
    clearJoinCode();
    expect(pendingJoinCode()).toBeNull();
  });

  it("works without storage", () => {
    vi.stubGlobal("sessionStorage", blockedStorage());
    expect(() => rememberJoinCode("7K3M9QX2HD")).not.toThrow();
    expect(pendingJoinCode()).toBeNull();
    expect(() => clearJoinCode()).not.toThrow();
  });
});
