import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { useLocale } from "../stores/locale";
import RelativeTime, { TICK_MS } from "./RelativeTime.vue";

const NOW = new Date("2026-11-06T12:00:00Z");

function mountTime(props: { iso: string | null; zone: string; fallback?: string }) {
  return mount(RelativeTime, { props });
}

describe("RelativeTime", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
    vi.setSystemTime(NOW);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows the relative time with the exact one in datetime and title", () => {
    const iso = "2026-11-06T11:55:00Z";
    const wrapper = mountTime({ iso, zone: "Europe/Berlin", fallback: "Fri 6 Nov" });
    const time = wrapper.get("time");
    expect(time.text()).toBe("5 min ago");
    expect(time.attributes("datetime")).toBe(iso);
    expect(time.attributes("title")).toBe("Fri, 6 Nov 2026, 12:55 CET");
    expect(time.attributes("aria-label")).toBeUndefined();
    wrapper.unmount();
  });

  it("speaks German when the locale is German", async () => {
    const wrapper = mountTime({ iso: "2026-11-05T09:00:00Z", zone: "Europe/Berlin" });
    expect(wrapper.text()).toBe("yesterday");
    useLocale().locale = "de";
    await nextTick();
    expect(wrapper.text()).toBe("gestern");
    expect(wrapper.get("time").attributes("title")).toBe("Do., 5. Nov. 2026, 10:00 MEZ");
    wrapper.unmount();
  });

  it("counts the day in the row's zone", () => {
    // 6 Nov 05:00 in Tokyo, still 5 Nov in New York.
    vi.setSystemTime(new Date("2026-11-06T01:00:00Z"));
    const iso = "2026-11-05T20:00:00Z";
    const tokyo = mountTime({ iso, zone: "Asia/Tokyo" });
    const newYork = mountTime({ iso, zone: "America/New_York" });
    expect(tokyo.text()).toBe("5 hr ago");
    expect(newYork.text()).toBe("5 hr ago");
    vi.setSystemTime(new Date("2026-11-06T16:00:00Z"));
    vi.advanceTimersByTime(TICK_MS);
    // 7 Nov 01:00 in Tokyo; 6 Nov 11:00 in New York.
    return nextTick().then(() => {
      expect(tokyo.text()).toBe("yesterday");
      expect(newYork.text()).toBe("yesterday");
      tokyo.unmount();
      newYork.unmount();
    });
  });

  it("falls back to the server's label for a time it cannot read", () => {
    const wrapper = mountTime({ iso: "garbage", zone: "UTC", fallback: "Thu 1 Oct 2026" });
    expect(wrapper.find("time").exists()).toBe(false);
    expect(wrapper.text()).toBe("Thu 1 Oct 2026");
    wrapper.unmount();
  });

  it("moves on with one shared clock that stops when nothing shows a time", async () => {
    const intervals = vi.spyOn(globalThis, "setInterval");
    const a = mountTime({ iso: "2026-11-06T11:59:30Z", zone: "UTC" });
    const b = mountTime({ iso: "2026-11-06T11:50:00Z", zone: "UTC" });
    expect(intervals).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(1);
    expect(a.text()).toBe("just now");
    expect(b.text()).toBe("10 min ago");

    vi.advanceTimersByTime(TICK_MS * 2);
    await nextTick();
    expect(a.text()).toBe("2 min ago");
    expect(b.text()).toBe("12 min ago");

    a.unmount();
    expect(vi.getTimerCount()).toBe(1);
    b.unmount();
    expect(vi.getTimerCount()).toBe(0);
    intervals.mockRestore();
  });

  it("pauses while the page is hidden and catches up when it shows again", async () => {
    const wrapper = mountTime({ iso: "2026-11-06T11:59:30Z", zone: "UTC" });
    const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(vi.getTimerCount()).toBe(0);

    vi.setSystemTime(new Date("2026-11-06T12:30:00Z"));
    hidden.mockReturnValue(false);
    document.dispatchEvent(new Event("visibilitychange"));
    await nextTick();
    expect(vi.getTimerCount()).toBe(1);
    expect(wrapper.text()).toBe("30 min ago");
    wrapper.unmount();
    hidden.mockRestore();
  });

  it("starts a fresh label from the current time after an idle clock", () => {
    const first = mountTime({ iso: "2026-11-06T11:59:30Z", zone: "UTC" });
    first.unmount();
    vi.setSystemTime(new Date("2026-11-06T13:00:00Z"));
    const second = mountTime({ iso: "2026-11-06T11:59:30Z", zone: "UTC" });
    expect(second.text()).toBe("1 hr ago");
    second.unmount();
  });
});
