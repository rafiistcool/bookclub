/*
 * The kill switch for the Python app's offline worker (#52, risk R22 in
 * docs/phase1/spec/platform.md). Until PR #3 the Python app registered a service worker at
 * /sw.js, and some installed apps still run it. Since the private move the Rust server answers
 * on the same hostname, and only the app can reach that worker: the browser checks /sw.js for
 * updates, and while that check got the SPA shell (HTML) it failed and the old worker stayed.
 *
 * The server serves this file from STATIC_DIR as JavaScript, with the shell's CSP. The browser
 * bypasses the HTTP cache for a worker's script, so it needs no header of its own. It replaces
 * the old worker, deletes the old caches, unregisters itself and reloads the open pages, which
 * then load the new app. It has no fetch handler, so nothing is served from a cache in the
 * meantime. Unregistering also ends the old Web Push subscription. The app itself registers no
 * worker, so a browser without the old one never fetches this file.
 *
 * The code below the comment is deploy/legacy-origin/sw.js, which does the same on the old
 * hostname once a new name redirects from it (src/swKillSwitch.test.ts keeps them equal).
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
