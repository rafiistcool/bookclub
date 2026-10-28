import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { clubOut, user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";

const createClub = vi.fn();

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
  return { ApiError, api: { createClub: (...args: unknown[]) => createClub(...args) } };
});

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { translateApiError } from "../i18n/apiErrors";
import { useSession } from "../stores/session";
import NewClubPage from "./NewClubPage.vue";

async function mountPage(verified = true) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ timezone: "Europe/Berlin", email_verified: verified });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/clubs/new", component: NewClubPage },
      { path: "/:rest(.*)*", component: { template: "<p>elsewhere</p>" } },
    ],
  });
  await router.push("/clubs/new");
  const wrapper = mount(NewClubPage, { global: { plugins: [pinia, router, i18n] } });
  await flushPromises();
  return { wrapper, router };
}

describe("NewClubPage", () => {
  beforeEach(() => {
    createClub.mockReset();
    vi.stubGlobal("localStorage", memoryStorage());
  });

  afterEach(() => vi.unstubAllGlobals());

  it("creates the club in the creator's zone and opens its members page", async () => {
    createClub.mockResolvedValue(clubOut({ slug: "thursday-readers-x1y2", name: "Thursday readers", role: "owner", member_count: 1 }));
    const { wrapper, router } = await mountPage();
    expect((wrapper.get("select").element as HTMLSelectElement).value).toBe("Europe/Berlin");
    await wrapper.get("input[name=club-name]").setValue("  Thursday readers ");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(createClub).toHaveBeenCalledWith({ name: "Thursday readers", timezone: "Europe/Berlin" });
    expect(router.currentRoute.value.path).toBe("/c/thursday-readers-x1y2/members");
    expect(useSession().user?.clubs[0]).toMatchObject({ slug: "thursday-readers-x1y2", role: "owner" });
  });

  it("shows the club limit as the mapped message", async () => {
    const detail = "You can create at most 10 clubs";
    i18n.global.locale.value = "de";
    try {
      // The client translates a detail when the answer arrives.
      createClub.mockImplementation(() => Promise.reject(new ApiError(translateApiError(detail), 409, detail)));
      const { wrapper, router } = await mountPage();
      await wrapper.get("input[name=club-name]").setValue("Elfter Club");
      await wrapper.get("form").trigger("submit");
      await flushPromises();
      expect(wrapper.get(".error").text()).toBe(i18n.global.t("errors.clubLimit"));
      expect(wrapper.get(".error").text()).not.toBe(detail);
      expect(router.currentRoute.value.path).toBe("/clubs/new");
    } finally {
      i18n.global.locale.value = "en";
    }
  });

  it("asks an unverified reader to confirm their address first", async () => {
    const { wrapper } = await mountPage(false);
    expect(wrapper.get(".verify-note").text()).toContain("ada@example.com");
  });
});
