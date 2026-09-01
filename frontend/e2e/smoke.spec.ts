import { expect, test, type Page } from "@playwright/test";

const SEARCH_FIXTURE = {
  page: 1,
  has_more: false,
  items: [
    { ol_work_key: "/works/OL1W", title: "Circe", authors: "Madeline Miller", cover_id: null, year: 2018, on_shelf: null, shelf_id: null, club_pick: false },
    { ol_work_key: "/works/OL2W", title: "Dune", authors: "Frank Herbert", cover_id: null, year: 1965, on_shelf: null, shelf_id: null, club_pick: false },
    { ol_work_key: "/works/OL3W", title: "Piranesi", authors: "Susanna Clarke", cover_id: null, year: 2020, on_shelf: null, shelf_id: null, club_pick: false },
  ],
};

async function mockOpenLibrary(page: Page) {
  // Search and work details go to Open Library server-side; the smoke run must
  // not depend on the network, so intercept the app's own API calls.
  await page.route("**/api/books/search**", (route) =>
    route.fulfill({ json: SEARCH_FIXTURE }),
  );
  await page.route("**/api/books/work/**", async (route) => {
    const id = route.request().url().split("/").pop() ?? "";
    const hit = SEARCH_FIXTURE.items.find((row) => row.ol_work_key.endsWith(id)) ?? SEARCH_FIXTURE.items[0];
    await route.fulfill({
      json: {
        ...hit,
        cover_url: null,
        description: "A witch on an island, exiled by the gods.",
        pages: 393,
        subjects: ["Greek mythology", "Fantasy"],
        ol_rating: 4.3,
        ol_rating_count: 812,
        on_shelf: null,
        shelf_id: null,
        club_pick: false,
        members: [],
        quote_count: 0,
      },
    });
  });
}

const user = `smoke${Date.now().toString(36)}`;

test.describe.configure({ mode: "serial" });

test("register with the bootstrap invite lands on an empty home", async ({ page }) => {
  await page.goto("/register");
  await page.getByLabel("Invite code").fill("E2E-INVITE");
  await page.getByLabel("Username").fill(user);
  await page.getByLabel("Password", { exact: false }).first().fill("password123");
  await page.getByRole("button", { name: "Create account" }).click();

  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole("heading", { level: 1, name: "Home" })).toBeVisible();
  await expect(page.getByText("No club pick yet")).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Main" }).getByRole("link", { name: "Library" })).toBeVisible();

  // Phone layout: nothing wider than the viewport.
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});

test("library shows search first, opens a detail sheet, adds with undo", async ({ page }) => {
  await login(page);
  await mockOpenLibrary(page);
  await page.goto("/library");

  const search = page.getByRole("searchbox", { name: "Search books" });
  await expect(search).toBeVisible();
  const box = await search.boundingBox();
  expect(box!.y).toBeLessThan(140);

  await page.getByRole("button", { name: "Circe" }).click();
  const sheet = page.getByRole("dialog", { name: "Circe" });
  await expect(sheet).toBeVisible();
  await expect(sheet.getByText("393 pages")).toBeVisible();
  await expect(sheet.getByText("A witch on an island")).toBeVisible();

  await sheet.getByRole("button", { name: "Add to shelf" }).click();
  const status = page.getByRole("dialog", { name: "Add to shelf" });
  await expect(status).toBeVisible();
  await status.getByRole("button", { name: /Want to read/ }).click();

  const toast = page.getByRole("status");
  await expect(toast).toContainText("Added to Want to read");
  await expect(toast.getByRole("button", { name: "Undo" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Circe" }).getByText("Want")).toBeVisible();
});

test("shelf is a segmented list on a phone; actions open a sheet; set club pick", async ({ page }) => {
  await login(page);
  await page.goto("/shelf");

  const tabs = page.getByRole("tablist", { name: "Shelf section" });
  await expect(tabs).toBeVisible();
  await expect(tabs.getByRole("tab", { name: /Want/ })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("heading", { level: 3, name: "Circe" })).toBeVisible();

  await page.getByRole("button", { name: "Actions for Circe" }).click();
  const actions = page.getByRole("dialog", { name: "Circe" });
  await expect(actions.getByRole("button", { name: "Move to…" })).toBeVisible();
  await actions.getByRole("button", { name: "Set as club pick" }).click();

  const pickSheet = page.getByRole("dialog", { name: "Set club pick" });
  await pickSheet.getByLabel(/Why this book/).fill("Everyone kept recommending it.");
  await pickSheet.getByRole("button", { name: "Set club pick" }).click();
  await expect(page.getByRole("status")).toContainText("is the club pick");

  await page.goto("/");
  await expect(page.getByText("Club pick", { exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { level: 1, name: "Circe" })).toBeVisible();
  await expect(page.getByText("Everyone kept recommending it.")).toBeVisible();
  await expect(page.getByRole("button", { name: /Move/ }).first()).toBeVisible();
});

test("notes: post, flag a spoiler, react", async ({ page }) => {
  await login(page);
  await page.goto("/");
  const composer = page.locator("form.composer");
  await composer.getByRole("textbox").fill("The pig scene is unforgettable.");
  await composer.getByText("Spoiler").click();
  await composer.getByRole("spinbutton", { name: "Safe up to percent" }).fill("40");
  await composer.getByRole("button", { name: "Post" }).click();

  const note = page.locator(".note").filter({ hasText: "The pig scene" });
  await expect(note).toBeVisible();
  await expect(note.getByText("≤ 40%")).toBeVisible();
  await note.getByRole("button", { name: "Add reaction" }).click();
  await note.getByRole("button", { name: "React ❤️" }).click();
  await expect(note.getByRole("button", { name: /❤️ 1/ })).toHaveAttribute("aria-pressed", "true");
});

test("settings: theme toggle, notification prefs, password form, invites", async ({ page }) => {
  await login(page);
  await page.goto("/settings");
  await page.getByRole("radio", { name: "Dark" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.getByRole("radio", { name: "Light" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");

  const newNotes = page.getByRole("switch").nth(3);
  await expect(newNotes).toHaveAttribute("aria-checked", "true");
  await newNotes.click();
  await expect(newNotes).toHaveAttribute("aria-checked", "false");

  await expect(page.getByRole("heading", { name: "Password" })).toBeVisible();
  await page.getByRole("button", { name: "Create invite" }).click();
  await expect(page.locator(".invite-code").first()).toBeVisible();

  await expect(page.getByRole("heading", { level: 1, name: "Settings" })).toBeVisible();
});

test("reading schedule: add a milestone and post into its thread", async ({ page }) => {
  await login(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Add milestone" }).click();
  const sheet = page.getByRole("dialog", { name: "Add milestone" });
  await sheet.getByLabel("Title").fill("Part one");
  await sheet.getByLabel("From chapter").fill("1");
  await sheet.getByLabel("To chapter").fill("8");
  await sheet.getByRole("button", { name: "Save" }).click();

  const row = page.getByRole("button", { name: /Part one/ });
  await expect(row).toBeVisible();
  await expect(row).toContainText("Ch. 1–8");
  await row.click();
  const thread = page.getByRole("region", { name: "Milestone notes" });
  await thread.getByRole("textbox").fill("Slow start but I'm in.");
  await thread.getByRole("button", { name: "Post" }).click();
  await expect(thread.getByText("Slow start but I'm in.")).toBeVisible();
});

test("quotes: save from the actions sheet and see it on the quotes page", async ({ page }) => {
  await login(page);
  await page.goto("/shelf");
  await page.getByRole("button", { name: "Actions for Circe" }).click();
  await page.getByRole("dialog", { name: "Circe" }).getByRole("button", { name: "Save a quote" }).click();
  const sheet = page.getByRole("dialog", { name: "Save a quote" });
  await sheet.getByLabel("The line").fill("Humbling women seems to me a chief pastime of poets.");
  await sheet.getByLabel(/Page/).fill("12");
  await sheet.getByRole("button", { name: "Save quote" }).click();
  await expect(page.getByRole("status")).toContainText("Quote saved");

  await page.goto("/quotes");
  await expect(page.getByText("Humbling women seems to me")).toBeVisible();
  await expect(page.getByText("p. 12")).toBeVisible();
});

test("scan sheet resolves a typed ISBN into the detail sheet", async ({ page }) => {
  await login(page);
  await mockOpenLibrary(page);
  await page.route("**/api/books/isbn/**", (route) =>
    route.fulfill({
      json: { isbn: "9780316556347", ol_work_key: "/works/OL1W", title: "Circe", authors: "Madeline Miller", cover_id: null, year: 2018, pages: 393, on_shelf: "want_to_read", shelf_id: 1 },
    }),
  );
  await page.goto("/library");
  await page.getByRole("button", { name: "Scan a barcode" }).click();
  const scan = page.getByRole("dialog", { name: "Scan a book" });
  await scan.getByLabel("ISBN").fill("978-0-316-55634-7");
  await scan.getByRole("button", { name: "Look up" }).click();
  await expect(page.getByRole("dialog", { name: "Circe" })).toBeVisible();
});

test("offline shell: the service worker is served and registers", async ({ page, request }) => {
  const sw = await request.get("/sw.js");
  expect(sw.status()).toBe(200);
  expect(await sw.text()).toContain("bookclub-shell");
  const manifest = await request.get("/manifest.webmanifest");
  expect((await manifest.json()).icons.length).toBeGreaterThanOrEqual(2);
  expect((await request.get("/icons/icon-192.png")).status()).toBe(200);

  await login(page);
  await page.goto("/");
  await expect
    .poll(() => page.evaluate(() => navigator.serviceWorker.getRegistration().then((r) => Boolean(r))), {
      timeout: 15_000,
    })
    .toBe(true);
});

test("club page, member page, overlap, stats and quotes render", async ({ page }) => {
  await login(page);
  await page.goto("/friends");
  await expect(page.getByRole("heading", { level: 1, name: "Club" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Activity" })).toBeVisible();
  await expect(page.getByText(/added to Want to read/)).toBeVisible();

  await page.getByRole("link", { name: "You" }).click();
  await expect(page.getByRole("tab", { name: /Finished/ })).toBeVisible();

  await page.goto("/overlap");
  await expect(page.getByText("No shared wants yet.")).toBeVisible();

  await page.goto("/stats");
  await expect(page.getByText("books finished")).toBeVisible();

  await page.goto("/quotes");
  await expect(page.getByRole("heading", { level: 1, name: "Quotes" })).toBeVisible();
});

async function login(page: Page) {
  await page.goto("/login");
  if (!page.url().endsWith("/login")) return; // session cookie still valid
  await page.getByLabel("Username").fill(user);
  await page.getByLabel("Password", { exact: false }).first().fill("password123");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/$/);
}
