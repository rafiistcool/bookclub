import { mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import BackLink from "./BackLink.vue";

const stub = { template: "<p />" };

async function mountLink(props: { label: string; fallback: string; exact?: boolean }, back: string | null) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/:any(.*)*", component: stub }],
  });
  await router.push("/settings/account");
  const goBack = vi.spyOn(router, "back").mockImplementation(() => {});
  const push = vi.spyOn(router, "push");
  // vue-router's web history keeps the previous path in `history.state.back`.
  window.history.replaceState(back === null ? null : { back }, "");
  const wrapper = mount(BackLink, { props, global: { plugins: [router] } });
  await wrapper.get("a").trigger("click");
  return { goBack, push };
}

describe("BackLink", () => {
  afterEach(() => window.history.replaceState(null, ""));

  it("goes back to whatever page came before", async () => {
    const { goBack, push } = await mountLink({ label: "← Back", fallback: "/settings" }, "/shelf");
    expect(goBack).toHaveBeenCalled();
    expect(push).not.toHaveBeenCalled();
  });

  it("opens the fallback when the page was opened directly", async () => {
    const { goBack, push } = await mountLink({ label: "← Back", fallback: "/settings" }, null);
    expect(goBack).not.toHaveBeenCalled();
    expect(push).toHaveBeenCalledWith("/settings");
  });

  it("with exact, goes back only to the page its label names", async () => {
    const { goBack, push } = await mountLink({ label: "← Settings", fallback: "/settings", exact: true }, "/shelf");
    expect(goBack).not.toHaveBeenCalled();
    expect(push).toHaveBeenCalledWith("/settings");
  });

  it("with exact, goes back when the previous page is the named one, whatever its query", async () => {
    const { goBack, push } = await mountLink({ label: "← Settings", fallback: "/settings", exact: true }, "/settings?sheet=language");
    expect(goBack).toHaveBeenCalled();
    expect(push).not.toHaveBeenCalled();
  });
});
