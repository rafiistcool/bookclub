/**
 * Where the book page's "←" link leads: back to the page the reader came
 * from, named after it ("← Regal", "← Start", "← mara"), and to Discover only
 * when the book was opened directly (a shared link, a reload).
 *
 * `previous` is vue-router's `history.state.back`: the previous entry's path
 * in this tab, or null when there is none.
 */
export type BackTarget =
  | { kind: "back"; page: BackPage | null; username?: string }
  | { kind: "fallback"; to: "/discover" };

/** A page the link can name; null names none ("← Zurück"). */
export type BackPage = "home" | "discover" | "shelf" | "club" | "profile" | "user";

export function backTarget(previous: string | null | undefined): BackTarget {
  if (typeof previous !== "string" || !previous.startsWith("/")) return { kind: "fallback", to: "/discover" };
  const path = previous.split(/[?#]/)[0].replace(/\/+$/, "") || "/";
  if (path === "/") return { kind: "back", page: "home" };
  if (path === "/discover" || path === "/scan") return { kind: "back", page: "discover" };
  if (path === "/shelf" || path.startsWith("/shelf/")) return { kind: "back", page: "shelf" };
  if (path === "/clubs" || path.startsWith("/c/")) return { kind: "back", page: "club" };
  if (path === "/me") return { kind: "back", page: "profile" };
  const user = /^\/u\/([^/]+)/.exec(path);
  if (user) {
    let username = user[1];
    try {
      username = decodeURIComponent(username);
    } catch {
      // A malformed escape: show it as it stands.
    }
    return { kind: "back", page: "user", username };
  }
  return { kind: "back", page: null };
}
