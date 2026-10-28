import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { clubOut, invite, member, user } from "../../test/fixtures";
import { memoryStorage } from "../../test/storage";

const api = {
  clubMembers: vi.fn(),
  clubInvites: vi.fn(),
  createInvite: vi.fn(),
  revokeInvite: vi.fn(),
  changeMemberRole: vi.fn(),
  removeMember: vi.fn(),
  transferClub: vi.fn(),
  leaveClub: vi.fn(),
  deleteClub: vi.fn(),
  updateClub: vi.fn(),
  clubs: vi.fn(),
};

vi.mock("../../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => (...args: unknown[]) => (api as Record<string, (...a: unknown[]) => unknown>)[key]!(...args) }),
}));

import { i18n } from "../../i18n";
import { useClubs } from "../../stores/clubs";
import { useSession } from "../../stores/session";
import ClubMembersPage from "./ClubMembersPage.vue";

const ROSTER = [
  member({ username: "olga", role: "owner" }),
  member({ username: "adam", role: "admin" }),
  member({ username: "mia", role: "member" }),
  member({ username: "max", role: "member" }),
];

async function mountAs(role: "owner" | "admin" | "member", me: string) {
  const pinia = createPinia();
  setActivePinia(pinia);
  const club = clubOut({ role, member_count: 4 });
  useSession().user = user({ username: me, clubs: [{ ...club }] });
  useClubs().current = club;
  api.clubMembers.mockResolvedValue({
    items: ROSTER.map((row) => ({ ...row, you: row.username === me })),
  });
  api.clubInvites.mockResolvedValue({
    items: [invite({ id: 1, code: "AAAA1111AA" }), invite({ id: 2, code: "BBBB2222BB", active: false, revoked: true })],
  });
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/c/:slug/members", component: ClubMembersPage },
      { path: "/:rest(.*)*", component: { template: "<p>elsewhere</p>" } },
    ],
  });
  await router.push(`/c/${club.slug}/members`);
  const wrapper = mount(ClubMembersPage, {
    global: { plugins: [pinia, router, i18n], stubs: { BookCover: true } },
  });
  await flushPromises();
  return { wrapper, router };
}

function manageButtonsFor(wrapper: Awaited<ReturnType<typeof mountAs>>["wrapper"]): string[] {
  return wrapper
    .findAll(".member-card")
    .filter((row) => row.find(".manage-btn").exists())
    .map((row) => row.get(".member-meta strong").text());
}

describe("ClubMembersPage", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    vi.stubGlobal("localStorage", memoryStorage());
  });

  afterEach(() => vi.unstubAllGlobals());

  it("lists everyone, the viewer included, and hides admin actions from members", async () => {
    const { wrapper } = await mountAs("member", "mia");
    expect(wrapper.findAll(".member-card")).toHaveLength(4);
    expect(wrapper.get(".is-you").text()).toContain("mia");
    expect(manageButtonsFor(wrapper)).toEqual([]);
    expect(wrapper.find(".invites").exists()).toBe(false);
    expect(api.clubInvites).not.toHaveBeenCalled();
    expect(wrapper.text()).not.toContain("Club settings");
    expect(wrapper.text()).toContain("Leave the club");
    expect(wrapper.text()).not.toContain("Delete the club");
  });

  it("lets admins remove members only, and manage invites", async () => {
    const { wrapper } = await mountAs("admin", "adam");
    expect(manageButtonsFor(wrapper)).toEqual(["mia", "max"]);
    expect(wrapper.findAll(".invite-row:not(.past)").map((row) => row.get("code").text())).toEqual(["AAAA1111AA"]);

    api.createInvite.mockResolvedValue(invite({ id: 3, code: "CCCC3333CC", max_uses: null }));
    const link = wrapper.findAll(".invites .btn").find((button) => button.text() === "New invite link")!;
    await link.trigger("click");
    await flushPromises();
    expect(api.createInvite).toHaveBeenCalledWith("readers-ab12", { kind: "link" });
    expect(wrapper.get(".fresh-invite").text()).toContain("http://localhost/join/CCCC3333CC");

    api.revokeInvite.mockResolvedValue(undefined);
    const revoke = wrapper.findAll(".invite-row:not(.past) .text-btn").find((button) => button.text() === "Withdraw")!;
    await revoke.trigger("click");
    await flushPromises();
    expect(api.revokeInvite).toHaveBeenCalledWith("readers-ab12", 3);

    // An admin removes a member after a second, confirming tap.
    api.removeMember.mockResolvedValue(undefined);
    const mia = wrapper.findAll(".member-card").find((row) => row.text().includes("mia"))!;
    await mia.get(".manage-btn").trigger("click");
    const sheet = wrapper.get(".sheet");
    expect(sheet.text()).not.toContain("Make admin");
    expect(sheet.text()).not.toContain("Hand over the club");
    await sheet.get(".btn-danger").trigger("click");
    expect(api.removeMember).not.toHaveBeenCalled();
    await wrapper.get(".sheet .btn-danger").trigger("click");
    await flushPromises();
    expect(api.removeMember).toHaveBeenCalledWith("readers-ab12", "mia");
  });

  it("gives the owner roles, the hand-over and the delete", async () => {
    const { wrapper, router } = await mountAs("owner", "olga");
    expect(manageButtonsFor(wrapper)).toEqual(["adam", "mia", "max"]);
    expect(wrapper.text()).not.toContain("Leave the club");

    const adam = wrapper.findAll(".member-card").find((row) => row.text().includes("adam"))!;
    await adam.get(".manage-btn").trigger("click");
    expect(wrapper.get(".sheet").text()).toContain("Make a regular member");
    api.transferClub.mockResolvedValue(clubOut({ role: "admin" }));
    const handOver = () => wrapper.findAll(".sheet .btn").find((button) => /Hand over|hand it over/.test(button.text()))!;
    await handOver().trigger("click");
    await handOver().trigger("click");
    await flushPromises();
    expect(api.transferClub).toHaveBeenCalledWith("readers-ab12", "adam");
    expect(useClubs().current?.role).toBe("admin");

    // Back to owner for the delete.
    useClubs().current = clubOut({ role: "owner" });
    await flushPromises();
    await wrapper.findAll(".danger-zone .btn-danger").find((button) => button.text() === "Delete the club")!.trigger("click");
    const confirm = wrapper.get(".delete-form button[type=submit]");
    expect(confirm.attributes("disabled")).toBeDefined();
    await wrapper.get(".delete-form input").setValue("Readers");
    api.deleteClub.mockResolvedValue(undefined);
    api.clubs.mockResolvedValue({ items: [] });
    await wrapper.get(".delete-form").trigger("submit");
    await flushPromises();
    expect(api.deleteClub).toHaveBeenCalledWith("readers-ab12");
    expect(router.currentRoute.value.path).toBe("/clubs");
    expect(useSession().user?.clubs).toEqual([]);
  });
});
