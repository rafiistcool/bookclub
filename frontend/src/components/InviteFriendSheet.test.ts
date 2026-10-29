import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invite, user } from "../test/fixtures";
import type { ClubSummary } from "../types";

const api: Record<string, ReturnType<typeof vi.fn>> = { createInvite: vi.fn() };

vi.mock("../api/client", () => {
  class ApiError extends Error {
    status: number;
    detail: unknown;
    constructor(message: string, status: number, detail: unknown) {
      super(message);
      this.status = status;
      this.detail = detail;
    }
  }
  return { ApiError, api: new Proxy({}, { get: (_target, key: string) => api[key] }) };
});

import { ApiError } from "../api/client";
import { i18n } from "../i18n";
import { translateApiError } from "../i18n/apiErrors";
import { useSession } from "../stores/session";
import InviteFriendSheet from "./InviteFriendSheet.vue";

const club = (slug: string, name: string, role: ClubSummary["role"]): ClubSummary => ({
  slug,
  name,
  role,
  timezone: "UTC",
  member_count: 3,
});

const CLUBS = [
  club("readers-ab12", "Readers", "owner"),
  club("night-owls-cd34", "Night owls", "member"),
  club("poets-ef56", "Poets", "admin"),
];

let wrapper: VueWrapper | null = null;

function mountSheet(clubs = CLUBS) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ clubs });
  wrapper = mount(InviteFriendSheet, { props: { username: "grace" }, attachTo: document.body, global: { plugins: [pinia, i18n] } });
  return wrapper;
}

function rowFor(sheet: VueWrapper, name: string) {
  const row = sheet.findAll(".invite-club").find((element) => element.get(".invite-club-name").text() === name);
  if (!row) throw new Error(`no club ${name}`);
  return row;
}

describe("InviteFriendSheet", () => {
  beforeEach(() => {
    api.createInvite!.mockReset();
    i18n.global.locale.value = "en";
  });

  afterEach(() => {
    wrapper?.unmount();
    wrapper = null;
    document.body.innerHTML = "";
  });

  it("offers the clubs the caller owns or runs, and sends a personal invite", async () => {
    const sheet = mountSheet();
    expect(sheet.get("h2").text()).toBe("Invite grace to a club");
    expect(sheet.text()).toContain("Only grace can use it, for 30 days.");
    expect(sheet.findAll(".invite-club-name").map((name) => name.text())).toEqual(["Readers", "Poets"]);

    api.createInvite!.mockResolvedValue(invite({ personal: true, invitee: "grace" }));
    await rowFor(sheet, "Readers").get("button").trigger("click");
    await flushPromises();
    expect(api.createInvite).toHaveBeenCalledWith("readers-ab12", { kind: "friend", username: "grace" });
    expect(rowFor(sheet, "Readers").get(".invite-sent").text()).toBe("Invitation sent");
    expect(rowFor(sheet, "Readers").find("button").exists()).toBe(false);
    // The other club can still be asked.
    expect(rowFor(sheet, "Poets").get("button").text()).toBe("Invite");
  });

  it("says why an invitation failed, by the club it was for", async () => {
    const sheet = mountSheet();
    api.createInvite!.mockRejectedValue(
      new ApiError(translateApiError("That friend is already in this club"), 409, "That friend is already in this club"),
    );
    await rowFor(sheet, "Poets").get("button").trigger("click");
    await flushPromises();
    expect(rowFor(sheet, "Poets").get(".error").text()).toBe("That friend is already in this club");
    expect(rowFor(sheet, "Readers").find(".error").exists()).toBe(false);

    // The client translates the detail in the reader's language.
    i18n.global.locale.value = "de";
    api.createInvite!.mockRejectedValue(
      new ApiError(translateApiError("No friend with that name"), 404, "No friend with that name"),
    );
    await rowFor(sheet, "Readers").get("button").trigger("click");
    await flushPromises();
    expect(rowFor(sheet, "Readers").get(".error").text()).toBe("Keiner deiner Freunde hat diesen Namen");
  });

  it("explains that only a club's admins invite, and closes", async () => {
    const sheet = mountSheet([club("night-owls-cd34", "Night owls", "member")]);
    expect(sheet.find(".invite-club").exists()).toBe(false);
    expect(sheet.text()).toContain("You do not run a club. Only a club's admins can invite people into it.");
    await sheet.findAll("button").find((element) => element.text() === "Done")!.trigger("click");
    expect(sheet.emitted("close")).toHaveLength(1);
  });
});
