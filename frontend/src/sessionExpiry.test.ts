import { flushPromises } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { api, setUnauthorizedHandler } from "./api/client";
import { installSessionExpiry } from "./sessionExpiry";
import { useSession } from "./stores/session";
import { user } from "./test/fixtures";

function answer(status: number, body: unknown) {
  return vi.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    text: () => Promise.resolve(JSON.stringify(body)),
  });
}

async function setup(path: string) {
  const page = { template: "<div />" };
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/login", component: page, meta: { guest: true } },
      { path: "/shelf", component: page, meta: { auth: true } },
    ],
  });
  installSessionExpiry(router);
  await router.push(path);
  await router.isReady();
  const session = useSession();
  session.user = user();
  session.ready = true;
  return { router, session };
}

describe("an ended session", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    setUnauthorizedHandler(null);
  });

  it("forgets the user and goes to the login page, which comes back here", async () => {
    const { router, session } = await setup("/shelf?filter=finished");
    vi.stubGlobal("fetch", answer(401, { detail: "Not signed in" }));
    await expect(api.myShelf()).rejects.toMatchObject({ status: 401 });
    await flushPromises();
    expect(session.user).toBeNull();
    expect(router.currentRoute.value.path).toBe("/login");
    expect(router.currentRoute.value.query.next).toBe("/shelf?filter=finished");
  });

  it("leaves a wrong password at login alone", async () => {
    const { router, session } = await setup("/login");
    session.user = null;
    vi.stubGlobal("fetch", answer(401, { detail: "Wrong username or password" }));
    await expect(api.login({ username: "ada", password: "x" })).rejects.toMatchObject({ status: 401 });
    await flushPromises();
    expect(router.currentRoute.value.fullPath).toBe("/login");
  });

  it("does nothing more once the session is already gone", async () => {
    const { router, session } = await setup("/shelf");
    session.user = null;
    vi.stubGlobal("fetch", answer(401, { detail: "Not signed in" }));
    await expect(api.myShelf()).rejects.toMatchObject({ status: 401 });
    await flushPromises();
    expect(router.currentRoute.value.fullPath).toBe("/shelf");
  });
});
