import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { user } from "../test/fixtures";
import type { ClubSummary } from "../types";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  me: vi.fn(),
  changeEmail: vi.fn(),
  resendVerification: vi.fn(),
  changePassword: vi.fn(),
  sessions: vi.fn(),
  endSession: vi.fn(),
  notifications: vi.fn(),
  saveNotifications: vi.fn(),
  dismissEmailPrompt: vi.fn(),
  deleteAccount: vi.fn(),
};

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: new Proxy({}, { get: (_target, key: string) => api[key] }),
}));

import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import AccountSettings from "./AccountSettings.vue";
import DeleteAccount from "./DeleteAccount.vue";
import EmailPrompt from "./EmailPrompt.vue";
import NotificationSwitches from "./settings/NotificationSwitches.vue";
import SessionList from "./settings/SessionList.vue";

const SESSIONS = {
  items: [
    { id: 1, kind: "cookie", label: "Firefox on Linux", created_at: "2026-10-01T10:00:00Z", last_used_at: "2026-10-28T08:00:00Z", current: true },
    { id: 2, kind: "bearer", label: "Phone", created_at: "2026-09-01T10:00:00Z", last_used_at: "2026-10-20T08:00:00Z", current: false },
  ],
};

async function mountWith(component: object, me = user(), path = "/settings/account") {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = me;
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/:rest(.*)*", component: { template: "<p />" } }],
  });
  await router.push(path);
  const wrapper = mount(component, { attachTo: document.body, global: { plugins: [pinia, i18n, router] } });
  await flushPromises();
  return { wrapper, router };
}

describe("the account page", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    i18n.global.locale.value = "en";
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("shows a pending address and sends its link again", async () => {
    const { wrapper } = await mountWith(AccountSettings, user({ email: "old@example.com", email_verified: true, pending_email: "new@example.com" }));
    expect(wrapper.get(".email-row .row-detail").text()).toBe("old@example.com");
    expect(wrapper.find(".email-row .row-alert").exists()).toBe(true);
    expect(wrapper.get(".pending-line").text()).toContain("new@example.com");
    api.resendVerification.mockResolvedValue(undefined);
    await wrapper.get(".email-status .text-btn").trigger("click");
    expect(api.resendVerification).toHaveBeenCalled();
  });

  it("says nothing more about a confirmed address", async () => {
    const { wrapper } = await mountWith(AccountSettings);
    expect(wrapper.find(".email-status").exists()).toBe(false);
    expect(wrapper.find(".email-row .row-alert").exists()).toBe(false);
  });

  it("changes the address in a sheet, with the current password, never saying whether it is taken", async () => {
    const { wrapper } = await mountWith(AccountSettings);
    await wrapper.get(".email-row").trigger("click");
    const form = wrapper.get(".email-sheet form");
    await form.get("input[type=email]").setValue("next@example.com");
    await form.get("input[type=password]").setValue("secret pw");
    api.changeEmail.mockResolvedValue(undefined);
    api.me.mockResolvedValue(user({ pending_email: "next@example.com" }));
    await form.trigger("submit");
    await flushPromises();
    expect(api.changeEmail).toHaveBeenCalledWith("next@example.com", "secret pw");
    expect(useSession().user?.pending_email).toBe("next@example.com");
    expect(wrapper.find(".email-sheet").exists()).toBe(false);
  });

  it("changes the password in a sheet and drops the pending address that voided", async () => {
    const { wrapper } = await mountWith(AccountSettings, user({ email: "old@example.com", email_verified: true, pending_email: "new@example.com" }));
    expect(wrapper.find(".pending-line").exists()).toBe(true);
    expect(wrapper.find(".password-sheet").exists()).toBe(false);
    await wrapper.get(".password-row").trigger("click");
    const [current, next] = wrapper.get(".password-sheet form").findAll("input[type=password]");
    await current!.setValue("old pw");
    await next!.setValue("a new password");
    api.changePassword.mockResolvedValue(undefined);
    api.me.mockResolvedValue(user({ email: "old@example.com", email_verified: true, pending_email: null }));
    await wrapper.get(".password-sheet form").trigger("submit");
    await flushPromises();
    expect(api.changePassword).toHaveBeenCalledWith("old pw", "a new password");
    expect(wrapper.find(".password-sheet").exists()).toBe(false);
    expect(useSession().user?.pending_email).toBeNull();
    expect(wrapper.find(".pending-line").exists()).toBe(false);
    expect(wrapper.find(".email-status").exists()).toBe(false);
  });

  it("keeps the password sheet open with the server's reason when the change fails", async () => {
    const { wrapper } = await mountWith(AccountSettings);
    await wrapper.get(".password-row").trigger("click");
    const [current, next] = wrapper.get(".password-sheet form").findAll("input[type=password]");
    await current!.setValue("wrong");
    await next!.setValue("a new password");
    api.changePassword.mockRejectedValue(new Error("offline"));
    await wrapper.get(".password-sheet form").trigger("submit");
    await flushPromises();
    expect(wrapper.get(".password-sheet .error").text()).toBe("Could not change the password");
  });

  it("leads to the devices, downloads the export, and keeps deleting at the bottom, in red", async () => {
    const { wrapper } = await mountWith(AccountSettings);
    expect(wrapper.find('a[href="/settings/devices"]').exists()).toBe(true);
    const exportRow = wrapper.get(".export-row");
    expect(exportRow.attributes("href")).toBe("/api/auth/me/export");
    expect(exportRow.attributes("download")).toBe("");
    const rows = wrapper.findAll(".settings-row");
    expect(rows.at(-1)!.classes()).toEqual(expect.arrayContaining(["delete-row", "danger"]));
    await rows.at(-1)!.trigger("click");
    expect(wrapper.find(".delete-account").exists()).toBe(true);
  });

  it("opens the sheet a link asks for", async () => {
    const { wrapper, router } = await mountWith(AccountSettings, user(), "/settings/account?sheet=password");
    expect(wrapper.find(".password-sheet").exists()).toBe(true);
    expect(router.currentRoute.value.fullPath).toBe("/settings/account");
  });
});

describe("the signed-in devices", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.sessions.mockResolvedValue(SESSIONS);
    i18n.global.locale.value = "en";
  });

  it("lists the devices and signs another one out", async () => {
    const { wrapper } = await mountWith(SessionList);
    const rows = wrapper.findAll(".session-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]!.text()).toContain("This device");
    expect(rows[1]!.get(".session-label").text()).toBe("Phone");
    api.endSession.mockResolvedValue(undefined);
    await rows[1]!.get(".session-end").trigger("click");
    await flushPromises();
    expect(api.endSession).toHaveBeenCalledWith(2);
    expect(wrapper.findAll(".session-row")).toHaveLength(1);
  });

  it("signing out this device ends the session here", async () => {
    const { wrapper } = await mountWith(SessionList);
    api.endSession.mockResolvedValue(undefined);
    await wrapper.findAll(".session-row")[0]!.get(".session-end").trigger("click");
    await flushPromises();
    expect(useSession().user).toBeNull();
    expect(wrapper.emitted("loggedOut")).toHaveLength(1);
  });

  it("says so when the list does not load", async () => {
    api.sessions.mockRejectedValue(new Error("offline"));
    const { wrapper } = await mountWith(SessionList);
    expect(wrapper.get(".error").text()).toBe("Could not load the devices");
  });
});

describe("the notification switches, kept until something sends notifications", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.notifications.mockResolvedValue({ notify_meeting: true, notify_pick: true, notify_note: false });
  });

  it("saves one choice at a time", async () => {
    const { wrapper } = await mountWith(NotificationSwitches);
    api.saveNotifications.mockResolvedValue({ notify_meeting: true, notify_pick: true, notify_note: true });
    await wrapper.findAll(".notification-row input")[2]!.setValue(true);
    await flushPromises();
    expect(api.saveNotifications).toHaveBeenCalledWith({ notify_note: true });
  });
});

describe("the one-time email prompt", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
  });

  it("only shows for accounts without an address, and can be dismissed for good", async () => {
    expect((await mountWith(EmailPrompt)).wrapper.find(".email-prompt").exists()).toBe(false);
    const { wrapper } = await mountWith(EmailPrompt, user({ email: null, email_verified: false, needs_email_prompt: true }));
    expect(wrapper.find(".email-prompt").exists()).toBe(true);
    api.dismissEmailPrompt.mockResolvedValue(undefined);
    await wrapper.findAll(".btn").find((b) => b.text() === "Not now")!.trigger("click");
    await flushPromises();
    expect(api.dismissEmailPrompt).toHaveBeenCalled();
    expect(wrapper.find(".email-prompt").exists()).toBe(false);
  });
});

describe("deleting the account", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("names the clubs to hand over first and deletes with the password", async () => {
    const clubs: ClubSummary[] = [
      { slug: "readers-ab12", name: "Readers", role: "owner", timezone: "UTC", member_count: 3 },
      { slug: "solo-cd34", name: "Just me", role: "owner", timezone: "UTC", member_count: 1 },
      { slug: "owls-ef56", name: "Owls", role: "member", timezone: "UTC", member_count: 5 },
    ];
    const { wrapper } = await mountWith(DeleteAccount, user({ clubs }));
    expect(wrapper.get(".owned-clubs").text()).toContain("Readers");
    expect(wrapper.get(".owned-clubs").text()).not.toContain("Just me");
    expect(wrapper.get(".owned-clubs").text()).not.toContain("Owls");
    // Nothing goes before the password is typed.
    expect(wrapper.get(".btn-danger").attributes("disabled")).toBeDefined();
    await wrapper.get("input[type=password]").setValue("my pw");
    api.deleteAccount.mockResolvedValue(undefined);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.deleteAccount).toHaveBeenCalledWith("my pw");
    expect(useSession().user).toBeNull();
    expect(wrapper.emitted("deleted")).toHaveLength(1);
  });
});
