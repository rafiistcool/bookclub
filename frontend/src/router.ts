import { createRouter, createWebHistory } from "vue-router";
import { loginRoute } from "./nextPath";
import { installSessionExpiry } from "./sessionExpiry";
import { useSession } from "./stores/session";

const router = createRouter({
  history: createWebHistory(),
  scrollBehavior(to, from, saved) {
    if (saved) return saved;
    return { top: 0 };
  },
  routes: [
    {
      path: "/login",
      component: () => import("./pages/LoginPage.vue"),
      meta: { guest: true },
    },
    {
      path: "/register",
      component: () => import("./pages/RegisterPage.vue"),
      meta: { guest: true },
    },
    {
      path: "/",
      component: () => import("./layouts/AppShell.vue"),
      meta: { auth: true },
      children: [
        { path: "", component: () => import("./pages/HomePage.vue") },
        { path: "discover", component: () => import("./pages/DiscoverPage.vue") },
        { path: "shelf", component: () => import("./pages/ShelfPage.vue") },
        { path: "shelf/physical", component: () => import("./pages/PhysicalShelfPage.vue") },
        { path: "club", component: () => import("./pages/ClubPage.vue") },
        // Another reader's shelves. Personal data follows the audience, not a club.
        {
          path: "u/:username/shelf",
          component: () => import("./pages/MemberPhysicalShelfPage.vue"),
        },
        {
          path: "u/:username",
          component: () => import("./pages/MemberShelfPage.vue"),
        },
        // A numeric work id. Legacy keys (OL…W, BC…, ISBN…, GB…) resolve on the page.
        {
          path: "book/:id",
          component: () => import("./pages/BookDetailPage.vue"),
        },
        { path: "settings", component: () => import("./pages/SettingsPage.vue") },

        // Paths from the pre-redesign IA, kept so bookmarks and the PWA's
        // stored start URL keep resolving.
        { path: "library", redirect: "/discover" },
        { path: "friends", redirect: "/club" },
        { path: "friends/:username", redirect: (to) => `/u/${to.params.username}` },
        { path: "club/:username/shelf", redirect: (to) => `/u/${to.params.username}/shelf` },
        { path: "club/:username", redirect: (to) => `/u/${to.params.username}` },
        { path: "overlap", redirect: "/club" },
        { path: "invites", redirect: "/settings" },
        { path: "pick", redirect: "/" },
      ],
    },
  ],
});

router.beforeEach(async (to) => {
  const session = useSession();
  if (!session.ready) {
    try {
      await session.hydrate();
    } catch {
      session.ready = true;
      session.user = null;
    }
  }
  if (to.meta.auth && !session.user) {
    return loginRoute(to.fullPath);
  }
  if (to.meta.guest && session.user) {
    return { path: "/" };
  }
  return true;
});

installSessionExpiry(router);

export default router;
