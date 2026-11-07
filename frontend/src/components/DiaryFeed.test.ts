import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { bookOut, post, user } from "../test/fixtures";
import type { DiaryFeedItem } from "../types";

const diaryFeed = vi.fn();
const editDiaryEntry = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    diaryFeed: (...args: unknown[]) => diaryFeed(...args),
    editDiaryEntry: (...args: unknown[]) => editDiaryEntry(...args),
  },
}));

import { i18n } from "../i18n";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import DiaryFeed from "./DiaryFeed.vue";

const SECRET = "The coconut pillow on p. 237 is a tell.";

function feedItem(overrides: Partial<DiaryFeedItem> = {}): DiaryFeedItem {
  return {
    parent_author: null,
    my_progress: 20,
    my_status: "currently_reading",
    club: null,
    book: bookOut({ id: 1, title: "The Wedding People", authors: "Alison Espach", year: 2024 }),
    entry: post({
      id: 11,
      author: "anni",
      body: SECRET,
      spoiler_upto: 80,
      progress_at: 80,
      status_at: "currently_reading",
      created_at: "2026-09-01T12:00:00Z",
      created_label: "Sep 1",
    }),
    ...overrides,
  };
}

async function mountClub(items: DiaryFeedItem[], props: { club?: string; mine?: boolean } = {}) {
  diaryFeed.mockResolvedValue({ items, has_more: false, timezone: "UTC" });
  return mountFeed(props);
}

async function mountFeed(
  props: {
    club?: string;
    mine?: boolean;
    pageSize?: number;
    limit?: number;
    moreTo?: string;
    heading?: string;
    headingHidden?: boolean;
    threadWork?: number | null;
  } = {},
) {
  const wrapper = mount(DiaryFeed, {
    props,
    global: {
      plugins: [createPinia(), i18n],
      stubs: { BookCover: true },
    },
  });
  await flushPromises();
  return wrapper;
}

describe("DiaryFeed (recently written)", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    diaryFeed.mockReset();
    editDiaryEntry.mockReset();
  });

  it("does not leak a spoiler-flagged excerpt when the viewer is behind", async () => {
    const wrapper = await mountClub([feedItem()]);
    expect(wrapper.find(".feed-body").exists()).toBe(false);
    const guard = wrapper.get(".feed-guard");
    expect(guard.get(".feed-guard-line").text()).toBe("Spoiler up to 80% · you're at 20%");
    expect(guard.get(".feed-guard-hint").text()).toBe("Tap to read");
    expect(wrapper.html()).not.toContain(SECRET);
    // A flagged post's line names the flag, so the foot keeps where its writer was.
    expect(wrapper.get(".feed-foot").text()).toContain("at 80%");
    // A tap shows the excerpt here, for this view.
    await guard.trigger("click");
    expect(wrapper.find(".feed-guard").exists()).toBe(false);
    expect(wrapper.get(".feed-body").text()).toContain(SECRET);
  });

  it("folds an unflagged note written further on than the viewer is", async () => {
    const wrapper = await mountClub([feedItem({ entry: { ...feedItem().entry, spoiler_upto: null } })]);
    expect(wrapper.get(".feed-guard-line").text()).toBe("at 80% · you're at 20%");
    expect(wrapper.html()).not.toContain(SECRET);
    // The folded line says where the writer was; the foot does not say it again.
    expect(wrapper.get(".feed-foot").text()).not.toContain("80%");
    // Once read, the foot carries the marker again.
    await wrapper.get(".feed-guard").trigger("click");
    expect(wrapper.get(".feed-foot").text()).toContain("at 80%");
    const caughtUp = await mountClub([
      feedItem({ my_progress: 80, entry: { ...feedItem().entry, spoiler_upto: null } }),
    ]);
    expect(caughtUp.find(".feed-guard").exists()).toBe(false);
    expect(caughtUp.get(".feed-body").text()).toContain(SECRET);
  });

  it("reads again when a reload comes while a page is loading", async () => {
    let answer: (value: unknown) => void = () => {};
    diaryFeed.mockReturnValueOnce(new Promise((resolve) => (answer = resolve)));
    const wrapper = mount(DiaryFeed, { global: { plugins: [createPinia(), i18n], stubs: { BookCover: true } } });
    // The viewer moved on while the first page was on its way: that page predates it.
    diaryFeed.mockResolvedValue({
      items: [feedItem({ my_progress: 80, entry: { ...feedItem().entry, spoiler_upto: null } })],
      has_more: false,
      timezone: "UTC",
    });
    await (wrapper.vm as unknown as { reload: () => Promise<void> }).reload();
    answer({ items: [feedItem({ entry: { ...feedItem().entry, spoiler_upto: null } })], has_more: false, timezone: "UTC" });
    await flushPromises();
    expect(diaryFeed).toHaveBeenCalledTimes(2);
    expect(wrapper.find(".feed-guard").exists()).toBe(false);
    expect(wrapper.get(".feed-body").text()).toContain(SECRET);
  });

  it("still shows the excerpt to the writer", async () => {
    const wrapper = await mountClub([
      feedItem({
        my_progress: 10,
        my_status: "currently_reading",
        entry: { ...feedItem().entry, mine: true, author: "selina" },
      }),
    ]);
    const row = wrapper.get(".feed-body");
    expect(wrapper.find(".feed-guard").exists()).toBe(false);
    expect(row.text()).toContain(SECRET);
  });

  it("unfolds a long note in place instead of only linking away", async () => {
    const long = `${"A thought that keeps going. ".repeat(8)}\nAnd a second paragraph.`;
    const wrapper = await mountClub([
      feedItem({ entry: { ...feedItem().entry, body: long, spoiler_upto: null, progress_at: 10 } }),
    ]);
    const body = wrapper.get(".feed-body");
    expect(body.text()).toContain("…");
    expect(body.text()).not.toContain("second paragraph");
    const more = wrapper.get(".feed-more");
    expect(more.attributes("aria-expanded")).toBe("false");
    await more.trigger("click");
    expect(wrapper.get(".feed-body").text()).toContain("And a second paragraph.");
    expect(wrapper.get(".feed-more").attributes("aria-expanded")).toBe("true");
    // The row still leads to the note itself.
    expect(wrapper.get("a.feed-head").attributes("href")).toBe("/book/1#entry-11");
    await wrapper.get(".feed-more").trigger("click");
    expect(wrapper.get(".feed-body").text()).not.toContain("second paragraph");
  });

  it("offers nothing to unfold for short notes or shielded spoilers", async () => {
    const short = await mountClub([
      feedItem({ entry: { ...feedItem().entry, body: "Loved it.", spoiler_upto: null } }),
    ]);
    expect(short.find(".feed-more").exists()).toBe(false);
    const long = `${SECRET} `.repeat(8);
    const shielded = await mountClub([feedItem({ entry: { ...feedItem().entry, body: long } })]);
    expect(shielded.find(".feed-more").exists()).toBe(false);
    expect(shielded.html()).not.toContain(SECRET);
  });

  it("asks for one club's feed, or the combined one without a slug", async () => {
    await mountClub([], { club: "readers-ab12" });
    expect(diaryFeed).toHaveBeenLastCalledWith(undefined, 8, "readers-ab12");
    await mountClub([]);
    expect(diaryFeed).toHaveBeenLastCalledWith(undefined, 8, undefined);
  });

  it("names the club of a club post in the combined feed only", async () => {
    const club = { slug: "readers-ab12", name: "Readers" };
    const combined = await mountClub([feedItem({ club, entry: { ...feedItem().entry, club } })]);
    expect(combined.get(".feed-club").text()).toBe("Readers");
    expect(combined.get(".feed-club").attributes("href")).toBe("/c/readers-ab12");
    const scoped = await mountClub([feedItem({ club, entry: { ...feedItem().entry, club } })], {
      club: "readers-ab12",
    });
    expect(scoped.find(".feed-club").exists()).toBe(false);
  });

  it("leaves out the club's posts on the thread the page shows, reading on past a page of them", async () => {
    const club = { slug: "readers-ab12", name: "Readers" };
    const threadPost = (id: number) =>
      feedItem({ club, book: bookOut({ id: 7, title: "Kairos" }), entry: { ...feedItem().entry, id, club } });
    const otherBook = feedItem({ club, entry: { ...feedItem().entry, id: 3, club } });
    const diaryEntry = feedItem({ book: bookOut({ id: 7, title: "Kairos" }), entry: { ...feedItem().entry, id: 2 } });
    diaryFeed
      .mockResolvedValueOnce({ items: [threadPost(9), threadPost(8)], has_more: true, timezone: "UTC" })
      .mockResolvedValueOnce({ items: [threadPost(5), otherBook, diaryEntry], has_more: false, timezone: "UTC" });
    const wrapper = await mountFeed({ club: "readers-ab12", threadWork: 7 });
    expect(diaryFeed).toHaveBeenNthCalledWith(1, undefined, 8, "readers-ab12");
    expect(diaryFeed).toHaveBeenNthCalledWith(2, 8, 8, "readers-ab12");
    // A member's own diary entry on that book is not in the club thread, so it stays.
    expect(wrapper.findAll(".feed-list > li")).toHaveLength(2);
    expect(wrapper.find(".feed-load-more").exists()).toBe(false);
  });

  it("dates a note relative to now, counting days in the reader's zone, a club post too", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    try {
      // 2 Sep 06:00 in Tokyo, still 1 Sep (17:00) in New York.
      vi.setSystemTime(new Date("2026-09-01T21:00:00Z"));
      const pinia = createPinia();
      setActivePinia(pinia);
      const club = { slug: "readers-ab12", name: "Readers" };
      useSession().user = user({
        timezone: "America/New_York",
        clubs: [{ ...club, role: "member", timezone: "Asia/Tokyo", member_count: 2 }],
      });
      useLocale().locale = "de";
      diaryFeed.mockResolvedValue({
        items: [
          feedItem({ club, entry: { ...feedItem().entry, id: 12, club } }),
          feedItem(),
        ],
        has_more: false,
        timezone: "UTC",
      });
      const wrapper = mount(DiaryFeed, { global: { plugins: [pinia, i18n], stubs: { BookCover: true } } });
      await flushPromises();
      const times = wrapper.findAll(".feed-foot time");
      // Days counted in the reader's New York for both; the club's time stays in the title.
      expect(times.map((time) => time.text())).toEqual(["vor 9 Std.", "vor 9 Std."]);
      expect(times[0]!.attributes("datetime")).toBe("2026-09-01T12:00:00Z");
      expect(times[0]!.attributes("title")).toBe("Di., 1. Sept. 2026, 21:00 GMT+9");
      expect(times[1]!.attributes("title")).toBe("Di., 1. Sept. 2026, 08:00 GMT-4");
    } finally {
      vi.useRealTimers();
    }
  });

  describe("with mine (/diary)", () => {
    const own = (id: number, overrides: Partial<DiaryFeedItem["entry"]> = {}) =>
      feedItem({ entry: post({ id, author: "ada", mine: true, body: `Entry ${id}`, ...overrides }) });

    it("lists the reader's own diary writing, asked of the server with mine", async () => {
      diaryFeed
        .mockResolvedValueOnce({
          items: [own(10), { ...own(8, { parent_id: 2 }), parent_author: "anni" }],
          has_more: true,
          timezone: "UTC",
        })
        .mockResolvedValueOnce({ items: [own(7, { visibility: "only_me" })], has_more: false, timezone: "UTC" });
      const wrapper = await mountFeed({ mine: true, pageSize: 2 });
      expect(diaryFeed.mock.calls).toEqual([[undefined, 2, undefined, true]]);
      expect(wrapper.findAll(".feed-body").map((row) => row.text())).toEqual(["Entry 10", "Entry 8"]);
      // Rows lead with the book; a reply says whom it answered.
      const reply = wrapper.findAll(".feed-head")[1];
      expect(reply.text()).toContain("The Wedding People");
      expect(reply.text()).toContain("your reply to anni");
      // Roots carry their "Only me" switch; replies have no visibility of their own.
      expect(wrapper.findAll(".feed-only-me")).toHaveLength(1);

      await wrapper.get(".feed-load-more").trigger("click");
      await flushPromises();
      expect(diaryFeed).toHaveBeenLastCalledWith(8, 2, undefined, true);
      expect(wrapper.findAll(".feed-body").map((row) => row.text())).toEqual([
        "Entry 10",
        "Entry 8",
        "Only you Entry 7",
      ]);
      expect(wrapper.findAll(".feed-only-me")).toHaveLength(2);
      expect(wrapper.find(".feed-load-more").exists()).toBe(false);
    });

    it("keeps the switch on the foot line and sets the book title as a title", async () => {
      diaryFeed.mockResolvedValue({ items: [own(10)], has_more: false, timezone: "UTC" });
      const wrapper = await mountFeed({ mine: true });
      expect(wrapper.find(".feed-foot-row .feed-only-me").exists()).toBe(true);
      expect(wrapper.get(".feed-head .feed-title").text()).toBe("The Wedding People");
    });

    it("can keep its heading for screen readers only, under a page title that says it", async () => {
      diaryFeed.mockResolvedValue({ items: [own(10)], has_more: false, timezone: "UTC" });
      const wrapper = await mountFeed({ mine: true, heading: "Your entries", headingHidden: true });
      const heading = wrapper.get(".section-head h2");
      expect(heading.text()).toBe("Your entries");
      expect(heading.classes()).toContain("visually-hidden");
    });

    it("switches an entry to only me and back", async () => {
      diaryFeed.mockResolvedValue({ items: [own(10)], has_more: false, timezone: "UTC" });
      editDiaryEntry.mockResolvedValue(post({ id: 10, mine: true, visibility: "only_me" }));
      const wrapper = await mountFeed({ mine: true });
      await wrapper.get(".feed-only-me input").setValue(true);
      await flushPromises();
      expect(editDiaryEntry).toHaveBeenCalledWith(10, { visibility: "only_me" });
      expect(wrapper.get(".only-me-badge").text()).toBe("Only you");

      editDiaryEntry.mockResolvedValue(post({ id: 10, mine: true, visibility: "audience" }));
      await wrapper.get(".feed-only-me input").setValue(false);
      await flushPromises();
      expect(editDiaryEntry).toHaveBeenLastCalledWith(10, { visibility: "audience" });
      expect(wrapper.find(".only-me-badge").exists()).toBe(false);
    });
  });
});

describe("DiaryFeed rows", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    diaryFeed.mockReset();
  });

  it("keeps each reaction's emoji and count together", async () => {
    const item = feedItem();
    item.entry.spoiler_upto = null;
    item.entry.reactions = [
      { emoji: "😮", count: 1, mine: false, users: ["ada"] },
      { emoji: "❤️", count: 2, mine: false, users: ["ada", "bo"] },
    ];
    const wrapper = await mountClub([item], { club: "readers-ab12" });
    expect(wrapper.findAll(".feed-reaction").map((r) => r.text())).toEqual(["😮\u00a01", "❤️\u00a02"]);
  });

  it("shows rows shaped like the feed while it loads", () => {
    diaryFeed.mockReturnValue(new Promise(() => {}));
    const wrapper = mount(DiaryFeed, { global: { plugins: [createPinia(), i18n], stubs: { BookCover: true } } });
    expect(wrapper.findAll(".skeleton-rows .skeleton-row")).toHaveLength(3);
  });
});

describe("DiaryFeed preview (Home)", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    diaryFeed.mockReset();
    useSession().user = user({ username: "ada" });
  });

  it("asks only the preview's rows and links to the whole feed instead of paging", async () => {
    diaryFeed.mockResolvedValue({ items: [feedItem()], has_more: true, timezone: "UTC" });
    const wrapper = await mountFeed({ limit: 4, moreTo: "/diary" });
    expect(diaryFeed).toHaveBeenCalledWith(undefined, 4, undefined);
    expect(wrapper.get(".feed-all").attributes("href")).toBe("/diary");
    expect(wrapper.get(".feed-all").text()).toBe("See all");
    expect(wrapper.find(".feed-load-more").exists()).toBe(false);
  });

  it("pages in place without a page to link to, and links nowhere when all is shown", async () => {
    diaryFeed.mockResolvedValue({ items: [feedItem()], has_more: true, timezone: "UTC" });
    let wrapper = await mountFeed({ limit: 4 });
    expect(wrapper.find(".feed-all").exists()).toBe(false);
    await wrapper.get(".feed-load-more").trigger("click");
    await flushPromises();
    expect(diaryFeed).toHaveBeenLastCalledWith(11, 4, undefined);

    diaryFeed.mockResolvedValue({ items: [feedItem()], has_more: false, timezone: "UTC" });
    wrapper = await mountFeed({ limit: 4, moreTo: "/diary" });
    expect(wrapper.find(".feed-all").exists()).toBe(false);
    expect(wrapper.find(".feed-load-more").exists()).toBe(false);
  });
});
