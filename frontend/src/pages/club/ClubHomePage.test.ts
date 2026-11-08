import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bookOut, clubOut, clubPick, nomination, shelfItem, user, vote } from "../../test/fixtures";
import type { ClubPick } from "../../types";

const api = {
  clubPick: vi.fn(),
  clubPickHistory: vi.fn(),
  vote: vi.fn(),
  patchClubPick: vi.fn(),
  setClubPick: vi.fn(),
  meetingIcsPath: (slug: string) => `/api/clubs/${slug}/pick/meeting.ics`,
};

vi.mock("../../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => (api as Record<string, unknown>)[key] }),
}));

import { i18n } from "../../i18n";
import { useClubs } from "../../stores/clubs";
import { useFinish } from "../../stores/finish";
import { useSession } from "../../stores/session";
import ClubHomePage from "./ClubHomePage.vue";

const PICK = clubPick({
  id: 5,
  book: bookOut({ id: 9, title: "Circe" }),
  meeting_at: "2026-11-05T18:00:00Z",
  meeting_local: "2026-11-05T19:00",
  meeting_label: "Thu 5 Nov",
  note: "Bring snacks",
});

// The diary stub answers the page's reload after a progress save or a finish.
const BookDiary = { name: "BookDiary", template: "<div class=\"diary-stub\" />", methods: { reload: () => Promise.resolve() } };
const STUBS = { BookCover: true, BookDiary, MilestoneList: true, ProgressEditor: true };

async function mountAs(role: "owner" | "admin" | "member", pick: ClubPick = PICK, history: ClubPick[] = []) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const club = clubOut({ role, member_count: 4 });
  useSession().user = user({ clubs: [{ ...club }] });
  useClubs().current = club;
  api.clubPick.mockResolvedValue({ pick, timezone: "Europe/Berlin" });
  api.clubPickHistory.mockResolvedValue({ items: history, timezone: "Europe/Berlin" });
  api.vote.mockResolvedValue(vote({ nominations: [nomination({ id: 2, votes: 2 })] }));
  const wrapper = mount(ClubHomePage, {
    global: {
      plugins: [pinia, i18n],
      stubs: STUBS,
    },
  });
  await flushPromises();
  return wrapper;
}

describe("ClubHomePage", () => {
  beforeEach(() => {
    api.clubPick.mockReset();
    api.clubPickHistory.mockReset();
    api.vote.mockReset();
    api.patchClubPick.mockReset();
    api.setClubPick.mockReset();
  });

  it("shows the club's pick, its meeting and a calendar link", async () => {
    const wrapper = await mountAs("member");
    expect(api.clubPick).toHaveBeenCalledWith("readers-ab12");
    expect(wrapper.get(".hero-title").text()).toBe("Circe");
    expect(wrapper.text()).toContain("Bring snacks");
    expect(wrapper.get(".meeting a").attributes("href")).toBe("/api/clubs/readers-ab12/pick/meeting.ics");
    expect(wrapper.get(".next-up-card").attributes("href")).toBe("/c/readers-ab12/vote");
    // The card is the link: no "Vote →" in it; the section head keeps its one "Vote" link.
    expect(wrapper.get(".next-up-card").text()).not.toContain("→");
    expect(wrapper.findAll('.next-up-section a[href="/c/readers-ab12/vote"]')).toHaveLength(2);
    // The milestone list is the section itself, with no wrapper that would stay when it renders nothing.
    expect(wrapper.get("milestone-list-stub").classes()).toEqual(["section", "milestone-section"]);
    expect(wrapper.get("milestone-list-stub").element.parentElement?.classList.contains("club-home")).toBe(true);
    // No caps kicker over the title, and the cover and title are the links (no "Open book" button).
    expect(wrapper.find(".hero .kicker").exists()).toBe(false);
    expect(wrapper.find(".hero-actions").exists()).toBe(false);
    expect(wrapper.get(".hero-title").attributes("href")).toBe("/book/9");
  });

  it("dates a past pick by its meeting day, without the weekday and time, the year only before this one", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-11-07T12:00:00Z"));
    try {
      const past = { ...PICK, id: 4, book: bookOut({ id: 8, title: "Kairos" }), meeting_at: "2026-10-15T17:30:00Z" };
      const older = { ...past, id: 3, book: bookOut({ id: 7, title: "Circe" }), meeting_at: "2025-10-15T17:30:00Z" };
      const wrapper = await mountAs("member", PICK, [past, older]);
      expect(wrapper.findAll(".past-tile .book-tile-sub").map((tile) => tile.text())).toEqual(["15 Oct", "15 Oct 2025"]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("puts the one next step under the pick, by the viewer's shelf", async () => {
    const fresh = await mountAs("member");
    expect(fresh.get(".hero-action .start-reading").text()).toBe("Start reading");
    fresh.unmount();
    const done = await mountAs("member", { ...PICK, on_shelf: "finished", shelf_id: 30 });
    expect(done.get(".hero-action .to-discussion").attributes("href")).toBe("/c/readers-ab12/discussions");
    expect(done.find(".hero-action .start-reading").exists()).toBe(false);
  });

  it("finishes the book from the progress editor and reloads where everyone is", async () => {
    const reading = {
      ...PICK,
      on_shelf: "currently_reading" as const,
      shelf_id: 30,
      readers: [
        { username: "ada", status: "currently_reading" as const, progress: 90, rating: null, take: "", dnf_reason: "", current_page: null, progress_pages: null, avatar_url: null },
        { username: "grace", status: "finished" as const, progress: null, rating: 4, take: "Loved it", dnf_reason: "", current_page: null, progress_pages: null, avatar_url: null },
      ],
    };
    const wrapper = await mountAs("member", reading);
    const editor = wrapper.findComponent({ name: "ProgressEditor" });
    expect(editor.props("shelfId")).toBe(30);
    expect(editor.props("clubSlug")).toBe("readers-ab12");
    expect(wrapper.text()).toContain("1 of 4 finished");
    // The rings get the readers and the club's size; the takes follow as rows.
    const rings = wrapper.findComponent({ name: "ReaderRings" });
    expect(rings.props("size")).toBe("md");
    expect(rings.props("memberCount")).toBe(4);
    expect(rings.props("readers").map((row: { username: string }) => row.username)).toEqual(["ada", "grace"]);
    expect(rings.findAll("li .visually-hidden").map((row) => row.text())).toContain("grace: Finished");
    expect(wrapper.get(".reader-takes").text()).toContain("Loved it");

    api.clubPick.mockResolvedValue({
      pick: { ...reading, on_shelf: "finished", readers: reading.readers.map((row) => ({ ...row, status: "finished" as const, progress: null })) },
      timezone: "Europe/Berlin",
    });
    editor.vm.$emit("finished", shelfItem({ id: 30, status: "finished" }));
    await flushPromises();
    expect(api.clubPick).toHaveBeenCalledTimes(2);
    expect(wrapper.findComponent({ name: "ProgressEditor" }).exists()).toBe(false);
    expect(wrapper.find(".hero-action .to-discussion").exists()).toBe(true);
    expect(wrapper.text()).toContain("2 of 4 finished");

    // In the moment the reader gives five stars and a take: the takes show it without a reload of the page.
    const rated = { ...reading.readers[0]!, status: "finished" as const, progress: null, rating: 5, take: "Fierce" };
    api.clubPick.mockResolvedValue({
      pick: { ...reading, on_shelf: "finished", readers: [rated, reading.readers[1]!] },
      timezone: "Europe/Berlin",
    });
    const finish = useFinish();
    finish.celebrate({ item: shelfItem({ id: 30, status: "finished" }), clubSlug: "readers-ab12" });
    finish.close(shelfItem({ id: 30, status: "finished", rating: 5, take: "Fierce" }), finish.serial);
    await flushPromises();
    expect(api.clubPick).toHaveBeenCalledTimes(3);
    expect(wrapper.get(".reader-takes").text()).toContain("Fierce");

    // Another book's stars are not this pick's business.
    finish.celebrate({ item: shelfItem({ id: 31, status: "finished" }) });
    finish.close(shelfItem({ id: 31, status: "finished", rating: 2 }), finish.serial);
    await flushPromises();
    expect(api.clubPick).toHaveBeenCalledTimes(3);
  });

  it("counts the last day before the meeting in hours", async () => {
    const soon = new Date(Date.now() + 3 * 60 * 60_000 + 5 * 60_000).toISOString();
    const wrapper = await mountAs("member", { ...PICK, meeting_at: soon });
    expect(wrapper.get(".meeting .countdown").text()).toBe("in 3 hours");
  });

  it("hides the meeting edit from members", async () => {
    const wrapper = await mountAs("member");
    expect(wrapper.find(".meeting-edit").exists()).toBe(false);
  });

  it("edits the meeting with PATCH, leaving the note and the setter alone", async () => {
    const wrapper = await mountAs("admin");
    await wrapper.get(".meeting-edit").trigger("click");
    const input = wrapper.get(".sheet input[type=datetime-local]");
    expect((input.element as HTMLInputElement).value).toBe("2026-11-05T19:00");
    await input.setValue("2026-11-12T19:30");
    api.patchClubPick.mockResolvedValue({ ...PICK, meeting_local: "2026-11-12T19:30" });
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();
    expect(api.patchClubPick).toHaveBeenCalledWith("readers-ab12", { meeting_at: "2026-11-12T19:30" });
    expect(api.setClubPick).not.toHaveBeenCalled();
  });

  it("clears the meeting with an explicit null", async () => {
    const wrapper = await mountAs("owner");
    await wrapper.get(".meeting-edit").trigger("click");
    await wrapper.get(".sheet .clear-btn").trigger("click");
    api.patchClubPick.mockResolvedValue({ ...PICK, meeting_at: null, meeting_local: null });
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();
    expect(api.patchClubPick).toHaveBeenCalledWith("readers-ab12", { meeting_at: null });
    expect(wrapper.text()).toContain("No meeting scheduled yet.");
  });

  it("tells members who picks when there is no pick", async () => {
    const wrapper = await mountAs("member");
    api.clubPick.mockResolvedValue({ pick: null, timezone: "Europe/Berlin" });
    const empty = mount(ClubHomePage, {
      global: { plugins: [i18n], stubs: STUBS },
    });
    await flushPromises();
    expect(empty.get(".empty").text()).toContain("An admin sets the next book");
    wrapper.unmount();
  });

  describe("the welcome after joining", () => {
    afterEach(() => {
      window.history.replaceState(null, "");
      Reflect.deleteProperty(HTMLElement.prototype, "scrollIntoView");
    });

    it("greets a reader who just joined, once, and leads to the start button", async () => {
      window.history.replaceState({ current: "/c/readers-ab12", joined: true }, "");
      const scrolled = vi.fn();
      HTMLElement.prototype.scrollIntoView = scrolled;
      const wrapper = await mountAs("member");
      const welcome = wrapper.get(".welcome");
      expect(welcome.get(".welcome-title").text()).toBe("Welcome to Readers!");
      expect(welcome.text()).toContain("This is what you're all reading.");
      // Taken out of the history entry: a reload or a later visit shows none.
      expect(window.history.state).toEqual({ current: "/c/readers-ab12", joined: undefined });

      await welcome.get(".welcome-action").trigger("click");
      expect(wrapper.find(".welcome").exists()).toBe(false);
      expect(scrolled).toHaveBeenCalledTimes(1);
      expect(scrolled.mock.instances[0]).toBe(wrapper.get(".start-reading").element);

      const again = await mountAs("member");
      expect(again.find(".welcome").exists()).toBe(false);
    });

    it("can be dismissed", async () => {
      window.history.replaceState({ joined: true }, "");
      const wrapper = await mountAs("member");
      await wrapper.get(".welcome-close").trigger("click");
      expect(wrapper.find(".welcome").exists()).toBe(false);
    });

    it("points to the vote while the club has no book", async () => {
      window.history.replaceState({ joined: true }, "");
      const pinia = createPinia();
      setActivePinia(pinia);
      const club = clubOut({ role: "member", member_count: 4 });
      useSession().user = user({ clubs: [{ ...club }] });
      useClubs().current = club;
      api.clubPick.mockResolvedValue({ pick: null, timezone: "Europe/Berlin" });
      api.clubPickHistory.mockResolvedValue({ items: [], timezone: "Europe/Berlin" });
      api.vote.mockResolvedValue(vote({ nominations: [] }));
      const wrapper = mount(ClubHomePage, { global: { plugins: [pinia, i18n], stubs: STUBS } });
      await flushPromises();
      expect(wrapper.get(".welcome").text()).toContain("No book yet. Suggest one.");
      expect(wrapper.get(".welcome-action").attributes("href")).toBe("/c/readers-ab12/vote");
    });

    it("stays away from an ordinary visit", async () => {
      const wrapper = await mountAs("member");
      expect(wrapper.find(".welcome").exists()).toBe(false);
    });
  });
});
