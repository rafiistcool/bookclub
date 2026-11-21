import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AdminRequestSignalOut, AdminUserOut } from "../../api/generated";
import { user } from "../../test/fixtures";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  adminSuspendedUsers: vi.fn(),
  adminFriendRequestSignals: vi.fn(),
  adminSuspendUser: vi.fn(),
  adminUnsuspendUser: vi.fn(),
};

vi.mock("../../api/client", () => {
  class ApiError extends Error {
    status: number;
    detail: unknown;
    constructor(message: string, status: number, detail: unknown = null) {
      super(message);
      this.status = status;
      this.detail = detail;
    }
  }
  return { ApiError, api: new Proxy({}, { get: (_target, key: string) => api[key] }) };
});

import { ApiError } from "../../api/client";
import { i18n } from "../../i18n";
import { useLocale } from "../../stores/locale";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";
import AdminUsers from "./AdminUsers.vue";

function account(username: string, overrides: Partial<AdminUserOut> = {}): AdminUserOut {
  return {
    id: username.length * 100 + username.charCodeAt(0),
    username,
    avatar_url: null,
    is_admin: false,
    created_at: "2026-11-01T08:00:00Z",
    suspended_at: null,
    ...overrides,
  };
}

function signal(username: string, pending: number, blocks: number, overrides: Partial<AdminUserOut> = {}): AdminRequestSignalOut {
  return { user: account(username, overrides), pending_requests: pending, blocks_received: blocks };
}

async function mountUsers() {
  const pinia = createPinia();
  setActivePinia(pinia);
  useSession().user = user({ username: "root", is_admin: true, timezone: "UTC" });
  useLocale().locale = i18n.global.locale.value;
  const wrapper = mount(AdminUsers, { attachTo: document.body, global: { plugins: [pinia] } });
  await flushPromises();
  return wrapper;
}

function buttonIn(scope: Pick<VueWrapper, "findAll">, label: string) {
  const found = scope.findAll("button").find((node) => node.text() === label);
  if (!found) throw new Error(`no button "${label}"`);
  return found;
}

async function showRequests(wrapper: VueWrapper) {
  await buttonIn(wrapper.get(".user-views"), "Friend requests").trigger("click");
  await flushPromises();
}

describe("AdminUsers", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.adminSuspendedUsers.mockResolvedValue({ items: [] });
    api.adminFriendRequestSignals.mockResolvedValue({ items: [] });
    i18n.global.locale.value = "en";
    document.body.innerHTML = "";
  });

  it("lists the suspended accounts first, and the friend requests only when asked", async () => {
    api.adminSuspendedUsers.mockResolvedValue({ items: [account("mallory", { suspended_at: "2026-11-20T08:00:00Z" })] });
    const wrapper = await mountUsers();
    expect(wrapper.get(".user-views button[aria-pressed=true]").text()).toBe("Suspended");
    expect(wrapper.get(".user-list").text()).toContain("mallory");
    expect(api.adminFriendRequestSignals).not.toHaveBeenCalled();

    api.adminFriendRequestSignals.mockResolvedValue({
      items: [signal("spammer", 48, 6), signal("dee", 0, 2, { suspended_at: "2026-11-20T08:00:00Z" })],
    });
    await showRequests(wrapper);
    expect(api.adminFriendRequestSignals).toHaveBeenCalledTimes(1);
    expect(wrapper.get(".user-views button[aria-pressed=true]").text()).toBe("Friend requests");
    const rows = wrapper.findAll(".signal-row");
    expect(rows.map((row) => row.get("strong").text())).toEqual(["spammer", "dee"]);
    expect(rows[0]!.text()).toContain("Waiting: 48 · Blocked by (30 days): 6");
    expect(rows[0]!.find("button").text()).toBe("Suspend…");
    // A suspended account is marked, without the button.
    expect(rows[1]!.get(".badge").text()).toBe("Suspended");
    expect(rows[1]!.find("button").exists()).toBe(false);
    expect(wrapper.find(".user-list:not(.signal-list)").exists()).toBe(false);
  });

  it("says when no account has friend requests waiting or blocks, and when the list cannot be loaded", async () => {
    const wrapper = await mountUsers();
    await showRequests(wrapper);
    expect(wrapper.text()).toContain("No account has friend requests waiting or was blocked in the last 30 days.");

    await buttonIn(wrapper.get(".user-views"), "Suspended").trigger("click");
    api.adminFriendRequestSignals.mockRejectedValue(new ApiError("Not found", 404, "Not found"));
    await showRequests(wrapper);
    expect(wrapper.get(".error").text()).toContain("Not found");
    api.adminFriendRequestSignals.mockResolvedValue({ items: [signal("spammer", 50, 0)] });
    await buttonIn(wrapper.get(".error"), "Try again").trigger("click");
    await flushPromises();
    expect(wrapper.findAll(".signal-row")).toHaveLength(1);
  });

  it("suspends from the list through the form's second tap, and marks the row", async () => {
    api.adminFriendRequestSignals.mockResolvedValue({ items: [signal("spammer", 48, 6)] });
    const wrapper = await mountUsers();
    await showRequests(wrapper);
    await buttonIn(wrapper.get(".signal-row"), "Suspend…").trigger("click");
    await flushPromises();
    // The name is in the form, which asks for the confirming tap.
    expect((wrapper.get("input[name=suspend-username]").element as HTMLInputElement).value).toBe("spammer");
    expect(wrapper.get(".suspend-form").text()).toContain("spammer is signed out everywhere");
    const confirm = buttonIn(wrapper.get(".suspend-form"), "Suspend spammer");
    expect(document.activeElement).toBe(confirm.element);
    expect(api.adminSuspendUser).not.toHaveBeenCalled();

    api.adminSuspendUser.mockResolvedValue(account("spammer", { suspended_at: "2026-11-21T08:00:00Z" }));
    await wrapper.get(".suspend-form").trigger("submit");
    await flushPromises();
    expect(api.adminSuspendUser).toHaveBeenCalledWith("spammer");
    expect(useToast().items.map((toast) => toast.message)).toContain("spammer is suspended");
    expect(wrapper.get(".signal-row .badge").text()).toBe("Suspended");
  });
});
