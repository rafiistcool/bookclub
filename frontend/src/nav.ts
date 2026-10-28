export type NavIconName = "home" | "discover" | "shelf" | "club" | "settings";

export type NavItem = {
  to: string;
  labelKey: string;
  icon: NavIconName;
  /** A shorter label for the phone tab bar, where five tabs share the width. */
  shortLabelKey?: string;
  bottomBar: boolean;
  /** Other path prefixes that belong to this tab. */
  also?: readonly string[];
};

export const NAV_ITEMS: readonly NavItem[] = [
  { to: "/", labelKey: "nav.home", icon: "home", bottomBar: true },
  { to: "/discover", labelKey: "nav.discover", icon: "discover", bottomBar: true },
  { to: "/shelf", labelKey: "nav.shelf", icon: "shelf", bottomBar: true, also: ["/diary", "/quotes", "/stats"] },
  // `/club` leads to the last-used club, the only club, or the club list (router.ts).
  { to: "/club", labelKey: "nav.club", icon: "club", bottomBar: true, also: ["/c", "/clubs"] },
  { to: "/settings", labelKey: "nav.settings", shortLabelKey: "nav.settingsShort", icon: "settings", bottomBar: true },
];

function under(path: string, prefix: string): boolean {
  return path === prefix || path.startsWith(`${prefix}/`);
}

/** Whether `path` belongs to a tab: its own path, anything below it, or one of its `also` prefixes. */
export function isActive(path: string, item: string | Pick<NavItem, "to" | "also">): boolean {
  const { to, also = [] } = typeof item === "string" ? { to: item } : item;
  if (to === "/") return path === "/";
  return under(path, to) || also.some((prefix) => under(path, prefix));
}
