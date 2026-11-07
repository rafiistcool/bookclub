import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { bookOut, clubOut, clubPick, shelfItem, user } from "../test/fixtures";
import type { ClubOut, ClubPick, ShelfItem } from "../types";

const api = {
  clubPick: vi.fn(),
  addToShelf: vi.fn(),
  patchShelf: vi.fn(),
};

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status: number;
    detail: unknown;
    item?: unknown;
    constructor(message: string, status = 400, detail: unknown = message) {
      super(message);
      this.status = status;
      this.detail = detail;
    }
  },
  api: new Proxy({}, { get: (_target, key: string) => (api as Record<string, unknown>)[key] }),
}));

import { ApiError } from "../api/client";
import { useClubs } from "../stores/clubs";
import { useFinish } from "../stores/finish";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import HomeHero from "./HomeHero.vue";

const READERS = clubOut({ slug: "readers-ab12", name: "Readers", member_count: 4 });
const OWLS = clubOut({ slug: "night-owls-cd34", name: "Night owls", member_count: 2 });
const CIRCE = bookOut({ id: 7, title: "Circe", authors: "Madeline Miller", pages: 400 });

function pickOf(overrides: Partial<ClubPick> = {}): ClubPick {
  return clubPick({ book: CIRCE, ...overrides });
}

async function mountHero(
  props: { club: ClubOut | null; others?: ClubOut[]; entry?: ShelfItem | null; pending?: boolean },
  pick: ClubPick | null = pickOf(),
) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({
    clubs: [READERS, OWLS].map(({ slug, name, role, timezone, member_count }) => ({ slug, name, role, timezone, member_count })),
  });
  api.clubPick.mockResolvedValue({ pick, timezone: "Europe/Berlin" });
  const wrapper = mount(HomeHero, {
    props: { entry: null, ...props },
    // The swap's cross-fade would keep the outgoing book on screen until its animation ends.
    global: { plugins: [pinia], stubs: { BookCover: true, transition: true } },
  });
  await flushPromises();
  return wrapper;
}

/** The hero's one action: what `.hero-action` holds. */
function actionOf(wrapper: Awaited<ReturnType<typeof mountHero>>): string[] {
  const action = wrapper.get(".hero-action");
  return [
    action.find(".progress-trigger").exists() && "progress",
    action.find(".hero-start").exists() && "start",
    action.find(".hero-rate").exists() && "rate",
    action.find(".hero-vote").exists() && "vote",
    action.find(".hero-open").exists() && "open",
  ].filter((name): name is string => Boolean(name));
}

describe("HomeHero", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows the club's book with the meeting and where everyone is, from one pick call", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-11-06T12:00:00Z"));
    const wrapper = await mountHero(
      { club: READERS },
      pickOf({
        meeting_at: "2026-11-19T18:30:00Z",
        readers: [
          { username: "ada", status: "currently_reading", rating: null, take: "", dnf_reason: "", progress: 20, current_page: null, progress_pages: null, avatar_url: null },
          { username: "grace", status: "finished", rating: 4, take: "", dnf_reason: "", progress: null, current_page: null, progress_pages: null, avatar_url: null },
        ],
        on_shelf: "currently_reading",
        shelf_id: 3,
      }),
    );
    expect(api.clubPick).toHaveBeenCalledTimes(1);
    expect(api.clubPick).toHaveBeenCalledWith("readers-ab12");
    expect(wrapper.get(".hero-club").attributes("href")).toBe("/c/readers-ab12");
    expect(wrapper.get(".hero-title").text()).toBe("Circe");
    expect(wrapper.get(".hero-authors").text()).toBe("Madeline Miller");
    // The "·" stays at the end of the first line when the date wraps.
    expect(wrapper.get(".hero-meeting").text()).toBe("Meeting in 13 days\u00a0· Thu 19 Nov");
    // The date never breaks inside ("Nov" alone on a line).
    expect(wrapper.get(".hero-meeting .meeting-date").text()).toBe("Thu 19 Nov");
    expect(wrapper.get(".hero-meeting .meeting-when").text()).toBe("in 13 days");
    expect(wrapper.get(".hero-meeting time").attributes("datetime")).toBe("2026-11-19T18:30:00Z");
    // My ring and grace's, plus the two members who have not started.
    expect(wrapper.findAll(".reader-rings li .visually-hidden").map((row) => row.text())).toEqual([
      "ada: 20%",
      "grace: finished",
      "2 not started yet",
    ]);
    expect(actionOf(wrapper)).toEqual(["progress"]);
    // The cover settles in on arrival: the page's one authored moment.
    expect(wrapper.get(".hero-cover").classes()).toContain("settle");
  });

  it("offers to start a book that is not on the shelf, then the progress", async () => {
    const wrapper = await mountHero({ club: READERS });
    expect(actionOf(wrapper)).toEqual(["start"]);
    // Not on my shelf: I am not among the rings.
    expect(wrapper.findAll(".reader-rings li .visually-hidden").map((row) => row.text())).toEqual(["4 not started yet"]);
    const created = shelfItem({ id: 9, status: "currently_reading", progress: null, book: CIRCE });
    api.addToShelf.mockResolvedValue(created);
    await wrapper.get(".hero-start").trigger("click");
    await flushPromises();
    expect(api.addToShelf).toHaveBeenCalledWith({ work_id: 7, status: "currently_reading" });
    expect(wrapper.emitted("entry")).toEqual([[created]]);
    expect(actionOf(wrapper)).toEqual(["progress"]);
    expect(wrapper.findAll(".reader-rings li .visually-hidden").map((row) => row.text())).toEqual([
      "ada: 0%",
      "3 not started yet",
    ]);
    expect(useToast().items.map((row) => row.message)).toContain("Added to Reading");
  });

  it("moves a wanted book to reading", async () => {
    const entry = shelfItem({ id: 5, status: "want_to_read", book: CIRCE });
    const wrapper = await mountHero({ club: READERS, entry });
    expect(actionOf(wrapper)).toEqual(["start"]);
    api.patchShelf.mockResolvedValue({ ...entry, status: "currently_reading", progress: null });
    await wrapper.get(".hero-start").trigger("click");
    await flushPromises();
    expect(api.patchShelf).toHaveBeenCalledWith(5, { status: "currently_reading", position: 0 });
    expect(wrapper.emitted("entry")?.[0]?.[0]).toMatchObject({ id: 5, status: "currently_reading" });
  });

  it("saves progress for the hero's entry with an Undo", async () => {
    const entry = shelfItem({ id: 5, status: "currently_reading", progress: 30, book: CIRCE });
    const wrapper = await mountHero({ club: READERS, entry });
    expect(actionOf(wrapper)).toEqual(["progress"]);
    // The pick's book counts in pages: 30 % of 400.
    expect(wrapper.get(".progress-trigger").text()).toContain("p. 120 of 400");
    await wrapper.get(".progress-trigger").trigger("click");
    await wrapper.get(".sheet input[type=range]").setValue("220");
    api.patchShelf.mockResolvedValue({ ...entry, progress: 55, current_page: 220 });
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();
    // The page itself: the server derives the percent.
    expect(api.patchShelf).toHaveBeenCalledWith(5, { current_page: 220 });
    expect(wrapper.emitted("entry")?.[0]?.[0]).toMatchObject({ id: 5, progress: 55, current_page: 220 });
    const toast = useToast();
    const saved = toast.items.find((row) => row.message === "Progress: p. 220 · 55%");
    expect(saved?.action?.label).toBe("Undo");
    // Back to the place before: it was saved in percent, so the percent comes back.
    api.patchShelf.mockClear();
    api.patchShelf.mockResolvedValue({ ...entry });
    await toast.run(saved!.id);
    await flushPromises();
    expect(api.patchShelf).toHaveBeenCalledWith(5, { progress: 30 });
  });

  it("undoes a save on the book it was made for, even after a chip swap", async () => {
    const circe = shelfItem({ id: 5, status: "currently_reading", progress: 30, book: CIRCE });
    const wrapper = await mountHero({ club: READERS, others: [READERS, OWLS], entry: circe });
    await wrapper.get(".progress-trigger").trigger("click");
    await wrapper.get(".sheet input[type=range]").setValue("220");
    api.patchShelf.mockResolvedValue({ ...circe, progress: 55 });
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();
    const toast = useToast();
    const saved = toast.items.find((row) => row.message === "Progress: 55%")!;

    // The Owls' pick is another book I am reading; Home hands the hero its entry.
    const dune = bookOut({ id: 8, title: "Dune", pages: 600 });
    const duneEntry = shelfItem({ id: 6, status: "currently_reading", progress: 80, book: dune });
    api.clubPick.mockResolvedValue({
      pick: clubPick({ book: dune, on_shelf: "currently_reading", shelf_id: 6 }),
      timezone: "UTC",
    });
    await wrapper.get(".hero-chip").trigger("click");
    await wrapper.setProps({ club: OWLS, others: [READERS, OWLS], entry: duneEntry });
    await flushPromises();
    expect(wrapper.get(".hero-title").text()).toBe("Dune");

    api.patchShelf.mockClear();
    api.patchShelf.mockResolvedValue({ ...circe, progress: 30 });
    await toast.run(saved.id);
    await flushPromises();
    expect(api.patchShelf).toHaveBeenCalledTimes(1);
    expect(api.patchShelf).toHaveBeenCalledWith(5, { progress: 30 });
  });

  it("shows no action for the outgoing book during a swap, and its saved progress meanwhile", async () => {
    const circe = shelfItem({ id: 5, status: "currently_reading", progress: 30, book: CIRCE });
    const wrapper = await mountHero(
      { club: READERS, others: [READERS, OWLS], entry: circe },
      pickOf({
        on_shelf: "currently_reading",
        shelf_id: 5,
        readers: [{ username: "ada", status: "currently_reading", rating: null, take: "", dnf_reason: "", progress: 30, current_page: null, progress_pages: null, avatar_url: null }],
      }),
    );
    await wrapper.get(".progress-trigger").trigger("click");
    await wrapper.get(".sheet input[type=range]").setValue("200");
    api.patchShelf.mockResolvedValue({ ...circe, progress: 50 });
    await wrapper.get(".sheet .btn-primary").trigger("click");
    await flushPromises();

    // The Owls' pick is slow: Circe stays on screen, but Home's entry is the Owls' already (none).
    let answer!: (value: unknown) => void;
    api.clubPick.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    await wrapper.setProps({ club: OWLS, others: [READERS, OWLS], entry: null });
    await flushPromises();
    expect(wrapper.get(".hero-title").text()).toBe("Circe");
    expect(wrapper.find(".hero-action").exists()).toBe(false);
    // My ring falls back to the pick, which took the save: 50 %, not the 30 % it was loaded with.
    expect(wrapper.findAll(".reader-rings li .visually-hidden").map((row) => row.text())).toContain("ada: 50%");
    answer({ pick: null, timezone: "UTC" });
    await flushPromises();
    expect(wrapper.find(".hero-action").exists()).toBe(true);
  });

  it("asks for the stars of a finished book through the finished moment", async () => {
    const entry = shelfItem({ id: 5, status: "finished", rating: null, book: CIRCE });
    const wrapper = await mountHero({ club: READERS, entry });
    expect(actionOf(wrapper)).toEqual(["rate"]);
    await wrapper.get(".hero-rate").trigger("click");
    const finish = useFinish();
    expect(finish.item).toEqual(entry);
    expect(finish.clubSlug).toBe("readers-ab12");
    const rated = { ...entry, rating: 4 };
    finish.close(rated);
    await flushPromises();
    expect(wrapper.emitted("entry")).toEqual([[rated]]);
  });

  it("only leads to the book once it is finished and rated", async () => {
    const entry = shelfItem({ id: 5, status: "finished", rating: 4, book: CIRCE });
    const wrapper = await mountHero({ club: READERS, entry });
    expect(actionOf(wrapper)).toEqual(["open"]);
    expect(wrapper.get(".hero-done .stars").text()).toBe("★★★★☆");
    expect(wrapper.get(".hero-open").attributes("href")).toBe("/book/7");
  });

  it("leads to the vote while the club has no book", async () => {
    const wrapper = await mountHero({ club: READERS }, null);
    expect(wrapper.get(".hero-title").text()).toBe("No book yet");
    expect(actionOf(wrapper)).toEqual(["vote"]);
    expect(wrapper.get(".hero-vote").attributes("href")).toBe("/c/readers-ab12/vote");
    expect(wrapper.find(".reader-rings").exists()).toBe(false);
  });

  it("drops a club the viewer is no longer in", async () => {
    api.clubPick.mockReset();
    const pinia = createPinia();
    setActivePinia(pinia);
    useSession().user = user({ clubs: [{ slug: READERS.slug, name: "Readers", role: "member", timezone: "UTC", member_count: 4 }] });
    api.clubPick.mockRejectedValue(new ApiError("No such club", 404, "No such club"));
    const wrapper = mount(HomeHero, { props: { club: READERS, entry: null }, global: { plugins: [pinia], stubs: { BookCover: true } } });
    await flushPromises();
    expect(useSession().user?.clubs).toEqual([]);
    expect(useClubs().goneSlug).toBe("readers-ab12");
    expect(wrapper.find(".error").exists()).toBe(false);
  });

  it("swaps the hero to another club and remembers it", async () => {
    const wrapper = await mountHero({ club: READERS, others: [READERS, OWLS] });
    const chips = wrapper.findAll(".hero-chip");
    expect(chips.map((chip) => chip.text())).toEqual(["Night owls"]);
    expect(wrapper.get(".hero-others").text()).toContain("Also:");
    expect(wrapper.get(".hero-all").attributes("href")).toBe("/clubs");
    await chips[0]!.trigger("click");
    expect(useClubs().lastSlug).toBe("night-owls-cd34");
  });

  it("puts the viewer's own book here without a club", async () => {
    const entry = shelfItem({ id: 5, status: "currently_reading", progress: 30, book: CIRCE });
    const wrapper = await mountHero({ club: null, entry });
    expect(api.clubPick).not.toHaveBeenCalled();
    expect(wrapper.get(".hero-title").text()).toBe("Circe");
    expect(wrapper.find(".hero-club").exists()).toBe(false);
    expect(wrapper.find(".reader-rings").exists()).toBe(false);
    expect(wrapper.find(".hero-others").exists()).toBe(false);
    expect(actionOf(wrapper)).toEqual(["progress"]);
  });

  it("holds the hero's shape while Home loads", async () => {
    const wrapper = await mountHero({ club: null, pending: true });
    expect(wrapper.find(".hero-skeleton").exists()).toBe(true);
    expect(wrapper.get(".home-hero").attributes("aria-busy")).toBe("true");
    expect(api.clubPick).not.toHaveBeenCalled();
  });
});
