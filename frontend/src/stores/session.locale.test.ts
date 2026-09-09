import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const me = vi.fn();
const login = vi.fn();
const logout = vi.fn();
const savePreferences = vi.fn();

vi.mock("../api/client", () => ({
  ApiError: class ApiError extends Error {
    status = 401;
  },
  api: {
    me: (...args: unknown[]) => me(...args),
    login: (...args: unknown[]) => login(...args),
    logout: (...args: unknown[]) => logout(...args),
    savePreferences: (...args: unknown[]) => savePreferences(...args),
    register: vi.fn(),
  },
}));

import { useLocale } from "./locale";
import { useSession } from "./session";

const ada = {
  id: 1,
  username: "ada",
  theme: "paper",
  color_mode: "system",
  locale: "en",
  avatar_url: null,
};

describe("session locale on login", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    me.mockReset();
    login.mockReset();
    logout.mockReset();
    savePreferences.mockReset();
    login.mockResolvedValue(undefined);
    logout.mockResolvedValue(undefined);
    me.mockResolvedValue(ada);
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("adopts the server locale when the login toggle was not used", async () => {
    savePreferences.mockRejectedValue(new Error("offline"));
    const session = useSession();
    await session.login("ada", "password1");
    expect(useLocale().locale).toBe("en");
    expect(savePreferences).not.toHaveBeenCalled();
  });

  it("persists an explicit auth-screen locale after login", async () => {
    savePreferences.mockRejectedValue(new Error("offline"));
    useLocale().choose("de");
    await Promise.resolve();
    expect(useLocale().explicit).toBe(true);
    savePreferences.mockReset();
    savePreferences.mockResolvedValue({ ...ada, locale: "de" });

    const session = useSession();
    await session.login("ada", "password1");
    expect(savePreferences).toHaveBeenCalledWith({ locale: "de" });
    expect(useLocale().locale).toBe("de");
  });

  it("counts a click on the already-selected locale as explicit", async () => {
    savePreferences.mockRejectedValue(new Error("offline"));
    useLocale().choose("en");
    expect(useLocale().explicit).toBe(true);
    savePreferences.mockReset();
    savePreferences.mockResolvedValue(ada);

    await useSession().login("ada", "password1");
    expect(savePreferences).toHaveBeenCalledWith({ locale: "en" });
  });
});
