/*
 * The old Bookclub origin after the cutover (#52): this worker replaces the offline worker of
 * the Python app (frontend/public/sw.js until PR #3), which some installed apps still run.
 *
 * Every other path on the old host redirects to the new one, but a browser does not follow a
 * redirect when it checks a service worker for updates. Its check of /sw.js gets this file
 * instead (served with Cache-Control: no-store, see Caddyfile.example). The worker deletes the
 * old caches, unregisters itself and reloads the open pages, which then follow the redirect.
 * It has no fetch handler, so nothing is served from a cache in the meantime. Unregistering
 * also ends the old Web Push subscription.
 */

self.addEventListener("install", () => {
  // Replace the old worker at once, without waiting for its pages to close.
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      await self.clients.claim();
      // The old worker kept bookclub-shell-*, bookclub-covers-* and bookclub-data-* here;
      // nothing else on this origin uses the Cache API.
      const keys = await caches.keys();
      await Promise.all(keys.map((key) => caches.delete(key)));
      await self.registration.unregister();
      const windows = await self.clients.matchAll({ type: "window" });
      await Promise.all(
        windows.map((client) => client.navigate(client.url).catch(() => undefined)),
      );
    })(),
  );
});
