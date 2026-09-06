import { createRouter, createWebHistory } from "vue-router";
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
        { path: "club", component: () => import("./pages/ClubPage.vue") },
        {
          path: "club/:username",
          component: () => import("./pages/MemberShelfPage.vue"),
        },
        {
          path: "book/:workId",
          component: () => import("./pages/BookDetailPage.vue"),
        },
        { path: "settings", component: () => import("./pages/SettingsPage.vue") },

        // Paths from the pre-redesign IA, kept so bookmarks and the PWA's
        // stored start URL keep resolving.
        { path: "library", redirect: "/discover" },
        { path: "friends", redirect: "/club" },
        { path: "friends/:username", redirect: (to) => `/club/${to.params.username}` },
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
    return { path: "/login", query: { next: to.fullPath } };
  }
  if (to.meta.guest && session.user) {
    return { path: "/" };
  }
  return true;
});

export default router;
