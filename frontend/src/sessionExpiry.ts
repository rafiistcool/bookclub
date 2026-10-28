import type { Router } from "vue-router";
import { setUnauthorizedHandler } from "./api/client";
import { loginRoute } from "./nextPath";
import { useSession } from "./stores/session";

/**
 * A 401 on any request while signed in means the server ended the session
 * (logout everywhere, a password reset, a deleted account). Forget the user
 * and go to the login page, which brings them back here afterwards.
 */
export function installSessionExpiry(router: Router) {
  setUnauthorizedHandler(() => {
    const session = useSession();
    if (!session.user) return;
    session.reset();
    const current = router.currentRoute.value;
    if (current.meta.guest) return;
    void router.replace(loginRoute(current.fullPath));
  });
}
