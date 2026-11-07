import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { user } from "../test/fixtures";

const savePreferences = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { savePreferences: (...args: unknown[]) => savePreferences(...args) },
}));

vi.mock("../timezone", async (original) => ({
  ...(await original<typeof import("../timezone")>()),
  browserTimeZone: () => "Europe/Berlin",
}));

import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import TimeZoneSetting from "./TimeZoneSetting.vue";

function mountWith(timezone: string, props: { hideLabel?: boolean } = {}) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ timezone });
  return mount(TimeZoneSetting, { props, global: { plugins: [pinia, i18n] } });
}

describe("the personal time zone", () => {
  // A function returned from beforeEach would run as a teardown: keep the braces.
  beforeEach(() => {
    savePreferences.mockReset();
  });

  it("shows the saved zone and saves another one", async () => {
    const wrapper = mountWith("Europe/Berlin");
    const select = wrapper.get<HTMLSelectElement>("select");
    expect(select.element.value).toBe("Europe/Berlin");
    // The device uses the saved zone: nothing to offer.
    expect(wrapper.find(".device-zone").exists()).toBe(false);

    savePreferences.mockResolvedValue(user({ timezone: "America/New_York" }));
    await select.setValue("America/New_York");
    await flushPromises();
    expect(savePreferences).toHaveBeenCalledWith({ timezone: "America/New_York" });
    expect(useSession().user?.timezone).toBe("America/New_York");
    // Settings' row shows the saved zone: no toast says it again.
    expect(useToast().items).toHaveLength(0);
  });

  it("offers this device's zone when the account has another one", async () => {
    // An imported account keeps the zone the importer gave it.
    const wrapper = mountWith("UTC");
    // The zone is named once, in the sentence; the button only says what it does.
    // It comes first, as the one-tap way, above the full list.
    expect(wrapper.get(".device-zone .device-name").text()).toBe("This device: Europe/Berlin.");
    expect(wrapper.element.firstElementChild?.classList.contains("device-zone")).toBe(true);
    expect(wrapper.get(".device-zone button").text()).toBe("Use it");
    savePreferences.mockResolvedValue(user({ timezone: "Europe/Berlin" }));
    await wrapper.get(".device-zone button").trigger("click");
    await flushPromises();
    expect(savePreferences).toHaveBeenCalledWith({ timezone: "Europe/Berlin" });
    expect(wrapper.find(".device-zone").exists()).toBe(false);
    expect(wrapper.get<HTMLSelectElement>("select").element.value).toBe("Europe/Berlin");
  });

  it("goes back to the saved zone when saving fails", async () => {
    const wrapper = mountWith("UTC");
    savePreferences.mockRejectedValue(new Error("offline"));
    await wrapper.get("select").setValue("Asia/Tokyo");
    await flushPromises();
    expect(wrapper.get<HTMLSelectElement>("select").element.value).toBe("UTC");
    expect(useSession().user?.timezone).toBe("UTC");
    expect(useToast().items.at(-1)?.message).toBe("Could not save the time zone");
  });

  it("keeps the label for screen readers only under a sheet that is titled the same", () => {
    const shown = mountWith("Europe/Berlin");
    expect(shown.get(".timezone-field > span").classes()).not.toContain("visually-hidden");
    const hidden = mountWith("Europe/Berlin", { hideLabel: true });
    const label = hidden.get(".timezone-field > span");
    expect(label.classes()).toContain("visually-hidden");
    // Still the select's label: it wraps it.
    expect(hidden.get(".timezone-field").element.tagName).toBe("LABEL");
    expect(label.text()).toBe("Time zone");
  });
});
