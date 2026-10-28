import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  register: vi.fn(),
  verifyEmail: vi.fn(),
  me: vi.fn(),
  requestPasswordReset: vi.fn(),
  confirmPasswordReset: vi.fn(),
  savePreferences: vi.fn(),
};

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
import { useService } from "../stores/service";
import { useSession } from "../stores/session";
import ForgotPasswordPage from "./ForgotPasswordPage.vue";
import ResetPasswordPage from "./ResetPasswordPage.vue";
import SignupPage from "./SignupPage.vue";
import VerifyEmailPage from "./VerifyEmailPage.vue";

async function mountAt(component: object, path: string, signup = "invite") {
  const pinia = createPinia();
  setActivePinia(pinia);
  useService().signup = signup;
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/signup", component: SignupPage },
      { path: "/verify-email", component: VerifyEmailPage },
      { path: "/forgot-password", component: ForgotPasswordPage },
      { path: "/reset-password", component: ResetPasswordPage },
      { path: "/:rest(.*)*", component: { template: "<p>elsewhere</p>" } },
    ],
  });
  await router.push(path);
  const wrapper = mount(component, { global: { plugins: [pinia, router, i18n] } });
  await flushPromises();
  return { wrapper, router };
}

describe("auth pages", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
    api.savePreferences.mockResolvedValue(user());
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("sessionStorage", memoryStorage());
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  });

  afterEach(() => vi.unstubAllGlobals());

  it("signs up with the invite code from the link and opens the club it joined", async () => {
    const club = { slug: "readers-ab12", name: "Readers", role: "member", timezone: "UTC", member_count: 4 };
    api.register.mockResolvedValue(user({ username: "neu", clubs: [club] }));
    const { wrapper, router } = await mountAt(SignupPage, "/signup?code=7K3M9QX2HD");
    const invite = wrapper.get("input[name=invite]");
    expect((invite.element as HTMLInputElement).value).toBe("7K3M9QX2HD");
    expect(invite.attributes("required")).toBeDefined();
    await wrapper.get("input[name=username]").setValue("neu");
    await wrapper.get("input[name=email]").setValue(" neu@example.com ");
    await wrapper.get("input[name=password]").setValue("long enough");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.register).toHaveBeenCalledWith(
      expect.objectContaining({ username: "neu", email: "neu@example.com", invite_code: "7K3M9QX2HD" }),
    );
    expect(router.currentRoute.value.path).toBe("/c/readers-ab12");
  });

  it("makes the invite optional when sign-up is open", async () => {
    api.register.mockResolvedValue(user({ clubs: [] }));
    const { wrapper, router } = await mountAt(SignupPage, "/signup", "open");
    expect(wrapper.find("input[name=invite]").exists()).toBe(false);
    await wrapper.get(".invite-toggle").trigger("click");
    expect(wrapper.get("input[name=invite]").attributes("required")).toBeUndefined();
    await wrapper.get("input[name=username]").setValue("solo");
    await wrapper.get("input[name=email]").setValue("solo@example.com");
    await wrapper.get("input[name=password]").setValue("long enough");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.register).toHaveBeenCalledWith(expect.objectContaining({ invite_code: null }));
    expect(router.currentRoute.value.path).toBe("/");
  });

  it("confirms an address from the link's fragment and clears it from the address bar", async () => {
    api.verifyEmail.mockResolvedValue(undefined);
    const { wrapper, router } = await mountAt(VerifyEmailPage, "/verify-email#token=tok_123");
    expect(api.verifyEmail).toHaveBeenCalledWith("tok_123");
    expect(router.currentRoute.value.hash).toBe("");
    expect(wrapper.text()).toContain("Address confirmed");
  });

  it("explains an expired verification link", async () => {
    api.verifyEmail.mockRejectedValue(
      new ApiError("That link has expired or was already used", 400, "That link has expired or was already used"),
    );
    const { wrapper } = await mountAt(VerifyEmailPage, "/verify-email#token=old");
    expect(wrapper.get(".error").text()).toBe("That link has expired or was already used");
  });

  it("answers a reset request the same way for any address", async () => {
    api.requestPasswordReset.mockResolvedValue(undefined);
    const { wrapper } = await mountAt(ForgotPasswordPage, "/forgot-password");
    await wrapper.get("input[name=email]").setValue("who@example.com");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.requestPasswordReset).toHaveBeenCalledWith("who@example.com");
    expect(wrapper.text()).toContain("If an account has confirmed who@example.com");
  });

  it("sets a new password from the reset link and forgets this tab's session", async () => {
    api.confirmPasswordReset.mockResolvedValue(undefined);
    const { wrapper, router } = await mountAt(ResetPasswordPage, "/reset-password#token=reset_1");
    useSession().user = user();
    expect(router.currentRoute.value.hash).toBe("");
    await wrapper.get("input[name=new-password]").setValue("brand new pw");
    await wrapper.get("form").trigger("submit");
    await flushPromises();
    expect(api.confirmPasswordReset).toHaveBeenCalledWith("reset_1", "brand new pw");
    expect(useSession().user).toBeNull();
    expect(wrapper.text()).toContain("Password changed");
  });

  it("offers a new link when the reset link has no token", async () => {
    const { wrapper } = await mountAt(ResetPasswordPage, "/reset-password");
    expect(wrapper.find("form").exists()).toBe(false);
    expect(wrapper.get(".error").text()).toBe("That link has expired or was already used");
  });
});
