import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";

const api: Record<string, ReturnType<typeof vi.fn>> = { adminSummary: vi.fn() };

vi.mock("../../api/client", () => ({
  api: new Proxy({}, { get: (_target, key: string) => api[key] }),
}));

import { i18n } from "../../i18n";
import AdminLink from "./AdminLink.vue";

function queue(open: number, overdue = 0) {
  return { open, overdue, oldest: null };
}

async function mountLink() {
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: "/:rest(.*)*", component: { template: "<p />" } }] });
  const wrapper = mount(AdminLink, { global: { plugins: [router, i18n] } });
  await flushPromises();
  return wrapper;
}

describe("AdminLink", () => {
  beforeEach(() => {
    api.adminSummary.mockReset();
    i18n.global.locale.value = "en";
  });

  it("shows how many reports and requests wait, marked when one is overdue", async () => {
    api.adminSummary.mockResolvedValue({ target_hours: 72, reports: queue(2, 1), edit_requests: queue(1) });
    const wrapper = await mountLink();
    expect(wrapper.get("a").attributes("href")).toBe("/admin");
    const count = wrapper.get(".admin-count");
    expect(count.get("[aria-hidden=true]").text()).toBe("3");
    expect(count.get(".visually-hidden").text()).toBe("3 items waiting");
    expect(count.classes()).toContain("overdue");
  });

  it("is the plain link when nothing waits or the summary fails", async () => {
    api.adminSummary.mockResolvedValue({ target_hours: 72, reports: queue(0), edit_requests: queue(0) });
    expect((await mountLink()).find(".admin-count").exists()).toBe(false);
    api.adminSummary.mockRejectedValue(new Error("offline"));
    const wrapper = await mountLink();
    expect(wrapper.find(".admin-count").exists()).toBe(false);
    expect(wrapper.get("a").text()).toBe("Open the admin page");
  });
});
