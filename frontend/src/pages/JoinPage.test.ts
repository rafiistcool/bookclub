import { flushPromises, mount, RouterLinkStub } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { clubOut, user } from "../test/fixtures";
import { friendClubInvite, friendsOut } from "../test/friends";
import { memoryStorage } from "../test/storage";

const api = { previewInvite: vi.fn(), acceptInvite: vi.fn(), config: vi.fn(), friends: vi.fn() };

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
import JoinPage from "./JoinPage.vue";

const CLUB = { slug: "readers-ab12", name: "Readers" };

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

describe("JoinPage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("sessionStorage", memoryStorage());
  });

  afterEach(() => vi.unstubAllGlobals());

  it("keeps the code for a reader who still has to sign in or up", async () => {
    api.previewInvite.mockResolvedValue({ club: CLUB, kind: "club", personal: false });
    const { wrapper } = await mountJoin(false);
    expect(api.previewInvite).toHaveBeenCalledWith("7K3M9QX2HD");
    expect(wrapper.get(".join-club").text()).toBe("Readers");
    expect(api.acceptInvite).not.toHaveBeenCalled();
    expect(pendingJoinCode()).toBe("7K3M9QX2HD");
    const links = wrapper.findAllComponents(RouterLinkStub).map((link) => link.props("to"));
    expect(links).toContainEqual({ path: "/signup", query: { code: "7K3M9QX2HD" } });
    expect(links).toContainEqual({ path: "/login", query: { next: "/join/7K3M9QX2HD" } });
  });

  it("joins with one tap once signed in and opens the club", async () => {
    sessionStorage.setItem("bookclub.join", "7K3M9QX2HD");
    api.previewInvite.mockResolvedValue({ club: CLUB, kind: "club", personal: false });
    api.acceptInvite.mockResolvedValue(clubOut({ ...CLUB, member_count: 4 }));
    const { wrapper, router } = await mountJoin(true);
    await wrapper.get("button.btn-primary").trigger("click");
    await flushPromises();
    expect(api.acceptInvite).toHaveBeenCalledWith("7K3M9QX2HD");
    expect(router.currentRoute.value.path).toBe("/c/readers-ab12");
    expect(useSession().user?.clubs.map((club) => club.slug)).toEqual(["readers-ab12"]);
    expect(localStorage.getItem(LAST_CLUB_KEY)).toBe("readers-ab12");
    expect(pendingJoinCode()).toBeNull();
  });

  it("goes to the club when the reader is already in it", async () => {
    api.previewInvite.mockResolvedValue({ club: CLUB, kind: "club", personal: false });
    api.acceptInvite.mockRejectedValue(new ApiError("You are already in this club", 409, "You are already in this club"));
    const { wrapper, router } = await mountJoin(true);
    await wrapper.get("button.btn-primary").trigger("click");
    await flushPromises();
    expect(router.currentRoute.value.path).toBe("/c/readers-ab12");
  });

  it("shows the mapped error of a full club", async () => {
    api.previewInvite.mockResolvedValue({ club: CLUB, kind: "club", personal: false });
    api.acceptInvite.mockRejectedValue(new ApiError("This club is full", 409, "This club is full"));
    const { wrapper, router } = await mountJoin(true);
    await wrapper.get("button.btn-primary").trigger("click");
    await flushPromises();
    expect(wrapper.get(".error").text()).toBe("This club is full");
    expect(router.currentRoute.value.path).toBe("/join/7K3M9QX2HD");
  });

  it("offers only signing in for a personal invite", async () => {
    api.previewInvite.mockResolvedValue({ club: CLUB, kind: "club", personal: true });
    const { wrapper } = await mountJoin(false);
    expect(wrapper.get(".lede").text()).toBe(
      "This is a personal invite. Sign in with the account it was made for, and you join right after.",
    );
    const links = wrapper.findAllComponents(RouterLinkStub).map((link) => link.props("to"));
    expect(links).toContainEqual({ path: "/login", query: { next: "/join/7K3M9QX2HD" } });
    expect(links.some((to) => typeof to === "object" && to.path === "/signup")).toBe(false);
    expect(pendingJoinCode()).toBeNull();
    expect(api.friends).not.toHaveBeenCalled();
  });

  it("joins a personal invite made for the reader", async () => {
    api.previewInvite.mockResolvedValue({ club: CLUB, kind: "club", personal: true });
    api.friends.mockResolvedValue(friendsOut({ club_invites: [friendClubInvite({ code: "7K3M9QX2HD" })] }));
    api.acceptInvite.mockResolvedValue(clubOut({ ...CLUB, member_count: 4 }));
    const { wrapper, router } = await mountJoin(true, "7k3m9-qx2hd");
    await wrapper.get("button.btn-primary").trigger("click");
    await flushPromises();
    expect(api.acceptInvite).toHaveBeenCalledWith("7k3m9-qx2hd");
    expect(router.currentRoute.value.path).toBe("/c/readers-ab12");
  });

  it("says when a personal invite is someone else's", async () => {
    api.previewInvite.mockResolvedValue({ club: CLUB, kind: "club", personal: true });
    api.friends.mockResolvedValue(friendsOut({ club_invites: [friendClubInvite({ code: "OTHERCODE1" })] }));
    const { wrapper } = await mountJoin(true);
    expect(wrapper.get(".join-club").text()).toBe("Readers");
    expect(wrapper.get(".lede").text()).toBe(
      "This is a personal invite for someone else. Only the account it was made for can join with it.",
    );
    expect(wrapper.find("button.btn-primary").exists()).toBe(false);
    expect(api.acceptInvite).not.toHaveBeenCalled();
  });

  it("leaves a personal invite to the server when the invitations do not load", async () => {
    api.previewInvite.mockResolvedValue({ club: CLUB, kind: "club", personal: true });
    api.friends.mockRejectedValue(new ApiError("Network error", 0, null));
    const { wrapper } = await mountJoin(true);
    expect(wrapper.find("button.btn-primary").exists()).toBe(true);
  });

  it("explains an invalid invite", async () => {
    api.previewInvite.mockRejectedValue(new ApiError("That invite code is not valid", 404, "That invite code is not valid"));
    const { wrapper } = await mountJoin(true);
    expect(wrapper.get(".error").text()).toBe("That invite code is not valid");
    expect(wrapper.find("button.btn-primary").exists()).toBe(false);
  });
});
