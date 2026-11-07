import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { clubOut, clubPick, nomination, user, vote } from "../test/fixtures";

const api = { vote: vi.fn(), castVote: vi.fn(), applyVote: vi.fn(), setVoteDeadline: vi.fn() };

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => (api as Record<string, unknown>)[key] }),
}));

import { i18n } from "../i18n";
import { useClubs } from "../stores/clubs";
import { useSession } from "../stores/session";
import NextUpVote from "./NextUpVote.vue";

const OPEN = vote({
  vote_id: 4,
  nominations: [nomination({ id: 1, votes: 2, voters: ["ada", "mia"] }), nomination({ id: 2, votes: 0 })],
  voted_count: 2,
  member_count: 3,
  not_voted: ["max"],
});

async function mountVote(canManage: boolean, answer = OPEN) {
  const pinia = createPinia();
  setActivePinia(pinia);
  api.vote.mockResolvedValue(answer);
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
    // A list, not the empty state under it, and no confirm for a member.
    expect(member.find(".empty").exists()).toBe(false);
    expect(member.find(".confirm-winner").exists()).toBe(false);
    expect(member.get(".vote-status").text()).toContain("2 of 3 voted");
    expect(member.get(".vote-status").text()).toContain("max");
    expect(member.text()).not.toContain("Confirm winner");
    expect(member.text()).not.toContain("Set a deadline");

    api.castVote.mockResolvedValue({ ...OPEN, my_vote_id: 2 });
    await member.findAll(".vote-actions .btn")[1]!.trigger("click");
    expect(api.castVote).toHaveBeenCalledWith("readers-ab12", 2);

    // A member is told that voting alone does not swap the book.
    expect(member.get(".vote-blurb").text()).toBe(i18n.global.t("vote.blurb"));
    expect(member.get("#next-up-heading").text()).toBe("Next book");

    const admin = await mountVote(true);
    expect(admin.get(".vote-blurb").text()).toBe(i18n.global.t("vote.blurbAdmin"));
    expect(admin.text()).toContain("Confirm winner");
    expect(admin.text()).toContain("Set a deadline");
  });

  it("confirms a winner with the meeting through the club's vote", async () => {
    const wrapper = await mountVote(true);
    useClubs().current = clubOut({ current_pick: null });
    api.applyVote.mockResolvedValue({ pick: clubPick({ id: 9, meeting_at: "2026-12-01T18:00:00Z" }), vote: vote() });
    // One confirm for the whole list: it asks which book, then when to meet.
    await wrapper.get(".confirm-winner").trigger("click");
    // The chooser only lists the books; the consequence is said once, at the final confirm.
    const blurb = i18n.global.t("vote.confirmBlurb");
    expect(wrapper.get(".sheet").text()).not.toContain(blurb);
    await wrapper.findAll(".sheet .winner-choice")[0]!.trigger("click");
    expect(wrapper.findAll(".sheet")).toHaveLength(1);
    expect(wrapper.get(".sheet").text()).toContain(blurb);
    await wrapper.get(".sheet input[type=datetime-local]").setValue("2026-12-01T19:00");
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();
    expect(api.applyVote).toHaveBeenCalledWith("readers-ab12", 1, "2026-12-01T19:00");
    expect(wrapper.emitted("applied")).toHaveLength(1);
    // The club's other pages (the discussion) see the new pick without asking again.
    expect(useClubs().current?.current_pick).toMatchObject({ id: 9, meeting_at: "2026-12-01T18:00:00Z" });
  });

  it("moves the ballot at once and takes it back when the server refuses", async () => {
    const wrapper = await mountVote(false);
    useSession().user = user({ username: "max" });
    let refuse: (err: unknown) => void = () => {};
    api.castVote.mockReturnValue(new Promise((_resolve, reject) => (refuse = reject)));
    await wrapper.findAll(".vote-actions .btn")[1]!.trigger("click");
    // Before the server answers: the count, the pressed state and the turnout.
    const second = wrapper.findAll(".vote-row")[1]!;
    const count = (row: typeof second) => row.get(".tally-count:not(.count-leave-active)").text();
    expect(count(second)).toBe("1");
    // The book with the most votes still leads, as the server would say.
    expect(wrapper.findAll(".vote-row")[0]!.classes()).toContain("leading");
    expect(second.get(".vote-btn").attributes("aria-pressed")).toBe("true");
    expect(wrapper.get(".vote-status").text()).toContain("3 of 3 voted");
    refuse(new Error("offline"));
    await flushPromises();
    expect(count(wrapper.findAll(".vote-row")[1]!)).toBe("0");
    expect(wrapper.get(".vote-status").text()).toContain("2 of 3 voted");
  });

  it("shows only the skeleton while the vote loads", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    api.vote.mockReturnValue(new Promise(() => {}));
    const wrapper = mount(NextUpVote, {
      props: { slug: "readers-ab12", canManage: true },
      global: { plugins: [pinia, i18n], stubs: { BookCover: true } },
    });
    await flushPromises();
    expect(wrapper.find(".skeleton").exists()).toBe(true);
    expect(wrapper.find(".empty").exists()).toBe(false);
  });

  it("names who is missing, singular or plural, in German too", async () => {
    i18n.global.locale.value = "de";
    try {
      const one = await mountVote(false);
      expect(one.findAll(".vote-status .vote-fact")[1]!.text()).toBe("es fehlt noch: max");
      const two = await mountVote(false, { ...OPEN, voted_count: 1, not_voted: ["max", "tim"] });
      expect(two.findAll(".vote-status .vote-fact")[1]!.text()).toBe("es fehlen noch: max, tim");
    } finally {
      i18n.global.locale.value = "en";
    }
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
    // Each fact is its own unit; the dots between them come from CSS, so a
    // wrapped line never starts with one.
    expect(wrapper.findAll(".vote-status .vote-fact").map((fact) => fact.text())).toEqual([
      "2 of 3 voted",
      "still waiting for max",
      expect.stringMatching(/^closes /),
    ]);
    expect(wrapper.get(".vote-status").text()).not.toContain("·");
  });
});
