import { createRouter, createWebHistory } from "vue-router";
import { useSession } from "./stores/session";

const router = createRouter({
  history: createWebHistory(),
  scrollBehavior() {
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
        { path: "library", component: () => import("./pages/LibraryPage.vue") },
        { path: "shelf", component: () => import("./pages/ShelfPage.vue") },
        { path: "friends", component: () => import("./pages/FriendsPage.vue") },
        {
          path: "friends/:username",
          component: () => import("./pages/FriendShelfPage.vue"),
        },
        { path: "invites", component: () => import("./pages/InvitesPage.vue") },
        { path: "overlap", component: () => import("./pages/OverlapPage.vue") },
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
