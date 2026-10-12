export type NavIconName = "home" | "discover" | "shelf" | "club" | "settings";

export type NavItem = {
  to: string;
  labelKey: string;
  icon: NavIconName;
  /** A shorter label for the phone tab bar, where five tabs share the width. */
  shortLabelKey?: string;
  bottomBar: boolean;
};

export const NAV_ITEMS: readonly NavItem[] = [
  { to: "/", labelKey: "nav.home", icon: "home", bottomBar: true },
  { to: "/discover", labelKey: "nav.discover", icon: "discover", bottomBar: true },
  { to: "/shelf", labelKey: "nav.shelf", icon: "shelf", bottomBar: true },
  { to: "/club", labelKey: "nav.club", icon: "club", bottomBar: true },
  { to: "/settings", labelKey: "nav.settings", shortLabelKey: "nav.settingsShort", icon: "settings", bottomBar: true },
];

export function isActive(path: string, to: string): boolean {
  if (to === "/") return path === "/";
  return path === to || path.startsWith(`${to}/`);
}
