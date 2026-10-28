import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { user } from "../test/fixtures";

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

const SESSIONS = {
  items: [
    { id: 1, kind: "cookie", label: "Firefox on Linux", created_at: "2026-10-01T10:00:00Z", last_used_at: "2026-10-28T08:00:00Z", current: true },
    { id: 2, kind: "bearer", label: "Phone", created_at: "2026-09-01T10:00:00Z", last_used_at: "2026-10-20T08:00:00Z", current: false },
  ],
};

function mountWith(component: object, me = user()) {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = me;
  return mount(component, { global: { plugins: [pinia, i18n] } });
}

describe("account settings", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.sessions.mockResolvedValue(SESSIONS);
    api.notifications.mockResolvedValue({ notify_meeting: true, notify_pick: true, notify_note: false });
  });

  it("shows a pending address and sends its link again", async () => {
    const wrapper = mountWith(AccountSettings, user({ email: "old@example.com", email_verified: true, pending_email: "new@example.com" }));
    await flushPromises();
    expect(wrapper.get(".email-line").text()).toContain("old@example.com");
    expect(wrapper.get(".pending-line").text()).toContain("new@example.com");
    api.resendVerification.mockResolvedValue(undefined);
    await wrapper.findAll(".account-panel .text-btn").find((b) => b.text() === "Send it again")!.trigger("click");
    expect(api.resendVerification).toHaveBeenCalled();
  });

  it("changes the address with the current password and never says whether it is taken", async () => {
    const wrapper = mountWith(AccountSettings);
    await flushPromises();
    await wrapper.findAll(".account-panel .btn").find((b) => b.text() === "Change address")!.trigger("click");
    const form = wrapper.get(".account-panel form");
    await form.get("input[type=email]").setValue("next@example.com");
    await form.get("input[type=password]").setValue("secret pw");
    api.changeEmail.mockResolvedValue(undefined);
    api.me.mockResolvedValue(user({ pending_email: "next@example.com" }));
    await form.trigger("submit");
    await flushPromises();
    expect(api.changeEmail).toHaveBeenCalledWith("next@example.com", "secret pw");
    expect(useSession().user?.pending_email).toBe("next@example.com");
  });

  it("changes the password, lists devices and signs another one out", async () => {
    const wrapper = mountWith(AccountSettings);
    await flushPromises();
    const rows = wrapper.findAll(".session-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]!.text()).toContain("This device");
    api.endSession.mockResolvedValue(undefined);
    await rows[1]!.get(".text-btn").trigger("click");
    await flushPromises();
    expect(api.endSession).toHaveBeenCalledWith(2);
    expect(wrapper.findAll(".session-row")).toHaveLength(1);

    const passwordForm = wrapper.findAll("form.account-form").at(-1)!;
    const [current, next] = passwordForm.findAll("input[type=password]");
    await current!.setValue("old pw");
    await next!.setValue("a new password");
    api.changePassword.mockResolvedValue(undefined);
    await passwordForm.trigger("submit");
    await flushPromises();
    expect(api.changePassword).toHaveBeenCalledWith("old pw", "a new password");
  });

  it("signing out this device ends the session here", async () => {
    const wrapper = mountWith(AccountSettings);
    await flushPromises();
    api.endSession.mockResolvedValue(undefined);
    await wrapper.findAll(".session-row")[0]!.get(".text-btn").trigger("click");
    await flushPromises();
    expect(useSession().user).toBeNull();
    expect(wrapper.emitted("loggedOut")).toHaveLength(1);
  });

  it("saves one notification choice at a time", async () => {
    const wrapper = mountWith(AccountSettings);
    await flushPromises();
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
    expect(mountWith(EmailPrompt).find(".email-prompt").exists()).toBe(false);
    const wrapper = mountWith(EmailPrompt, user({ email: null, email_verified: false, needs_email_prompt: true }));
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

  it("names the clubs to hand over first and deletes with the password", async () => {
    const clubs = [
      { slug: "readers-ab12", name: "Readers", role: "owner", timezone: "UTC", member_count: 3 },
      { slug: "solo-cd34", name: "Just me", role: "owner", timezone: "UTC", member_count: 1 },
      { slug: "owls-ef56", name: "Owls", role: "member", timezone: "UTC", member_count: 5 },
    ];
    const wrapper = mountWith(DeleteAccount, user({ clubs }));
    expect(wrapper.get(".owned-clubs").text()).toContain("Readers");
    expect(wrapper.get(".owned-clubs").text()).not.toContain("Just me");
    expect(wrapper.get(".owned-clubs").text()).not.toContain("Owls");
    await wrapper.get(".btn-danger").trigger("click");
    await wrapper.get("input[type=password]").setValue("my pw");
    api.deleteAccount.mockResolvedValue(undefined);
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.deleteAccount).toHaveBeenCalledWith("my pw");
    expect(useSession().user).toBeNull();
    expect(wrapper.emitted("deleted")).toHaveLength(1);
  });
});
