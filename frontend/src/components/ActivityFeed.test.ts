import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { activityItem, bookOut, user } from "../test/fixtures";

const activity = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { activity: (...args: unknown[]) => activity(...args) },
}));

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
  return wrapper.findAll(".activity-row").map((row) => row.text());
}

describe("ActivityFeed times", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    activity.mockReset();
    useSession().user = user({
      timezone: "America/New_York",
      clubs: [{ ...OWLS, role: "member", timezone: "Asia/Tokyo", member_count: 2 }],
    });
  });

  it("shows a club feed in the club's zone, personal events included", async () => {
    const rows = await mountFeed({ club: OWLS.slug }, "Asia/Tokyo");
    expect(rows[0]).toContain("Fri, 2 Oct 2026");
    expect(rows[1]).toContain("Fri, 2 Oct 2026");
  });

  it("shows the home feed's club events in their club's zone and personal ones in the viewer's", async () => {
    const rows = await mountFeed({}, "America/New_York");
    expect(rows[0]).toContain("Fri, 2 Oct 2026");
    expect(rows[1]).toContain("Thu, 1 Oct 2026");
  });
});
