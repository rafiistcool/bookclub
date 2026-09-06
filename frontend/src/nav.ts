export type NavIconName = "home" | "discover" | "shelf" | "club" | "settings";

export type NavItem = {
  to: string;
  label: string;
  icon: NavIconName;
  /** Mobile keeps four tabs; Settings lives behind the account button. */
  bottomBar: boolean;
};

export const NAV_ITEMS: readonly NavItem[] = [
  { to: "/", label: "Home", icon: "home", bottomBar: true },
  { to: "/discover", label: "Discover", icon: "discover", bottomBar: true },
  { to: "/shelf", label: "Shelf", icon: "shelf", bottomBar: true },
  { to: "/club", label: "Club", icon: "club", bottomBar: true },
  { to: "/settings", label: "Settings", icon: "settings", bottomBar: false },
];

export function isActive(path: string, to: string): boolean {
  if (to === "/") return path === "/";
  return path === to || path.startsWith(`${to}/`);
}
