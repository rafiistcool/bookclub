import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { bookOut, clubOut, user } from "../../test/fixtures";
import type { ClubSummary } from "../../types";
import { memoryStorage } from "../../test/storage";

const api = { club: vi.fn(), pickThread: vi.fn() };

vi.mock("../../api/client", () => {
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

import { i18n } from "../../i18n";
import ClubLayout from "../../layouts/ClubLayout.vue";
import { useSession } from "../../stores/session";
import ClubDiscussionsPage from "./ClubDiscussionsPage.vue";

const READERS: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "Europe/Berlin", member_count: 3 };

async function mountAt(path: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs: [READERS] });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/c/:slug", component: ClubLayout, children: [{ path: "discussions", component: ClubDiscussionsPage }] }],
  });
  await router.push(path);
  const wrapper = mount(
    { template: "<RouterView />" },
    { global: { plugins: [pinia, router, i18n], stubs: { DiaryFeed: true } } },
  );
  await flushPromises();
  return wrapper;
}

describe("ClubDiscussionsPage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    vi.stubGlobal("localStorage", memoryStorage());
  });

  afterEach(() => vi.unstubAllGlobals());

  it("asks for no thread in a club without a current pick", async () => {
    api.club.mockResolvedValue(clubOut({ ...READERS, current_pick: null }));
    const wrapper = await mountAt("/c/readers-ab12/discussions");
    // The club answer already says so; the thread would only answer 404 `No club pick yet`.
    expect(api.pickThread).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain("No book is picked right now");
  });

  it("loads the current pick's thread when the club has one", async () => {
    api.club.mockResolvedValue(
      clubOut({ ...READERS, current_pick: { id: 5, book: bookOut(), meeting_at: null } }),
    );
    api.pickThread.mockResolvedValue({
      pick_id: 5,
      can_post: true,
      my_progress: null,
      my_status: null,
      items: [],
      timezone: "Europe/Berlin",
    });
    const wrapper = await mountAt("/c/readers-ab12/discussions");
    expect(api.pickThread).toHaveBeenCalledWith("readers-ab12", null);
    expect(wrapper.find(".compose").exists()).toBe(true);
  });
});
