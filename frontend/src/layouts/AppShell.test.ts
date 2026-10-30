import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { user } from "../test/fixtures";
import { friendClubInvite, friendOut, friendRequest, friendsOut } from "../test/friends";
import { memoryStorage } from "../test/storage";

const api: Record<string, ReturnType<typeof vi.fn>> = { friends: vi.fn() };

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => api[key] }),
}));

import { i18n } from "../i18n";
import { RELOAD_AFTER_MS } from "../stores/friends";
import { useSession } from "../stores/session";
import AppShell from "./AppShell.vue";

let wrapper: VueWrapper | null = null;

async function mountShell() {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ id: 1, username: "ada" });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: "/",
        component: AppShell,
        children: [{ path: ":rest(.*)*", component: { template: '<p class="page">page</p>' } }],
      },
    ],
  });
  await router.push("/friends");
  wrapper = mount({ template: "<RouterView />" }, { global: { plugins: [pinia, router, i18n] } });
  await flushPromises();
  return wrapper;
}

function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("AppShell", () => {
  beforeEach(() => {
    api.friends!.mockReset();
    i18n.global.locale.value = "en";
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
    vi.useRealTimers();
    vi.unstubAllGlobals();
    Reflect.deleteProperty(document, "visibilityState");
  });

  it("lists Friends in the sidebar after Club, with what waits, and keeps it out of the tab bar", async () => {
    api.friends!.mockResolvedValue(
      friendsOut({
        friends: [friendOut("grace")],
        incoming: [friendRequest("mia")],
        outgoing: [friendRequest("olga")],
        club_invites: [friendClubInvite()],
      }),
    );
    const shell = await mountShell();
    expect(api.friends).toHaveBeenCalledTimes(1);

    const side = shell.findAll(".sidebar-nav a");
    expect(side.map((link) => link.attributes("href"))).toEqual(["/", "/discover", "/shelf", "/club", "/friends", "/settings"]);
    const friends = side[4]!;
    expect(friends.attributes("aria-current")).toBe("page");
    const count = friends.get(".side-count");
    expect(count.get("[aria-hidden=true]").text()).toBe("2");
    expect(count.get(".visually-hidden").text()).toBe("2 requests or invitations waiting");
    expect(side.filter((link) => link.find(".side-count").exists())).toHaveLength(1);

    expect(shell.findAll(".bottom-nav a").map((link) => link.attributes("href"))).toEqual([
      "/",
      "/discover",
      "/shelf",
      "/club",
      "/settings",
    ]);
  });

  it("shows no count when nothing waits or the lists cannot be loaded", async () => {
    api.friends!.mockResolvedValue(friendsOut({ friends: [friendOut("grace")], outgoing: [friendRequest("olga")] }));
    let shell = await mountShell();
    expect(shell.find(".side-count").exists()).toBe(false);
    shell.unmount();

    api.friends!.mockRejectedValue(new Error("offline"));
    shell = await mountShell();
    expect(shell.find(".side-count").exists()).toBe(false);
    expect(shell.find(".page").exists()).toBe(true);
  });

  it("asks again when the tab comes back after a minute, and when the reader's clubs change", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-10-29T12:00:00Z"));
    api.friends!.mockResolvedValue(friendsOut());
    const shell = await mountShell();
    expect(api.friends).toHaveBeenCalledTimes(1);

    setVisibility("visible");
    await flushPromises();
    expect(api.friends).toHaveBeenCalledTimes(1);

    vi.setSystemTime(new Date(Date.now() + RELOAD_AFTER_MS));
    api.friends!.mockResolvedValue(friendsOut({ incoming: [friendRequest("mia")] }));
    setVisibility("visible");
    await flushPromises();
    expect(api.friends).toHaveBeenCalledTimes(2);
    expect(shell.get(".side-count [aria-hidden=true]").text()).toBe("1");

    // Joining a club (from an invitation) can end an invitation that waited.
    useSession().user!.clubs = [{ slug: "readers-ab12", name: "Readers", role: "member", timezone: "UTC", member_count: 3 }];
    await flushPromises();
    expect(api.friends).toHaveBeenCalledTimes(3);

    // Signing out asks nothing, and an unmounted shell no longer listens.
    useSession().user = null;
    await flushPromises();
    expect(api.friends).toHaveBeenCalledTimes(3);
    shell.unmount();
    wrapper = null;
    vi.setSystemTime(new Date(Date.now() + 5 * RELOAD_AFTER_MS));
    setVisibility("visible");
    await flushPromises();
    expect(api.friends).toHaveBeenCalledTimes(3);
  });
});
