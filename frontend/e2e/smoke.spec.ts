import { expect, test, type Page } from "@playwright/test";

const HITS = [
  { ol_work_key: "/works/OL1W", title: "Circe", authors: "Madeline Miller", cover_id: null, year: 2018, on_shelf: null, shelf_id: null, club_pick: false },
  { ol_work_key: "/works/OL2W", title: "Dune", authors: "Frank Herbert", cover_id: null, year: 1965, on_shelf: null, shelf_id: null, club_pick: false },
  { ol_work_key: "/works/OL3W", title: "Piranesi", authors: "Susanna Clarke", cover_id: null, year: 2020, on_shelf: null, shelf_id: null, club_pick: false },
];

const PAGE = { page: 1, has_more: false, items: HITS };

async function mockOpenLibrary(page: Page) {
  // Search, browse, and work detail all go to Open Library server-side; the
  // smoke run must not depend on the network, so intercept our own API calls.
  await page.route("**/api/books/search**", (route) => route.fulfill({ json: PAGE }));
  await page.route("**/api/books/trending**", (route) => route.fulfill({ json: PAGE }));
  await page.route("**/api/books/subjects/**", (route) =>
    route.fulfill({ json: { ...PAGE, items: [HITS[1], HITS[2]] } }),
  );
  await page.route("**/api/books/works/**", async (route) => {
    const id = new URL(route.request().url()).pathname.split("/").pop() ?? "";
    const hit = HITS.find((row) => row.ol_work_key.endsWith(`/${id}`)) ?? HITS[0];
    await route.fulfill({
      json: {
        ...hit,
        description: "A witch on an island, exiled by the gods.",
        subjects: ["Greek mythology", "Fantasy"],
        rating: null,
        take: "",
        dnf_reason: "",
        progress: null,
        readers: [],
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
  await expect(page.getByRole("heading", { level: 1, name: `Hello, ${user}` })).toBeVisible();
  await expect(page.getByText("No club pick yet")).toBeVisible();
  const nav = page.getByRole("navigation", { name: "Main" }).filter({ visible: true });
  await expect(nav.getByRole("link", { name: "Discover" })).toBeVisible();

  // Phone layout: nothing wider than the viewport.
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});

test("discover shows search first and browse shelves, then opens a book", async ({ page }) => {
  await login(page);
  await mockOpenLibrary(page);
  await page.goto("/discover");

  const search = page.getByRole("searchbox", { name: "Search books" });
  await expect(search).toBeVisible();
  // Permanent search field: in the top third of the phone viewport, no scrolling.
  const box = await search.boundingBox();
  expect(box!.y).toBeLessThan(844 / 3);

  await expect(page.getByRole("heading", { name: "Trending today" })).toBeVisible();
  const trending = page.locator("section[aria-labelledby='trending']");
  await expect(trending.getByRole("link", { name: /Circe/ })).toBeVisible();

  await trending.getByRole("link", { name: /Circe/ }).click();
  await expect(page).toHaveURL(/\/book\/OL1W$/);
  await expect(page.getByRole("heading", { level: 1, name: "Circe" })).toBeVisible();
  await expect(page.getByText("A witch on an island")).toBeVisible();
  await expect(page.getByRole("link", { name: "Greek mythology" })).toBeVisible();
});

test("book detail: inline status control adds to the shelf", async ({ page }) => {
  await login(page);
  await mockOpenLibrary(page);
  await page.goto("/book/OL1W");

  const status = page.getByRole("group", { name: "Shelf status" });
  await expect(page.getByText("Not on your shelf yet")).toBeVisible();
  await status.getByRole("button", { name: "Want" }).click();

  await expect(page.getByRole("status")).toContainText("Moved to Want to read");
  await expect(status.getByRole("button", { name: "Want" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("Not on your shelf yet")).toHaveCount(0);
});

test("shelf is a segmented cover grid on a phone", async ({ page }) => {
  await login(page);
  await page.goto("/shelf");

  const filter = page.getByRole("group", { name: "Filter by status" });
  await expect(filter).toBeVisible();
  await expect(page.getByRole("link", { name: /Circe/ })).toBeVisible();

  await filter.getByRole("button", { name: /^Reading/ }).click();
  await expect(page.getByText("Nothing in Reading yet.")).toBeVisible();
  await filter.getByRole("button", { name: /^Want/ }).click();
  await expect(page.getByRole("link", { name: /Circe/ })).toBeVisible();
});

test("set the club pick from the book page; home shows it", async ({ page }) => {
  await login(page);
  await mockOpenLibrary(page);
  await page.goto("/book/OL1W");

  await page.getByRole("button", { name: "Set as club pick" }).click();
  const sheet = page.getByRole("dialog", { name: "Set as club pick" });
  await expect(sheet).toBeVisible();
  await sheet.getByRole("button", { name: "Set club pick" }).click();
  await expect(page.getByRole("status")).toContainText("Set as the club pick");
  await expect(page.getByText("Current club pick")).toBeVisible();

  await page.goto("/");
  await expect(page.getByText("Reading now")).toBeVisible();
  await expect(page.getByRole("heading", { level: 2, name: "Circe" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Where everyone is" })).toBeVisible();
});

test("settings: colour mode, palette, and invites", async ({ page }) => {
  await login(page);
  await page.goto("/settings");
  await expect(page.getByRole("heading", { level: 1, name: "Settings" })).toBeVisible();

  const mode = page.getByRole("group", { name: "Colour mode" });
  await mode.getByRole("button", { name: "Dark" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-mode", "dark");
  await mode.getByRole("button", { name: "Light" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-mode", "light");

  await page.getByRole("button", { name: /^Ink/ }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "ink");

  // The choice is stored on the account and survives a fresh load.
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "ink");
  await expect(page.locator("html")).toHaveAttribute("data-mode", "light");
  await page.getByRole("button", { name: /^Paper/ }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "paper");

  await page.getByRole("button", { name: "Create invite" }).click();
  await expect(page.locator(".invite-code").first()).toBeVisible();
});

test("club page and legacy redirects", async ({ page }) => {
  await login(page);
  await page.goto("/club");
  await expect(page.getByRole("heading", { level: 1, name: "Club" })).toBeVisible();
  await expect(page.getByText("You're the only one here")).toBeVisible();
  await expect(page.getByText("No shared wants yet.")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Next-up vote" })).toBeVisible();

  await page.goto("/library");
  await expect(page).toHaveURL(/\/discover$/);
  await page.goto("/friends");
  await expect(page).toHaveURL(/\/club$/);
});

async function login(page: Page) {
  await page.goto("/login");
  if (!page.url().endsWith("/login")) return; // session cookie still valid
  await page.getByLabel("Username").fill(user);
  await page.getByLabel("Password", { exact: false }).first().fill("password123");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page).toHaveURL(/\/$/);
}
