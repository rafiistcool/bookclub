import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { searchHit, user } from "../test/fixtures";
import { memoryStorage } from "../test/storage";

const api: Record<string, ReturnType<typeof vi.fn>> = {
  register: vi.fn(),
  login: vi.fn(),
  logout: vi.fn(),
  me: vi.fn(),
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
import { readBrowseSnapshot, writeBrowseSnapshot } from "../pages/discoverCache";
import { useSession } from "./session";

const SIGNUP = { username: "neu", email: "neu@example.com", password: "long enough", inviteCode: "" };

function zone(name: string) {
  const original = Intl.DateTimeFormat.prototype.resolvedOptions;
  vi.spyOn(Intl.DateTimeFormat.prototype, "resolvedOptions").mockImplementation(function (
    this: Intl.DateTimeFormat,
  ) {
    return { ...original.call(this), timeZone: name };
  });
}

/** What Discover keeps between visits: the hits carry the viewer's own badges. */
function browseAsPreviousReader() {
  writeBrowseSnapshot({
    trending: [searchHit({ id: 3, on_shelf: true, club_picks: [{ slug: "readers-ab12", name: "Readers" }] })],
    releases: null,
    rows: [],
    browseError: "",
  });
  expect(readBrowseSnapshot()).not.toBeNull();
}

describe("session", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    for (const fn of Object.values(api)) fn.mockReset();
    api.savePreferences.mockResolvedValue(user());
    vi.stubGlobal("localStorage", memoryStorage());
    vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("signs up in UTC when the server does not know the browser's zone", async () => {
    zone("Etc/Unknown");
    const refused = new ApiError("That is not a valid time zone", 400, "That is not a valid time zone");
    api.register.mockRejectedValueOnce(refused).mockResolvedValueOnce(user({ username: "neu", timezone: "UTC" }));
    await useSession().register(SIGNUP);
    expect(api.register).toHaveBeenCalledTimes(2);
    expect(api.register.mock.calls[0]![0]).toMatchObject({ timezone: "Etc/Unknown" });
    expect(api.register.mock.calls[1]![0]).toMatchObject({ username: "neu", timezone: null });
    expect(useSession().user?.username).toBe("neu");
  });

  it("retries without the zone when it was one of several messages, and then shows the rest", async () => {
    zone("Etc/Unknown");
    const both = "Password must be at least 8 characters; That is not a valid time zone";
    const short = "Password must be at least 8 characters";
    api.register
      .mockRejectedValueOnce(new ApiError(both, 400, both))
      .mockRejectedValueOnce(new ApiError(short, 400, short));
    await expect(useSession().register({ ...SIGNUP, password: "short" })).rejects.toMatchObject({ detail: short });
    expect(api.register).toHaveBeenCalledTimes(2);
  });

  it("does not retry other refusals", async () => {
    zone("Europe/Berlin");
    const taken = new ApiError("That username is already taken", 400, "That username is already taken");
    api.register.mockRejectedValue(taken);
    await expect(useSession().register(SIGNUP)).rejects.toBe(taken);
    expect(api.register).toHaveBeenCalledTimes(1);
  });

  it("forgets unsent notes and the last club when the reader leaves this device, not when the session expires", async () => {
    // Security review 2026-10-28 #12: a shared browser must not keep a deleted or signed-out
    // account's notes.
    const session = useSession();
    session.user = user();
    const note = "bookclub.draft.ada.7";
    localStorage.setItem(note, "Only for me");
    localStorage.setItem("bookclub.club", "readers-ab12");
    session.reset();
    expect(localStorage.getItem(note)).toBe("Only for me");
    expect(localStorage.getItem("bookclub.club")).toBe("readers-ab12");

    session.user = user();
    api.logout.mockResolvedValue(undefined);
    await session.logout();
    expect(localStorage.getItem(note)).toBeNull();
    expect(localStorage.getItem("bookclub.club")).toBeNull();

    localStorage.setItem(note, "Again");
    session.user = user();
    session.reset({ leavingDevice: true });
    expect(localStorage.getItem(note)).toBeNull();
  });

  it("forgets the previous account's Discover snapshot on logout, login and sign-up", async () => {
    const session = useSession();
    session.user = user();
    browseAsPreviousReader();
    api.logout.mockResolvedValue(undefined);
    await session.logout();
    expect(readBrowseSnapshot()).toBeNull();

    browseAsPreviousReader();
    session.reset();
    expect(readBrowseSnapshot()).toBeNull();

    browseAsPreviousReader();
    api.login.mockResolvedValue(undefined);
    api.me.mockResolvedValue(user({ username: "grace" }));
    await session.login("grace", "long enough");
    expect(readBrowseSnapshot()).toBeNull();

    browseAsPreviousReader();
    api.register.mockResolvedValue(user({ username: "neu" }));
    await session.register(SIGNUP);
    expect(readBrowseSnapshot()).toBeNull();
  });
});
