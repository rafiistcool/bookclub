import type { RouteLocationNormalized, RouteLocationRaw, RouteRecordRaw } from "vue-router";

/**
 * Settings' routes, relative to the app shell's children (router.ts spreads
 * them in): the index at `settings`, and the account, its devices and the
 * Goodreads import as pages of their own. The rest opens in sheets on the
 * index or the account page, by `?sheet=<name>` from a link.
 */

/**
 * Where the old one-page Settings' `#section` anchors lead now: the sub-page
 * that holds the section, the sheet that replaced it, or the profile for the
 * picture and the favourites. Nothing in the app links an anchor any more;
 * this keeps bookmarks working.
 */
const HASH_TARGETS: Readonly<Record<string, RouteLocationRaw>> = {
  profile: "/me",
  favorites: "/me",
  appearance: { path: "/settings", query: { sheet: "appearance" } },
  language: { path: "/settings", query: { sheet: "language" } },
  timezone: { path: "/settings", query: { sheet: "timezone" } },
  legal: { path: "/settings", query: { sheet: "legal" } },
  "account-email": { path: "/settings/account", query: { sheet: "email" } },
  "account-password": { path: "/settings/account", query: { sheet: "password" } },
  "your-data": "/settings/account",
  "delete-account": { path: "/settings/account", query: { sheet: "delete" } },
  "account-sessions": "/settings/devices",
  import: "/settings/import",
  "instance-admin": "/admin",
};

/** `/settings#…` → its new place; any other anchor is dropped and the index stays. */
export function settingsHashTarget(to: Pick<RouteLocationNormalized, "hash" | "query">): RouteLocationRaw | true {
  if (!to.hash) return true;
  return HASH_TARGETS[to.hash.slice(1)] ?? { path: "/settings", query: to.query };
}

export const settingsRoutes: RouteRecordRaw[] = [
  {
    path: "settings",
    component: () => import("./pages/SettingsPage.vue"),
    beforeEnter: (to) => settingsHashTarget(to),
  },
  { path: "settings/account", component: () => import("./pages/settings/AccountSettingsPage.vue") },
  { path: "settings/devices", component: () => import("./pages/settings/DevicesPage.vue") },
  { path: "settings/import", component: () => import("./pages/settings/ImportPage.vue") },
  // An unknown sub-page (an old or mistyped link) is the index.
  { path: "settings/:rest(.*)+", redirect: "/settings" },
];
