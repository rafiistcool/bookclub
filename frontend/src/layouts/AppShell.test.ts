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

async function mountShell(path = "/friends") {
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
  await router.push(path);
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

  it("puts Friends and Settings under the profile: the last tab, the account card", async () => {
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

    // The sidebar's rows: the four places; the profile is the card below them.
    const side = shell.findAll(".sidebar-nav a");
    expect(side.map((link) => link.attributes("href"))).toEqual(["/", "/discover", "/shelf", "/club"]);
    expect(side.some((link) => link.attributes("aria-current"))).toBe(false);
    const card = shell.get(".side-account");
    expect(card.classes()).toContain("active");
    const profile = card.get(".side-account-link");
    expect(profile.attributes("href")).toBe("/me");
    expect(profile.attributes("aria-current")).toBe("page");
    expect(profile.get("strong").text()).toBe("ada");
    const count = profile.get(".side-count");
    expect(count.get("[aria-hidden=true]").text()).toBe("2");
    expect(count.get(".visually-hidden").text()).toBe("2 requests or invitations waiting");
    expect(shell.findAll(".side-count")).toHaveLength(1);
    const gear = card.get(".side-settings");
    expect(gear.attributes("href")).toBe("/settings");
    expect(gear.attributes("aria-label")).toBe("Settings");
    expect(gear.attributes("aria-current")).toBeUndefined();

    // The phone: five tabs, the last one the reader's picture with a dot while something waits.
    const tabs = shell.findAll(".bottom-nav a");
    expect(tabs.map((link) => link.attributes("href"))).toEqual(["/", "/discover", "/shelf", "/club", "/me"]);
    const tab = tabs[4]!;
    expect(tab.attributes("aria-current")).toBe("page");
    expect(tab.find("svg").exists()).toBe(false);
    expect(tab.get(".tab-avatar").classes()).toContain("waiting");
    expect(tab.get(".tab-avatar .avatar").text()).toBe("A");
    expect(tab.text()).toContain("Profile");
    expect(tab.get(".visually-hidden").text()).toBe("2 requests or invitations waiting");

    // The Profil tab is the phone's way to the profile; the top bar carries only the wordmark.
    expect(shell.find(".topbar .avatar-link").exists()).toBe(false);
  });

  it("lights the profile on its own pages only", async () => {
    api.friends!.mockResolvedValue(friendsOut());
    const shell = await mountShell("/shelf");
    expect(shell.get(".side-account").classes()).not.toContain("active");
    expect(shell.get(".bottom-nav a[aria-current=page]").attributes("href")).toBe("/shelf");
    expect(shell.get(".tab-avatar").classes()).not.toContain("waiting");
    expect(shell.get(".bottom-nav a:last-of-type").find(".visually-hidden").exists()).toBe(false);
  });

  it("on Settings, marks the gear as the current page, not the profile", async () => {
    api.friends!.mockResolvedValue(friendsOut());
    const shell = await mountShell("/settings");
    const card = shell.get(".side-account");
    expect(card.classes()).toContain("active");
    expect(card.get(".side-account-link").attributes("aria-current")).toBeUndefined();
    const gear = card.get(".side-settings");
    expect(gear.attributes("aria-current")).toBe("page");
    expect(gear.classes()).toContain("on");
    // The phone has no gear: its profile tab stays the way to Settings.
    expect(shell.get(".bottom-nav a[aria-current=page]").attributes("href")).toBe("/me");
  });

  it("shows no count when nothing waits or the lists cannot be loaded", async () => {
    api.friends!.mockResolvedValue(friendsOut({ friends: [friendOut("grace")], outgoing: [friendRequest("olga")] }));
    let shell = await mountShell();
    expect(shell.find(".side-count").exists()).toBe(false);
    expect(shell.get(".tab-avatar").classes()).not.toContain("waiting");
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

  describe("the phone's top bar", () => {
    function scrollTo(y: number) {
      Object.defineProperty(window, "scrollY", { configurable: true, value: y });
      window.dispatchEvent(new Event("scroll"));
    }

    beforeEach(() => {
      api.friends!.mockResolvedValue(friendsOut());
      vi.stubGlobal("requestAnimationFrame", (run: FrameRequestCallback) => { queueMicrotask(() => run(0)); return 1; });
      vi.stubGlobal("cancelAnimationFrame", () => {});
    });

    afterEach(() => {
      Reflect.deleteProperty(window, "scrollY");
    });

    it("slides away on the way down, comes back on the way up and on a new page", async () => {
      vi.stubGlobal("matchMedia", (query: string) => ({
        matches: query === "(max-width: 720px)", addEventListener() {}, removeEventListener() {},
      }));
      const shell = await mountShell("/shelf");
      const bar = () => shell.get(".topbar");
      scrollTo(30);
      await flushPromises();
      expect(bar().classes()).not.toContain("away");
      scrollTo(400);
      await flushPromises();
      expect(bar().classes()).toContain("away");
      expect(shell.get(".shell").classes()).toContain("bar-away");
      // A finger resting does nothing; a real move up brings it back.
      scrollTo(396);
      await flushPromises();
      expect(bar().classes()).toContain("away");
      scrollTo(300);
      await flushPromises();
      expect(bar().classes()).not.toContain("away");
      scrollTo(800);
      await flushPromises();
      expect(bar().classes()).toContain("away");
      // Tabbing onto the wordmark shows the bar.
      await bar().trigger("focusin");
      expect(bar().classes()).not.toContain("away");
      scrollTo(1200);
      await flushPromises();
      expect(bar().classes()).toContain("away");
      await shell.vm.$router.push("/discover");
      await flushPromises();
      expect(bar().classes()).not.toContain("away");
    });

    it("stays on a page the router puts back deep down, and hides on the reader's next move down", async () => {
      vi.stubGlobal("matchMedia", (query: string) => ({
        matches: query === "(max-width: 720px)", addEventListener() {}, removeEventListener() {},
      }));
      const shell = await mountShell("/book/1");
      const bar = () => shell.get(".topbar");
      scrollTo(0);
      await flushPromises();
      // Back to the shelf: the path changes while the book page is still at the top,
      // then the router restores the shelf's 1500 px.
      await shell.vm.$router.push("/shelf");
      await flushPromises();
      scrollTo(1500);
      await flushPromises();
      expect(bar().classes()).not.toContain("away");
      // That position is where the reader's own scrolling counts from.
      scrollTo(1504);
      await flushPromises();
      expect(bar().classes()).not.toContain("away");
      scrollTo(1600);
      await flushPromises();
      expect(bar().classes()).toContain("away");
    });

    it("stays where it is on a wide screen", async () => {
      const shell = await mountShell("/shelf");
      scrollTo(400);
      await flushPromises();
      scrollTo(900);
      await flushPromises();
      expect(shell.get(".topbar").classes()).not.toContain("away");
    });
  });
});

