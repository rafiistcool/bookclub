import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PickPost, PickThread as Thread } from "../types";

vi.mock("../api/client", async () => {
  const actual = await vi.importActual<typeof import("../api/client")>("../api/client");
  return {
    ...actual,
    api: {
      pickPosts: vi.fn(),
      addPickPost: vi.fn(),
      editPickPost: vi.fn(),
      deletePickPost: vi.fn(),
      toggleReaction: vi.fn(),
    },
  };
});

import { api } from "../api/client";
import PickThread from "./PickThread.vue";

const mocked = api as unknown as Record<string, ReturnType<typeof vi.fn>>;

function post(overrides: Partial<PickPost>): PickPost {
  return {
    id: 1,
    author: "mara",
    mine: false,
    body: "The ending!",
    spoiler_upto: null,
    milestone_id: null,
    created_at: "2026-09-01T10:00:00Z",
    created_label: "Tue 01 Sep 2026, 12:00 CEST",
    edited: false,
    reactions: [],
    ...overrides,
  };
}

function thread(items: PickPost[], my_progress: number | null): Thread {
  return { pick_id: 1, can_post: true, my_progress, my_status: "currently_reading", items, timezone: "UTC" };
}

describe("PickThread spoiler shield", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it("blurs notes flagged past my progress and reveals on tap", async () => {
    mocked.pickPosts.mockResolvedValue(
      thread([post({ id: 1, spoiler_upto: 80 }), post({ id: 2, spoiler_upto: 20, body: "Early bit" })], 50),
    );
    const wrapper = mount(PickThread, { props: { pickId: 1 } });
    await flushPromises();

    const bodies = wrapper.findAll(".note-body");
    expect(bodies[0].classes()).toContain("hidden-spoiler");
    expect(bodies[1].classes()).not.toContain("hidden-spoiler");
    expect(wrapper.text()).toContain("you’re at 50%");

    await wrapper.get(".spoiler-cover button").trigger("click");
    expect(wrapper.findAll(".note-body")[0].classes()).not.toContain("hidden-spoiler");
  });

  it("hides flagged notes entirely when I have no progress yet, never my own", async () => {
    mocked.pickPosts.mockResolvedValue(
      thread([post({ id: 1, spoiler_upto: 10 }), post({ id: 2, spoiler_upto: 90, mine: true, author: "ada" })], null),
    );
    const wrapper = mount(PickThread, { props: { pickId: 1 } });
    await flushPromises();
    const bodies = wrapper.findAll(".note-body");
    expect(bodies[0].classes()).toContain("hidden-spoiler");
    expect(bodies[1].classes()).not.toContain("hidden-spoiler");
    expect(wrapper.text()).toContain("set your progress");
  });

  it("shows edit and delete only on my notes and posts with a spoiler flag", async () => {
    mocked.pickPosts.mockResolvedValue(thread([post({ id: 1 }), post({ id: 2, mine: true, author: "ada" })], 30));
    mocked.addPickPost.mockResolvedValue(post({ id: 3, mine: true, author: "ada", body: "New", spoiler_upto: 60 }));
    const wrapper = mount(PickThread, { props: { pickId: 1 } });
    await flushPromises();

    expect(wrapper.findAll('[aria-label="Delete note"]')).toHaveLength(1);
    expect(wrapper.findAll('[aria-label="Edit note"]')).toHaveLength(1);

    await wrapper.get("textarea").setValue("New");
    await wrapper.get('input[type="checkbox"]').setValue(true);
    await wrapper.get('input[type="number"]').setValue("60");
    await wrapper.get("form").trigger("submit");
    await flushPromises();

    expect(mocked.addPickPost).toHaveBeenCalledWith({ body: "New", spoiler_upto: 60, milestone_id: null }, 1);
    expect(wrapper.findAll(".note")).toHaveLength(3);
  });

  it("toggles reactions through the API and re-renders counts", async () => {
    mocked.pickPosts.mockResolvedValue(thread([post({ id: 1 })], 30));
    mocked.toggleReaction.mockResolvedValue(
      post({ id: 1, reactions: [{ emoji: "❤️", count: 1, mine: true, users: ["ada"] }] }),
    );
    const wrapper = mount(PickThread, { props: { pickId: 1 } });
    await flushPromises();
    await wrapper.get('[aria-label="Add reaction"]').trigger("click");
    await wrapper.get('[aria-label="React ❤️"]').trigger("click");
    await flushPromises();
    expect(mocked.toggleReaction).toHaveBeenCalledWith(1, "❤️");
    const chip = wrapper.get(".reaction.mine");
    expect(chip.text()).toContain("❤️ 1");
  });
});
