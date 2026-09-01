import { createRouter, createWebHistory } from "vue-router";
import { useSession } from "./stores/session";

declare module "vue-router" {
  interface RouteMeta {
    auth?: boolean;
    guest?: boolean;
    title?: string;
    back?: string;
    wide?: boolean;
  }
}

const router = createRouter({
  history: createWebHistory(),
  scrollBehavior(_to, _from, saved) {
    return saved ?? { top: 0 };
  },
  routes: [
    {
      path: "/login",
      component: () => import("./pages/LoginPage.vue"),
      meta: { guest: true, title: "Sign in" },
    },
    {
      path: "/register",
      component: () => import("./pages/RegisterPage.vue"),
      meta: { guest: true, title: "Create an account" },
    },
    {
      path: "/",
      component: () => import("./layouts/AppShell.vue"),
      meta: { auth: true },
      children: [
        { path: "", name: "home", component: () => import("./pages/HomePage.vue"), meta: { title: "Home" } },
        {
          path: "library",
          name: "library",
          component: () => import("./pages/LibraryPage.vue"),
          meta: { title: "Library", wide: true },
        },
        {
          path: "shelf",
          name: "shelf",
          component: () => import("./pages/ShelfPage.vue"),
          meta: { title: "Your shelf", wide: true },
        },
        {
          path: "friends",
          name: "friends",
          component: () => import("./pages/FriendsPage.vue"),
          meta: { title: "Club" },
        },
        {
          path: "friends/:username",
          name: "member",
          component: () => import("./pages/FriendShelfPage.vue"),
          meta: { title: "Member", back: "/friends", wide: true },
        },
        {
          path: "overlap",
          name: "overlap",
          component: () => import("./pages/OverlapPage.vue"),
          meta: { title: "Shared to-read", back: "/friends" },
        },
        {
          path: "stats",
          name: "stats",
          component: () => import("./pages/StatsPage.vue"),
          meta: { title: "Year in review", back: "/friends" },
        },
        {
          path: "quotes",
          name: "quotes",
          component: () => import("./pages/QuotesPage.vue"),
          meta: { title: "Quotes", back: "/friends" },
        },
        {
          path: "settings",
          name: "settings",
          component: () => import("./pages/SettingsPage.vue"),
          meta: { title: "Settings" },
        },
        { path: "invites", redirect: "/settings" },
        { path: "pick", redirect: "/" },
      ],
    },
    { path: "/:pathMatch(.*)*", redirect: "/" },
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
