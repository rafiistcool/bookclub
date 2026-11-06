import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";
import { post, user } from "../test/fixtures";
import type { DiaryEntry as Entry } from "../types";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import DiaryEntry from "./DiaryEntry.vue";

const CLUB = { slug: "readers-ab12", name: "Readers" };

function mountEntry(entry: Entry) {
  return mount(DiaryEntry, {
    props: { entry, myProgress: null, myStatus: null },
    global: { stubs: { Avatar: true, SpoilerControl: true } },
  });
}

describe("DiaryEntry time", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
    useSession().user = user({
      timezone: "Europe/Berlin",
      clubs: [{ ...CLUB, role: "member", timezone: "America/New_York", member_count: 2 }],
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows a diary entry relative to now in the reader's zone, the exact time on hover", () => {
    // 2 Sep 00:30 in Berlin, 1 Sep 18:30 in New York.
    vi.setSystemTime(new Date("2026-09-01T22:30:00Z"));
    const wrapper = mountEntry(post({ created_at: "2026-09-01T20:00:00Z", created_label: "1 Sep" }));
    const time = wrapper.get(".entry-time");
    expect(time.element.tagName).toBe("TIME");
    expect(time.classes()).toEqual(expect.arrayContaining(["finer", "subtle"]));
    expect(time.text()).toBe("yesterday");
    expect(time.attributes("datetime")).toBe("2026-09-01T20:00:00Z");
    expect(time.attributes("title")).toBe("Tue, 1 Sept 2026, 22:00 CEST");
    wrapper.unmount();
  });

  it("counts a club post's days in the club's zone, in German too", async () => {
    vi.setSystemTime(new Date("2026-09-01T22:30:00Z"));
    useLocale().locale = "de";
    const wrapper = mountEntry(post({ club: CLUB, created_at: "2026-09-01T20:00:00Z" }));
    expect(wrapper.get(".entry-time").text()).toBe("vor 2 Std.");
    expect(wrapper.get(".entry-time").attributes("title")).toBe("Di., 1. Sept. 2026, 16:00 GMT-4");

    // The shared clock moves the label along without a new render from outside.
    vi.setSystemTime(new Date("2026-09-02T05:00:00Z"));
    vi.advanceTimersByTime(60_000);
    await nextTick();
    expect(wrapper.get(".entry-time").text()).toBe("gestern");
    wrapper.unmount();
  });

  it("falls back to the server's label without a readable time", () => {
    const wrapper = mountEntry(post({ created_at: "", created_label: "1 Sep" }));
    expect(wrapper.get(".entry-time").element.tagName).toBe("SPAN");
    expect(wrapper.get(".entry-time").text()).toBe("1 Sep");
    wrapper.unmount();
  });
});
