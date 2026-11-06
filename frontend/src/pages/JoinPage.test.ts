import { flushPromises, mount, RouterLinkStub } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { clubOut, user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";
import type { InvitePreview } from "../types";

const api = { previewInvite: vi.fn(), acceptInvite: vi.fn(), config: vi.fn() };

vi.mock("../api/client", () => {
  class ApiError extends Error {
    status: number;
    detail: unknown;
    constructor(message: string, status: number, detail: unknown) {
      super(message);
      this.status = status;
      this.detail = detail;
    }
  }
  return { ApiError, api: new Proxy({}, { get: (_target, key: string) => (api as Record<string, unknown>)[key] }) };
});

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { pendingJoinCode } from "../invites";
import { LAST_CLUB_KEY } from "../stores/clubs";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import JoinPage from "./JoinPage.vue";

const CLUB = { slug: "readers-ab12", name: "Readers" };
const PICK = {
  title: "Circe",
  authors: "Madeline Miller",
  cover_url: "/media/circe/l",
  cover_thumb_url: "/media/circe/s",
  spine_color: null,
};

function invite(overrides: Partial<InvitePreview> = {}): InvitePreview {
  return {
    club: CLUB,
    kind: "club",
    personal: false,
    inviter: { username: "lena", avatar_url: "/media/lena/a" },
    pick: PICK,
    member_count: 4,
    full: false,
    expires_at: "2026-11-13T12:00:00Z",
    you_are_member: null,
    for_you: null,
    ...overrides,
  };
}

/** The same invite as a signed-in non-member sees it. */
function signedInInvite(overrides: Partial<InvitePreview> = {}): InvitePreview {
  return invite({ you_are_member: false, ...overrides });
}

async function mountJoin(signedIn: boolean, code = "7K3M9QX2HD") {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = signedIn ? user({ clubs: [] }) : null;
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/join/:code", component: JoinPage },
      { path: "/:rest(.*)*", component: { template: "<p>elsewhere</p>" } },
    ],
  });
  await router.push(`/join/${code}`);
  const wrapper = mount(JoinPage, {
    global: { plugins: [pinia, router, i18n], stubs: { RouterLink: RouterLinkStub } },
  });
  await flushPromises();
  return { wrapper, router };
}

function links(wrapper: Awaited<ReturnType<typeof mountJoin>>["wrapper"]) {
  return wrapper.findAllComponents(RouterLinkStub).map((link) => ({ to: link.props("to"), text: link.text() }));
}

describe("JoinPage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("sessionStorage", memoryStorage());
  });

  afterEach(() => vi.unstubAllGlobals());

  it("shows who invites, to which club, what it reads and how many are in it", async () => {
    api.previewInvite.mockResolvedValue(invite());
    const { wrapper } = await mountJoin(false);
    expect(api.previewInvite).toHaveBeenCalledWith("7K3M9QX2HD");
    expect(wrapper.get(".join-title").text()).toBe("lena invites you to Readers");
    expect(wrapper.get(".join-avatar img").attributes("src")).toBe("/media/lena/a");
    expect(wrapper.get(".join-pick img").attributes("src")).toBe("/media/circe/l");
    expect(wrapper.get(".join-pick figcaption").text()).toContain("Circe");
    expect(wrapper.get(".join-pick figcaption").text()).toContain("Madeline Miller");
    expect(wrapper.get(".join-members").text()).toBe("4 members");
  });

  it("names the club alone without an inviter, and shows no book without a pick", async () => {
    api.previewInvite.mockResolvedValue(invite({ inviter: null, pick: null, member_count: 1 }));
    const { wrapper } = await mountJoin(false);
    expect(wrapper.get(".join-title").text()).toBe("You're invited to Readers");
    expect(wrapper.find(".join-avatar").exists()).toBe(false);
    expect(wrapper.find(".join-pick").exists()).toBe(false);
    expect(wrapper.get(".join-members").text()).toBe("1 member");
  });

  it("keeps the code for a reader who still has to sign in or up, with one primary action", async () => {
    api.previewInvite.mockResolvedValue(invite());
    const { wrapper } = await mountJoin(false);
    expect(api.acceptInvite).not.toHaveBeenCalled();
    expect(pendingJoinCode()).toBe("7K3M9QX2HD");
    expect(links(wrapper)).toContainEqual({
      to: { path: "/signup", query: { code: "7K3M9QX2HD" } },
      text: "Create account and join",
    });
    expect(links(wrapper)).toContainEqual({
      to: { path: "/login", query: { next: "/join/7K3M9QX2HD" } },
      text: "I already have an account",
    });
    expect(wrapper.findAll(".btn-primary")).toHaveLength(1);
    expect(wrapper.find(".join-close").exists()).toBe(false);
  });

  it("joins with one tap once signed in and opens the club with its welcome", async () => {
    sessionStorage.setItem("bookclub.join", "7K3M9QX2HD");
    api.previewInvite.mockResolvedValue(signedInInvite());
    api.acceptInvite.mockResolvedValue(clubOut({ ...CLUB, member_count: 5 }));
    const { wrapper, router } = await mountJoin(true);
    expect(wrapper.get(".join-button").text()).toBe("Join");
    expect(wrapper.findAll(".btn")).toHaveLength(1);
    await wrapper.get(".join-button").trigger("click");
    await flushPromises();
    expect(api.acceptInvite).toHaveBeenCalledWith("7K3M9QX2HD");
    expect(router.currentRoute.value.path).toBe("/c/readers-ab12");
    expect(router.options.history.state.joined).toBe(true);
    expect(useSession().user?.clubs.map((club) => club.slug)).toEqual(["readers-ab12"]);
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBe("readers-ab12");
    expect(pendingJoinCode()).toBeNull();
  });

  it("goes straight to the club for a member, with a toast", async () => {
    api.previewInvite.mockResolvedValue(signedInInvite({ you_are_member: true }));
    const { router } = await mountJoin(true);
    expect(router.currentRoute.value.path).toBe("/c/readers-ab12");
    expect(router.options.history.state.joined).toBeUndefined();
    expect(useToast().items.map((toast) => toast.message)).toEqual(["You're already in Readers"]);
    expect(api.acceptInvite).not.toHaveBeenCalled();
  });

  it("goes to the club when the server says the reader is already in it", async () => {
    api.previewInvite.mockResolvedValue(signedInInvite());
    api.acceptInvite.mockRejectedValue(new ApiError("You are already in this club", 409, "You are already in this club"));
    const { wrapper, router } = await mountJoin(true);
    await wrapper.get(".join-button").trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.path).toBe("/c/readers-ab12");
  });

  it("says a club is full before anyone makes an account for it", async () => {
    api.previewInvite.mockResolvedValue(invite({ full: true, member_count: 20 }));
    const { wrapper } = await mountJoin(false);
    expect(wrapper.get(".join-dead-title").text()).toBe("Readers is full");
    expect(wrapper.get(".join-dead .lede").text()).toBe("It already has 20 members. Let lena know.");
    expect(links(wrapper).some(({ to }) => typeof to === "object" && to.path === "/signup")).toBe(false);
    expect(wrapper.find(".btn-primary").exists()).toBe(false);
    expect(pendingJoinCode()).toBeNull();
  });

  it("shows the mapped error when the club fills up meanwhile", async () => {
    api.previewInvite.mockResolvedValue(signedInInvite());
    api.acceptInvite.mockRejectedValue(new ApiError("This club is full", 409, "This club is full"));
    const { wrapper, router } = await mountJoin(true);
    await wrapper.get(".join-button").trigger("click");
    await flushPromises();
    expect(wrapper.get(".error").text()).toBe("This club is full");
    expect(router.currentRoute.value.path).toBe("/join/7K3M9QX2HD");
  });

  it("offers only signing in for a personal invite", async () => {
    api.previewInvite.mockResolvedValue(invite({ personal: true }));
    const { wrapper } = await mountJoin(false);
    expect(wrapper.get(".join-personal").text()).toBe(
      "Personal invite: sign in with the account it's for, and you join right after.",
    );
    expect(links(wrapper)).toContainEqual({
      to: { path: "/login", query: { next: "/join/7K3M9QX2HD" } },
      text: "Sign in and join",
    });
    expect(links(wrapper).some(({ to }) => typeof to === "object" && to.path === "/signup")).toBe(false);
    expect(pendingJoinCode()).toBeNull();
  });

  it("joins a personal invite made for the reader", async () => {
    api.previewInvite.mockResolvedValue(signedInInvite({ personal: true, for_you: true }));
    api.acceptInvite.mockResolvedValue(clubOut({ ...CLUB, member_count: 5 }));
    const { wrapper, router } = await mountJoin(true, "7k3m9-qx2hd");
    await wrapper.get(".join-button").trigger("click");
    await flushPromises();
    expect(api.acceptInvite).toHaveBeenCalledWith("7k3m9-qx2hd");
    expect(router.currentRoute.value.path).toBe("/c/readers-ab12");
  });

  it("says when a personal invite is someone else's", async () => {
    api.previewInvite.mockResolvedValue(signedInInvite({ personal: true, for_you: false }));
    const { wrapper } = await mountJoin(true);
    expect(wrapper.get(".join-dead-title").text()).toBe("This invite is for someone else");
    expect(links(wrapper)).toContainEqual({ to: "/", text: "Home" });
    expect(wrapper.find(".join-button").exists()).toBe(false);
    expect(api.acceptInvite).not.toHaveBeenCalled();
  });

  it("explains a link that no longer works and where to go instead", async () => {
    api.previewInvite.mockRejectedValue(new ApiError("That invite code is not valid", 404, "That invite code is not valid"));
    const signedIn = await mountJoin(true);
    expect(signedIn.wrapper.get(".join-dead-title").text()).toBe("This link no longer works");
    expect(signedIn.wrapper.get(".join-dead .lede").text()).toBe("Ask whoever sent it for a new one.");
    expect(links(signedIn.wrapper)).toContainEqual({ to: "/clubs", text: "Your clubs" });
    expect(signedIn.wrapper.find(".join-button").exists()).toBe(false);

    const signedOut = await mountJoin(false);
    expect(links(signedOut.wrapper)).toContainEqual({ to: "/login", text: "Sign in" });
  });

  it("offers another try when the invite could not be checked", async () => {
    api.previewInvite.mockRejectedValueOnce(new ApiError("Network error", 0, null));
    api.previewInvite.mockResolvedValueOnce(invite());
    const { wrapper } = await mountJoin(false);
    expect(wrapper.get(".join-dead-title").text()).toBe("Could not check that invite");
    await wrapper.get(".join-dead button").trigger("click");
    await flushPromises();
    expect(wrapper.get(".join-title").text()).toBe("lena invites you to Readers");
  });

  it("sends a signed-in reader home from a sign-up invite", async () => {
    api.previewInvite.mockResolvedValue(
      invite({ club: null, kind: "signup", inviter: null, pick: null, member_count: null }),
    );
    const { router } = await mountJoin(true);
    expect(router.currentRoute.value.path).toBe("/");
    expect(useToast().items.map((toast) => toast.message)).toEqual([
      "That invite is for a new account, and you already have one.",
    ]);
  });

  it("offers an account for a sign-up invite", async () => {
    api.previewInvite.mockResolvedValue(
      invite({ club: null, kind: "signup", inviter: null, pick: null, member_count: null }),
    );
    const { wrapper } = await mountJoin(false);
    expect(wrapper.get(".join-title").text()).toBe("You're invited");
    expect(links(wrapper)).toContainEqual({
      to: { path: "/signup", query: { code: "7K3M9QX2HD" } },
      text: "Create an account",
    });
    expect(pendingJoinCode()).toBe("7K3M9QX2HD");
  });
});
