import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { bookOut, clubOut, clubPick, nomination, user, vote } from "../../test/fixtures";

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

async function mountAs(role: "owner" | "admin" | "member") {
  const pinia = createPinia();
  setActivePinia(pinia);
  const club = clubOut({ role });
  useSession().user = user({ clubs: [{ ...club }] });
  useClubs().current = club;
  api.clubPick.mockResolvedValue({ pick: PICK, timezone: "Europe/Berlin" });
  api.clubPickHistory.mockResolvedValue({ items: [], timezone: "Europe/Berlin" });
  api.vote.mockResolvedValue(vote({ nominations: [nomination({ id: 2, votes: 2 })] }));
  const wrapper = mount(ClubHomePage, {
    global: {
      plugins: [pinia, i18n],
      stubs: { BookCover: true, BookDiary: true, MilestoneList: true },
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
      global: { plugins: [i18n], stubs: { BookCover: true, BookDiary: true, MilestoneList: true } },
    });
    await flushPromises();
    expect(empty.get(".empty").text()).toContain("An admin sets the next book");
    wrapper.unmount();
  });
});
