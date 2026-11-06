import { createRouter, createWebHistory, type RouteLocationNormalized, type RouteLocationRaw } from "vue-router";
import { loginRoute } from "./nextPath";
import { installSessionExpiry } from "./sessionExpiry";
import { settingsRoutes } from "./settingsRoutes";
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

/**
 * A link to the viewer's own name under `/u/` (feeds, readers lists, quotes)
 * opens their own page instead of the read-only view others get: the route's
 * `meta.ownPath`. It runs in the global guard, after the session is loaded,
 * and so also on a move from one reader to another, where `beforeEnter`
 * would not run. Names compare as the server does: trimmed, in lower case.
 */
function ownPage(to: RouteLocationNormalized): RouteLocationRaw | null {
  const target = to.meta.ownPath;
  const me = useSession().user?.username;
  if (typeof target !== "string" || !me) return null;
  if (String(to.params.username ?? "").trim().toLowerCase() !== me.toLowerCase()) return null;
  return { path: target, query: to.query, hash: to.hash };
}

/**
 * `#section` on a page (the club's "More" sheet opens the members page at
 * `#invites`): wait until the section is there and has stopped moving (the
 * page above it may still be loading), then bring it in with
 * `scrollIntoView`, which keeps the target's `scroll-margin-top` clear of the
 * sticky bars. For a while after, a section pushed down by late content is
 * followed, unless the reader has scrolled meanwhile. A section that never
 * appears (a member has no invites) leaves the page at the top. The wait ends
 * without a scroll once the reader has moved on (another page, Back) or
 * scrolled: vue-router applies whatever this resolves to, on whatever page is
 * current by then.
 */
function scrollToSection(id: string, fullPath: string): Promise<false | { top: number }> {
  return new Promise((resolve) => {
    const started = performance.now();
    // Not there yet: meanwhile the page starts at the top, as any other page does.
    if (!document.getElementById(id) && window.scrollY !== 0) window.scrollTo(0, 0);
    const startY = window.scrollY;
    let lastTop: number | null = null;
    let still = 0;
    let placed: { top: number; height: number; scrollY: number } | null = null;
    const topOf = (element: HTMLElement) => element.getBoundingClientRect().top + window.scrollY;
    const place = (element: HTMLElement) => {
      element.scrollIntoView({ block: "start" });
      placed = { top: topOf(element), height: document.documentElement.scrollHeight, scrollY: window.scrollY };
    };
    const check = () => {
      const element = document.getElementById(id);
      const elapsed = performance.now() - started;
      if (placed) {
        // Follow a late shift (or a page grown tall enough) until 2.5 s
        // after the tap, while the reader has not scrolled.
        if (!element || elapsed > 2500 || window.scrollY !== placed.scrollY) return;
        if (topOf(element) !== placed.top || document.documentElement.scrollHeight !== placed.height) place(element);
        requestAnimationFrame(check);
        return;
      }
      if (router.currentRoute.value.fullPath !== fullPath || window.scrollY !== startY) {
        resolve(false);
        return;
      }
      if (element) {
        const top = topOf(element);
        still = top === lastTop ? still + 1 : 0;
        lastTop = top;
        if (still >= 4 || elapsed > 1000) {
          place(element);
          resolve(false);
          requestAnimationFrame(check);
          return;
        }
      } else if (elapsed > 2000) {
        resolve({ top: 0 });
        return;
      }
      requestAnimationFrame(check);
    };
    requestAnimationFrame(check);
  });
}

const router = createRouter({
  history: createWebHistory(),
  scrollBehavior(to, from, saved) {
    if (saved) return saved;
    // Only a plain id: the mail links carry `#token=…`, which is no section.
    // A diary entry (`#entry-N`, the feeds' links) is BookDiary's: it centres it once loaded.
    const section = /^#([A-Za-z][\w-]*)$/.exec(to.hash)?.[1];
    if (section && !section.startsWith("entry-")) return scrollToSection(section, to.fullPath);
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
        // The barcode scanner (#56), opened from Discover; Back closes it.
        { path: "scan", component: () => import("./pages/ScanPage.vue") },
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
        // The viewer's own name leads to their own page (`ownPage`).
        {
          path: "u/:username/shelf",
          component: () => import("./pages/MemberPhysicalShelfPage.vue"),
          meta: { ownPath: "/shelf/physical" },
        },
        {
          path: "u/:username/stats",
          component: () => import("./pages/StatsPage.vue"),
          meta: { ownPath: "/stats" },
        },
        {
          path: "u/:username",
          component: () => import("./pages/MemberShelfPage.vue"),
          meta: { ownPath: "/me" },
        },
        // A numeric work id. Legacy keys (OL…W, BC…, ISBN…, GB…) resolve on the page.
        {
          path: "book/:id",
          component: () => import("./pages/BookDetailPage.vue"),
        },
        // The Profile tab: the reader's own page, with Friends and Settings under it.
        { path: "me", component: () => import("./pages/ProfilePage.vue") },
        // Friends, requests, blocks and invitations into clubs (#57). From the profile, and Home while some wait.
        { path: "friends", component: () => import("./pages/FriendsPage.vue") },
        ...settingsRoutes,
        // Instance admins only (`UserOut.is_admin`); linked from Settings.
        { path: "admin", component: () => import("./pages/AdminPage.vue"), meta: { admin: true } },

        // Paths from the pre-redesign IA, kept so bookmarks and the PWA's
        // stored start URL keep resolving.
        { path: "library", redirect: "/discover" },
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
  return ownPage(to) ?? true;
});

installSessionExpiry(router);

export default router;
