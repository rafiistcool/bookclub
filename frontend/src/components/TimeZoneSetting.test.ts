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

function mountWith(timezone: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ timezone });
  return mount(TimeZoneSetting, { global: { plugins: [pinia, i18n] } });
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
    expect(useToast().items.at(-1)?.message).toContain("America/New_York");
  });

  it("offers this device's zone when the account has another one", async () => {
    // An imported account keeps the zone the importer gave it.
    const wrapper = mountWith("UTC");
    expect(wrapper.get(".device-zone").text()).toContain("This device is set to Europe/Berlin.");
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
});
