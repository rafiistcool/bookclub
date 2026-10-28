import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { user } from "../test/fixtures";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  acceptTerms: vi.fn(),
  config: vi.fn(),
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
import { TERMS_CHANGED } from "../legal";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";
import TermsPrompt from "./TermsPrompt.vue";

const LEGAL = { legal: ["imprint", "privacy", "terms"] as const, terms_version: "2026-11-01", contact_email: null };

function mountPrompt(needsTerms: boolean) {
  setActivePinia(createPinia());
  useService().$patch({ ...LEGAL, legal: [...LEGAL.legal], name: "Bookclub" });
  useSession().user = user({ needs_terms: needsTerms });
  return mount(TermsPrompt);
}

describe("TermsPrompt", () => {
  beforeEach(() => {
    for (const fn of Object.values(api)) fn.mockReset();
  });

  afterEach(() => vi.restoreAllMocks());

  it("stays away from accounts that accepted the current terms", () => {
    expect(mountPrompt(false).find("section").exists()).toBe(false);
  });

  it("asks once, links the texts and records the current version", async () => {
    api.acceptTerms.mockResolvedValue(user({ needs_terms: false }));
    const wrapper = mountPrompt(true);
    expect(wrapper.get("h2").text()).toBe("Terms of use");
    expect(wrapper.text().replace(/\s+/g, " ")).toContain(
      "The terms of use of Bookclub are new or have changed. Please read them and accept them. The privacy policy explains what we store about you.",
    );
    expect(wrapper.findAll("a").map((link) => link.attributes("href"))).toEqual(["/terms", "/privacy"]);

    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(api.acceptTerms).toHaveBeenCalledWith("2026-11-01");
    expect(useSession().user?.needs_terms).toBe(false);
    expect(wrapper.find("section").exists()).toBe(false);
  });

  it("fetches the new version when the terms changed, and accepts that one next", async () => {
    api.acceptTerms
      .mockRejectedValueOnce(new ApiError("The terms of use have changed.", 400, TERMS_CHANGED))
      .mockResolvedValueOnce(user({ needs_terms: false }));
    api.config.mockResolvedValue({ ...LEGAL, terms_version: "2026-12-01" });
    const wrapper = mountPrompt(true);

    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(wrapper.get(".error").text()).toBe("The terms of use have changed.");
    expect(useService().terms_version).toBe("2026-12-01");
    expect(useSession().user?.needs_terms).toBe(true);

    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(api.acceptTerms).toHaveBeenLastCalledWith("2026-12-01");
    expect(wrapper.find("section").exists()).toBe(false);
  });

  it("loads the version first when the page has none", async () => {
    api.config.mockResolvedValue({ ...LEGAL, terms_version: "2026-11-01" });
    api.acceptTerms.mockResolvedValue(user({ needs_terms: false }));
    const wrapper = mountPrompt(true);
    useService().terms_version = null;
    await wrapper.get("button").trigger("click");
    await flushPromises();
    expect(api.config).toHaveBeenCalledTimes(1);
    expect(api.acceptTerms).toHaveBeenCalledWith("2026-11-01");
  });
});
