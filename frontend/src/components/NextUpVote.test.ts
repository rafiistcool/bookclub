import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { nomination, vote } from "../test/fixtures";

const api = { vote: vi.fn(), castVote: vi.fn(), applyVote: vi.fn(), setVoteDeadline: vi.fn() };

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => (api as Record<string, unknown>)[key] }),
}));

import { i18n } from "../i18n";
import NextUpVote from "./NextUpVote.vue";

const OPEN = vote({
  vote_id: 4,
  nominations: [nomination({ id: 1, votes: 2, voters: ["ada", "mia"] }), nomination({ id: 2, votes: 0 })],
  voted_count: 2,
  member_count: 3,
  not_voted: ["max"],
});

async function mountVote(canManage: boolean) {
  const pinia = createPinia();
  setActivePinia(pinia);
  api.vote.mockResolvedValue(OPEN);
  const wrapper = mount(NextUpVote, {
    props: { slug: "readers-ab12", canManage },
    global: { plugins: [pinia, i18n], stubs: { BookCover: true } },
  });
  await flushPromises();
  return wrapper;
}

describe("NextUpVote", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
  });

  it("lets every member vote in this club, and only admins confirm or set a deadline", async () => {
    const member = await mountVote(false);
    expect(api.vote).toHaveBeenCalledWith("readers-ab12");
    expect(member.get(".vote-status").text()).toContain("2 of 3 voted");
    expect(member.get(".vote-status").text()).toContain("max");
    expect(member.text()).not.toContain("Confirm winner");
    expect(member.text()).not.toContain("Set a deadline");

    api.castVote.mockResolvedValue({ ...OPEN, my_vote_id: 2 });
    await member.findAll(".vote-actions .btn")[1]!.trigger("click");
    expect(api.castVote).toHaveBeenCalledWith("readers-ab12", 2);

    const admin = await mountVote(true);
    expect(admin.text()).toContain("Confirm winner");
    expect(admin.text()).toContain("Set a deadline");
  });

  it("confirms a winner with the meeting through the club's vote", async () => {
    const wrapper = await mountVote(true);
    api.applyVote.mockResolvedValue({ pick: {}, vote: vote() });
    await wrapper.findAll(".vote-actions .text-btn")[0]!.trigger("click");
    await wrapper.get(".sheet input[type=datetime-local]").setValue("2026-12-01T19:00");
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();
    expect(api.applyVote).toHaveBeenCalledWith("readers-ab12", 1, "2026-12-01T19:00");
    expect(wrapper.emitted("applied")).toHaveLength(1);
  });

  it("sets the deadline in the club's zone", async () => {
    const wrapper = await mountVote(true);
    await wrapper.findAll(".vote-status .text-btn")[0]!.trigger("click");
    await wrapper.get(".sheet input[type=datetime-local]").setValue("2026-11-20T20:00");
    api.setVoteDeadline.mockResolvedValue({ ...OPEN, closes_at: "2026-11-20T19:00:00Z" });
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();
    expect(api.setVoteDeadline).toHaveBeenCalledWith("readers-ab12", "2026-11-20T20:00");
    expect(wrapper.get(".vote-status").text()).toContain("closes");
  });
});
