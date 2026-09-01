/* Bookclub service worker: offline app shell, cached covers, Web Push. */

const VERSION = "v1";
const SHELL_CACHE = `bookclub-shell-${VERSION}`;
const COVER_CACHE = `bookclub-covers-${VERSION}`;
const DATA_CACHE = `bookclub-data-${VERSION}`;
const SHELL_URLS = ["/", "/manifest.webmanifest", "/favicon.svg"];
const MAX_COVERS = 300;

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(SHELL_CACHE)
      .then((cache) => cache.addAll(SHELL_URLS).catch(() => undefined))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys
            .filter((key) => key.startsWith("bookclub-") && !key.endsWith(VERSION))
            .map((key) => caches.delete(key)),
        ),
      )
      .then(() => self.clients.claim()),
  );
});

async function trimCache(name, max) {
  const cache = await caches.open(name);
  const keys = await cache.keys();
  for (let index = 0; index < keys.length - max; index += 1) {
    await cache.delete(keys[index]);
  }
}

function isNavigation(request) {
  return request.mode === "navigate";
}

function isAsset(url) {
  return url.origin === self.location.origin && url.pathname.startsWith("/assets/");
}

function isCover(url) {
  return url.hostname === "covers.openlibrary.org";
}

function isReadableApi(url, request) {
  if (url.origin !== self.location.origin || request.method !== "GET") return false;
  // Only the member's own shelf and the club pick are worth having offline.
  return url.pathname === "/api/shelf" || url.pathname === "/api/pick" || url.pathname === "/api/config";
}

self.addEventListener("fetch", (event) => {
  const { request } = event;
  if (request.method !== "GET") return;
  const url = new URL(request.url);

  if (isNavigation(request)) {
    event.respondWith(
      fetch(request)
        .then((response) => {
          const copy = response.clone();
          caches.open(SHELL_CACHE).then((cache) => cache.put("/", copy));
          return response;
        })
        .catch(() => caches.match("/")),
    );
    return;
  }

  if (isAsset(url)) {
    event.respondWith(
      caches.match(request).then(
        (hit) =>
          hit ||
          fetch(request).then((response) => {
            const copy = response.clone();
            caches.open(SHELL_CACHE).then((cache) => cache.put(request, copy));
            return response;
          }),
      ),
    );
    return;
  }

  if (isCover(url)) {
    event.respondWith(
      caches.match(request).then(
        (hit) =>
          hit ||
          fetch(request).then((response) => {
            if (response.ok) {
              const copy = response.clone();
              caches
                .open(COVER_CACHE)
                .then((cache) => cache.put(request, copy))
                .then(() => trimCache(COVER_CACHE, MAX_COVERS));
            }
            return response;
          }),
      ),
    );
    return;
  }

  if (isReadableApi(url, request)) {
    event.respondWith(
      fetch(request)
        .then((response) => {
          if (response.ok) {
            const copy = response.clone();
            caches.open(DATA_CACHE).then((cache) => cache.put(request, copy));
          }
          return response;
        })
        .catch(() => caches.match(request)),
    );
  }
});

self.addEventListener("push", (event) => {
  let payload = {};
  try {
    payload = event.data ? event.data.json() : {};
  } catch {
    payload = { title: "Bookclub", body: event.data ? event.data.text() : "" };
  }
  const title = payload.title || "Bookclub";
  const options = {
    body: payload.body || "",
    tag: payload.tag || payload.kind || "bookclub",
    renotify: Boolean(payload.tag),
    icon: "/icons/icon-192.png",
    badge: "/icons/badge-72.png",
    data: { url: payload.url || "/" },
  };
  event.waitUntil(self.registration.showNotification(title, options));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const target = new URL(event.notification.data?.url || "/", self.location.origin).href;
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((clients) => {
      for (const client of clients) {
        if ("focus" in client) {
          client.navigate?.(target);
          return client.focus();
        }
      }
      return self.clients.openWindow(target);
    }),
  );
});
