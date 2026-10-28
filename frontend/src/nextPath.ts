import type { RouteLocationRaw } from "vue-router";

/**
 * A `?next=` target that stays inside this app: a path that starts with one
 * slash. Anything a browser could read as another origin (`//host`, a
 * backslash, control characters it would strip) becomes "/".
 */
export function safeNextPath(raw: unknown): string {
  if (typeof raw !== "string") return "/";
  if (!raw.startsWith("/") || raw.startsWith("//")) return "/";
  if (raw.includes("\\") || /[\u0000-\u001f\u007f]/.test(raw)) return "/";
  return raw;
}

/** The login page, coming back to `fullPath` afterwards. */
export function loginRoute(fullPath: string): RouteLocationRaw {
  return { path: "/login", query: { next: safeNextPath(fullPath) } };
}
