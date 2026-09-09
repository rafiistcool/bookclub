export type NavIconName = "home" | "discover" | "shelf" | "club" | "settings";

export type NavItem = {
  to: string;
  labelKey: string;
  icon: NavIconName;
  /** Mobile keeps four tabs; Settings lives behind the account button. */
  bottomBar: boolean;
};

export const NAV_ITEMS: readonly NavItem[] = [
  { to: "/", labelKey: "nav.home", icon: "home", bottomBar: true },
  { to: "/discover", labelKey: "nav.discover", icon: "discover", bottomBar: true },
  { to: "/shelf", labelKey: "nav.shelf", icon: "shelf", bottomBar: true },
  { to: "/club", labelKey: "nav.club", icon: "club", bottomBar: true },
  { to: "/settings", labelKey: "nav.settings", icon: "settings", bottomBar: false },
];

export function isActive(path: string, to: string): boolean {
  if (to === "/") return path === "/";
  return path === to || path.startsWith(`${to}/`);
}
