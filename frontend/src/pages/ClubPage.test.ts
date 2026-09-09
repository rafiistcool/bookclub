import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DiaryFeedItem } from "../types";

const diaryFeed = vi.fn();
const members = vi.fn();
const overlap = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: {
    diaryFeed: (...args: unknown[]) => diaryFeed(...args),
    members: (...args: unknown[]) => members(...args),
    overlap: (...args: unknown[]) => overlap(...args),
  },
}));

import { i18n } from "../i18n";
import ClubPage from "./ClubPage.vue";

const SECRET = "The coconut pillow on p. 237 is a tell.";

function feedItem(overrides: Partial<DiaryFeedItem> = {}): DiaryFeedItem {
  return {
    parent_author: null,
    my_progress: 20,
    my_status: "currently_reading",
    book: {
      id: 1,
      ol_work_key: "/works/OL1W",
      title: "The Wedding People",
      authors: "Alison Espach",
      cover_id: 9,
      year: 2024,
      cover_url: null,
    },
    entry: {
      id: 11,
      author: "anni",
      mine: false,
      body: SECRET,
      deleted: false,
      spoiler_upto: 80,
      progress_at: 80,
      status_at: "currently_reading",
      author_rating: null,
      parent_id: null,
      created_at: "2026-09-01T12:00:00Z",
      created_label: "Sep 1",
      edited: false,
      reactions: [],
      replies: [],
    },
    ...overrides,
  };
}

async function mountClub(items: DiaryFeedItem[]) {
  members.mockResolvedValue([]);
  overlap.mockResolvedValue({ items: [] });
  diaryFeed.mockResolvedValue({ items, has_more: false, timezone: "UTC" });
  const wrapper = mount(ClubPage, {
    global: {
      plugins: [createPinia(), i18n],
      stubs: { NextUpVote: true, BookCover: true },
    },
  });
  await flushPromises();
  return wrapper;
}

describe("ClubPage recently written", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    diaryFeed.mockReset();
    members.mockReset();
    overlap.mockReset();
  });

  it("does not leak a spoiler-flagged excerpt when the viewer is behind", async () => {
    const wrapper = await mountClub([feedItem()]);
    const row = wrapper.get(".feed-body");
    expect(row.classes()).toContain("shielded");
    expect(row.text()).toContain("spoiler-flagged");
    expect(row.text()).toContain("Spoiler-flagged note");
    expect(row.text()).not.toContain(SECRET);
    expect(wrapper.html()).not.toContain(SECRET);
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
    expect(row.classes()).not.toContain("shielded");
    expect(row.text()).toContain(SECRET);
  });
});
