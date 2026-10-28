import { createRouter, createWebHistory, type RouteLocationRaw } from "vue-router";
import { loginRoute } from "./nextPath";
import { installSessionExpiry } from "./sessionExpiry";
import { clubLanding, useClubs } from "./stores/clubs";
import { useSession } from "./stores/session";

const ClubsPage = () => import("./pages/ClubsPage.vue");
const LegalPage = () => import("./pages/LegalPage.vue");

/**
 * A club link without a slug (the Club tab, `/club`, the old single-club
 * paths): `sub` of the last-used or only club, else the club list. It runs as
 * `beforeEnter`, after the session (and with it the viewer's clubs) is loaded.
 */
function toClub(sub = "", withoutClub?: string) {
  return (): RouteLocationRaw => {
    const clubs = useClubs();
    if (withoutClub && !clubs.defaultSlug) return withoutClub;
    return clubLanding(clubs.items, clubs.lastSlug, sub);
  };
}

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
      path: "/signup",
      component: () => import("./pages/SignupPage.vue"),
      meta: { guest: true },
    },
    // The old sign-up path; `?code=` comes along.
    { path: "/register", redirect: (to) => ({ path: "/signup", query: to.query }) },
    {
      path: "/forgot-password",
      component: () => import("./pages/ForgotPasswordPage.vue"),
      meta: { guest: true },
    },
    // Mail links (`#token=…`). Public: a signed-in reader may open them too.
    { path: "/verify-email", component: () => import("./pages/VerifyEmailPage.vue") },
    { path: "/reset-password", component: () => import("./pages/ResetPasswordPage.vue") },
    {
      // An invite link. Public: a reader who is signed out signs in or up first.
      path: "/join/:code",
      component: () => import("./pages/JoinPage.vue"),
    },
    // The operator's legal texts (#53). Public and outside the app shell, whose
    // catch-all would send a signed-out visitor to the login page.
    { path: "/imprint", component: LegalPage, props: { doc: "imprint" } },
    { path: "/privacy", component: LegalPage, props: { doc: "privacy" } },
    { path: "/terms", component: LegalPage, props: { doc: "terms" } },
    // The German names people type or link (Impressum, Datenschutz).
    { path: "/impressum", redirect: "/imprint" },
    { path: "/datenschutz", redirect: "/privacy" },
    { path: "/nutzungsbedingungen", redirect: "/terms" },
    {
      path: "/",
      component: () => import("./layouts/AppShell.vue"),
      meta: { auth: true },
      children: [
        { path: "", component: () => import("./pages/HomePage.vue") },
        { path: "discover", component: () => import("./pages/DiscoverPage.vue") },
        { path: "shelf", component: () => import("./pages/ShelfPage.vue") },
        { path: "shelf/physical", component: () => import("./pages/PhysicalShelfPage.vue") },
        { path: "diary", component: () => import("./pages/DiaryPage.vue") },
        { path: "quotes", component: () => import("./pages/QuotesPage.vue") },
        { path: "stats", component: () => import("./pages/StatsPage.vue") },
        { path: "clubs", component: ClubsPage },
        { path: "clubs/new", component: () => import("./pages/NewClubPage.vue") },
        {
          path: "c/:slug",
          component: () => import("./layouts/ClubLayout.vue"),
          children: [
            { path: "", component: () => import("./pages/club/ClubHomePage.vue") },
            { path: "discussions", component: () => import("./pages/club/ClubDiscussionsPage.vue") },
            { path: "vote", component: () => import("./pages/club/ClubVotePage.vue") },
            { path: "members", component: () => import("./pages/club/ClubMembersPage.vue") },
            { path: "overlap", component: () => import("./pages/club/ClubOverlapPage.vue") },
            { path: "events", component: () => import("./pages/club/ClubEventsPage.vue") },
            { path: "stats", component: () => import("./pages/club/ClubStatsPage.vue") },
          ],
        },
        // The Club tab: never rendered itself, it always leads on.
        { path: "club", component: ClubsPage, beforeEnter: toClub() },
        // Another reader's shelves. Personal data follows the audience, not a club.
        {
          path: "u/:username/shelf",
          component: () => import("./pages/MemberPhysicalShelfPage.vue"),
        },
        {
          path: "u/:username/stats",
          component: () => import("./pages/StatsPage.vue"),
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
        // Instance admins only (`UserOut.is_admin`); linked from Settings.
        { path: "admin", component: () => import("./pages/AdminPage.vue"), meta: { admin: true } },

        // Paths from the pre-redesign IA, kept so bookmarks and the PWA's
        // stored start URL keep resolving.
        { path: "library", redirect: "/discover" },
        { path: "friends", redirect: "/club" },
        { path: "friends/:username", redirect: (to) => `/u/${to.params.username}` },
        { path: "club/:username/shelf", redirect: (to) => `/u/${to.params.username}/shelf` },
        { path: "club/:username", redirect: (to) => `/u/${to.params.username}` },
        { path: "overlap", component: ClubsPage, beforeEnter: toClub("/overlap") },
        { path: "invites", component: ClubsPage, beforeEnter: toClub("/members") },
        // The pick was the old Home; without a club to show it in, Home it is.
        { path: "pick", component: ClubsPage, beforeEnter: toClub("", "/") },

        // Anything else, also an unknown tab under a club or a reader: a
        // not-found page in the app shell. A signed-out visit signs in first.
        {
          path: ":pathMatch(.*)*",
          name: "not-found",
          component: () => import("./pages/NotFoundPage.vue"),
        },
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
  if (to.meta.admin && !session.user?.is_admin) {
    return { path: "/" };
  }
  return true;
});

installSessionExpiry(router);

export default router;
