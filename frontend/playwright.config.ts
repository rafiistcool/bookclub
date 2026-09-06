import { defineConfig } from "@playwright/test";

const PORT = Number(process.env.E2E_PORT ?? 8010);

/**
 * Smoke run at phone size against the real backend (fresh SQLite in a temp
 * dir, invite E2E-INVITE). Open Library calls are route-mocked in the spec so
 * the run never depends on the network. `npm run build` must have produced
 * backend/app/static first; CI does that in the same job.
 */
export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  expect: { timeout: 10_000 },
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["github"], ["list"]] : "list",
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    browserName: "chromium",
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true,
    trace: "retain-on-failure",
  },
  webServer: {
    command: `bash e2e/serve.sh ${PORT}`,
    url: `http://127.0.0.1:${PORT}/api/health`,
    reuseExistingServer: false,
    timeout: 120_000,
    stdout: "ignore",
    stderr: "pipe",
  },
});
