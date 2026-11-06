import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The scanner's photo never finishes loading here, so it stays open to look at.
vi.mock("./coverScan", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./coverScan")>()),
  loadForScan: () => new Promise(() => {}),
}));

import App from "./App.vue";
import { i18n } from "./i18n";
import { useCoverScan } from "./stores/coverScan";
import { useFinish } from "./stores/finish";
import { shelfItem } from "./test/fixtures";

const photo = new File(["photo"], "photo.jpg", { type: "image/jpeg" });

beforeEach(() => {
  setActivePinia(createPinia());
  i18n.global.locale.value = "en";
});

function mountApp() {
  return mount(App, { attachTo: document.body, global: { plugins: [i18n], stubs: { RouterView: true, ToastHost: true } } });
}

describe("App", () => {
  it("loads and mounts the cover scanner only while a photo is open", async () => {
    const wrapper = mountApp();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    const answer = useCoverScan().request(photo);
    await vi.waitFor(() => expect(wrapper.find('[role="dialog"]').exists()).toBe(true));
    // Mounted with the photo already waiting, it still opens it.
    expect(wrapper.get(".scan-status").text()).toBe("Loading the photo…");
    useCoverScan().finish(null);
    await expect(answer).resolves.toBeNull();
    await flushPromises();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(document.body.style.overflow).toBe("");
    wrapper.unmount();
  });

  it("imports the scanner lazily, not with the app", async () => {
    const source = await import("./App.vue?raw");
    expect(source.default).not.toMatch(/^import\s+CoverScanner\b/m);
    expect(source.default).toMatch(/defineAsyncComponent\(\(\) =>\s*import\("\.\/components\/CoverScanner\.vue"\)/);
  });

  it("loads and mounts the finished moment only while one is open", async () => {
    const wrapper = mountApp();
    const answer = useFinish().celebrate({ item: shelfItem({ status: "finished", book: { title: "Circe" } }) });
    await vi.waitFor(() => expect(wrapper.find('[role="dialog"]').exists()).toBe(true));
    expect(wrapper.get('[role="dialog"]').attributes("aria-label")).toBe("Finished!");
    expect(wrapper.get(".moment-title").text()).toBe("Circe");
    useFinish().close(null);
    await expect(answer).resolves.toBeNull();
    await flushPromises();
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
    expect(document.body.style.overflow).toBe("");
    wrapper.unmount();
  });

  it("imports the finished moment lazily, not with the app", async () => {
    const source = await import("./App.vue?raw");
    expect(source.default).not.toMatch(/^import\s+FinishedMoment\b/m);
    expect(source.default).toMatch(/defineAsyncComponent\(\(\) =>\s*import\("\.\/components\/FinishedMoment\.vue"\)/);
  });
});
