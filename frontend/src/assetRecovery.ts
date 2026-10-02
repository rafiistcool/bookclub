/** A tab left open across a release can still reference removed Vite chunks. */
export function installAssetRecovery() {
  window.addEventListener("vite:preloadError", (event) => {
    const key = "bookclub.asset-reload-at";
    const now = Date.now();
    try {
      const last = Number(window.sessionStorage.getItem(key));
      if (last > 0 && now - last < 60_000) return;
      window.sessionStorage.setItem(key, String(now));
    } catch {
      // Without a persisted guard, a stale proxy response could reload forever.
      return;
    }
    event.preventDefault();
    window.location.reload();
  });
}
