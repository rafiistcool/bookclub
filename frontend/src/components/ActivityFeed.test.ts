import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { activityItem, bookOut, user } from "../test/fixtures";

const activity = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { activity: (...args: unknown[]) => activity(...args) },
}));

import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import ActivityFeed from "./ActivityFeed.vue";

const OWLS = { slug: "night-owls-cd34", name: "Night owls" };
// 22:30 UTC: still the 1st in New York and London, the 2nd in Tokyo.
const AT = "2026-10-01T22:30:00Z";
const ITEMS = [
  activityItem({ id: 2, kind: "nominated", actor: "grace", club: OWLS, book: bookOut({ title: "Circe" }), created_at: AT }),
  activityItem({ id: 1, kind: "shelf_finished", actor: "ada", club: null, book: bookOut({ title: "Circe" }), created_at: AT }),
];

async function mountFeed(props: { club?: string }, timezone: string) {
  activity.mockResolvedValue({ items: ITEMS, has_more: false, timezone });
  const wrapper = mount(ActivityFeed, { props, global: { stubs: { BookCover: true, Avatar: true } } });
  await flushPromises();
  return wrapper;
}

const times = (wrapper: Awaited<ReturnType<typeof mountFeed>>) =>
  wrapper.findAll(".activity-row time").map((time) => time.text());

describe("ActivityFeed times", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    activity.mockReset();
    vi.useFakeTimers({ toFake: ["Date"] });
    // Monday 5 Oct, noon UTC: Friday 2 Oct is 3 days back in Tokyo, Thursday 1 Oct 4 in New York.
    vi.setSystemTime(new Date("2026-10-05T12:00:00Z"));
    useSession().user = user({
      timezone: "America/New_York",
      clubs: [{ ...OWLS, role: "member", timezone: "Asia/Tokyo", member_count: 2 }],
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("counts days in the viewer's zone and keeps the club's time in the title", async () => {
    // The viewer is in New York: 22:30 UTC is their Thursday, though it is Friday in Tokyo.
    const wrapper = await mountFeed({ club: OWLS.slug }, "Asia/Tokyo");
    expect(times(wrapper)).toEqual(["Thursday", "Thursday"]);
    const time = wrapper.get(".activity-row time");
    expect(time.attributes("datetime")).toBe(AT);
    expect(time.attributes("title")).toBe("Fri, 2 Oct 2026, 07:30 GMT+9");
  });

  it("shows the home feed's club events with their club's time in the title", async () => {
    const wrapper = await mountFeed({}, "America/New_York");
    expect(times(wrapper)).toEqual(["Thursday", "Thursday"]);
    expect(wrapper.findAll(".activity-row time")[0]!.attributes("title")).toBe("Fri, 2 Oct 2026, 07:30 GMT+9");
    expect(wrapper.findAll(".activity-row time")[1]!.attributes("title")).toBe("Thu, 1 Oct 2026, 18:30 GMT-4");
  });

  it("counts yesterday in the row's zone, in German too", async () => {
    // 3 Oct 06:00 in Tokyo, 2 Oct 17:00 in New York.
    vi.setSystemTime(new Date("2026-10-02T21:00:00Z"));
    useLocale().locale = "de";
    const wrapper = await mountFeed({}, "America/New_York");
    expect(times(wrapper)).toEqual(["gestern", "gestern"]);
    vi.setSystemTime(new Date("2026-10-01T23:00:00Z"));
    const later = await mountFeed({}, "America/New_York");
    expect(times(later)).toEqual(["vor 30 Min.", "vor 30 Min."]);
  });

  it("shows a day and month for older lines, the year only before this one", async () => {
    vi.setSystemTime(new Date("2026-11-20T12:00:00Z"));
    const wrapper = await mountFeed({}, "America/New_York");
    expect(times(wrapper)).toEqual(["1 Oct", "1 Oct"]);
    vi.setSystemTime(new Date("2027-01-20T12:00:00Z"));
    const nextYear = await mountFeed({}, "America/New_York");
    expect(times(nextYear)).toEqual(["1 Oct 2026", "1 Oct 2026"]);
  });
});

describe("ActivityFeed preview (Home)", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    activity.mockReset();
    useSession().user = user();
  });

  it("asks only the preview's rows and links to the whole feed instead of paging", async () => {
    activity.mockResolvedValue({ items: ITEMS, has_more: true, timezone: "UTC" });
    const wrapper = mount(ActivityFeed, {
      props: { limit: 5, moreTo: "/c/night-owls-cd34/events" },
      global: { stubs: { BookCover: true, Avatar: true } },
    });
    await flushPromises();
    expect(activity).toHaveBeenCalledWith({ before: undefined, limit: 5, club: undefined });
    expect(wrapper.findAll(".activity-row")).toHaveLength(2);
    expect(wrapper.get(".activity-all").attributes("href")).toBe("/c/night-owls-cd34/events");
    expect(wrapper.get(".activity-all").text()).toBe("See all");
    expect(wrapper.find(".activity-more").exists()).toBe(false);
  });

  it("keeps paging where no page holds the whole feed", async () => {
    activity.mockResolvedValue({ items: ITEMS, has_more: true, timezone: "UTC" });
    const wrapper = mount(ActivityFeed, { props: { limit: 5 }, global: { stubs: { BookCover: true, Avatar: true } } });
    await flushPromises();
    expect(wrapper.find(".activity-all").exists()).toBe(false);
    await wrapper.get(".activity-more").trigger("click");
    await flushPromises();
    expect(activity).toHaveBeenLastCalledWith({ before: 1, limit: 5, club: undefined });
  });
});

