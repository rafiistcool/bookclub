<script setup lang="ts">
import { XMarkIcon } from "@heroicons/vue/24/outline";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute, useRouter } from "vue-router";
import { api, ApiError } from "../api/client";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import LanguageSwitch from "../components/LanguageSwitch.vue";
import LegalLinks from "../components/LegalLinks.vue";
import { tp } from "../i18n";
import { clearJoinCode, parseInviteCode, rememberJoinCode } from "../invites";
import { useClubs } from "../stores/clubs";
import { useService } from "../stores/service";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { InvitePreview } from "../types";

/**
 * An invite link (`/join/<code>`): who invites, to which club, what it reads
 * now and how many are in it, then one action. Signed in, one tap joins.
 * Signed out, the code waits (in the login `next` and in this tab) while the
 * reader signs in or creates an account, and then they come back here.
 *
 * The dead ends say what to do next: a code that no longer works, a full
 * club, someone else's personal invite. A reader already in the club goes
 * straight to it, and a sign-up invite opened while signed in goes home.
 * A personal invite never makes an account, so signed out it offers only
 * signing in.
 */
const { t } = useI18n();
const route = useRoute();
const router = useRouter();
const service = useService();
const session = useSession();
const clubs = useClubs();
const toast = useToast();

const code = computed(() => parseInviteCode(String(route.params.code ?? "")));
const preview = ref<InvitePreview | null>(null);
const loaded = ref(false);
/** The code is unknown or no longer valid (the server's 404). */
const invalid = ref(false);
/** The preview could not be loaded (network, server). */
const loadFailed = ref(false);
const error = ref("");
const pending = ref(false);

const signedIn = computed(() => Boolean(session.user));
const club = computed(() => preview.value?.club ?? null);
const inviter = computed(() => preview.value?.inviter ?? null);
const personal = computed(() => Boolean(preview.value?.personal));
const loginLink = computed(() => ({ path: "/login", query: { next: `/join/${code.value}` } }));
const signupLink = computed(() => ({ path: "/signup", query: { code: code.value } }));

type DeadEnd = {
  title: string;
  body: string;
  action: { to: string | { path: string; query: Record<string, string> }; label: string } | null;
};

/** A page that cannot lead into the club, with the one thing left to do. */
const deadEnd = computed<DeadEnd | null>(() => {
  // Signed out, the way on is signing in: a member then goes on into the club.
  const home = signedIn.value ? { to: "/", label: t("join.home") } : { to: loginLink.value, label: t("join.signIn") };
  if (invalid.value) {
    return {
      title: t("join.invalidTitle"),
      body: t("join.invalidBody"),
      action: signedIn.value ? { to: "/clubs", label: t("join.toClubs") } : { to: "/login", label: t("join.signIn") },
    };
  }
  const current = preview.value;
  if (!current || !club.value) return null;
  if (signedIn.value && current.for_you === false) {
    return { title: t("join.notYoursTitle"), body: t("join.notYoursBody"), action: home };
  }
  if (current.full && !current.you_are_member) {
    return {
      title: t("join.fullTitle", { club: club.value.name }),
      body: inviter.value ? t("join.fullBodyInviter", { name: inviter.value.username }) : t("join.fullBody"),
      action: home,
    };
  }
  return null;
});

function isMember(current: InvitePreview): boolean {
  const slug = current.club?.slug;
  if (!slug) return false;
  return Boolean(current.you_are_member) || Boolean(session.user?.clubs.some((row) => row.slug === slug));
}

/** Leaves for a page that fits better; the page stays a skeleton meanwhile. */
async function leave(to: string, message: string) {
  clearJoinCode();
  toast.show(message);
  await router.replace(to);
}

async function load() {
  loaded.value = false;
  loadFailed.value = false;
  invalid.value = false;
  try {
    const current = await api.previewInvite(code.value);
    preview.value = current;
    if (session.user && current.club && isMember(current)) {
      await leave(`/c/${current.club.slug}`, t("join.alreadyInToast", { club: current.club.name }));
      return;
    }
    if (session.user && current.kind === "signup") {
      await leave("/", t("join.signupInviteSignedInToast"));
      return;
    }
    // A personal code never signs anyone up, and a full club takes nobody: sign-up need not keep it.
    if (!session.user && !current.personal && !current.full) rememberJoinCode(code.value);
  } catch (err) {
    if (err instanceof ApiError && err.status === 404) invalid.value = true;
    else loadFailed.value = true;
  }
  loaded.value = true;
}

async function join() {
  if (pending.value) return;
  pending.value = true;
  error.value = "";
  try {
    const joined = await api.acceptInvite(code.value);
    clearJoinCode();
    clubs.upsert(joined);
    clubs.remember(joined.slug);
    // The club's page greets the new member once (ClubHomePage's welcome).
    await router.replace({ path: `/c/${joined.slug}`, state: { joined: true } });
  } catch (err) {
    // Already a member: the invite was not used, and the club is right there.
    if (err instanceof ApiError && err.status === 409 && club.value && err.detail === "You are already in this club") {
      await leave(`/c/${club.value.slug}`, t("join.alreadyInToast", { club: club.value.name }));
      return;
    }
    error.value = err instanceof ApiError ? err.message : t("join.failed");
  } finally {
    pending.value = false;
  }
}

onMounted(load);
</script>

<template>
  <main class="auth-page join-page">
    <header class="join-top">
      <h1 class="wordmark">{{ service.name }}</h1>
      <RouterLink v-if="signedIn" class="icon-btn join-close" to="/" :aria-label="t('common.close')">
        <XMarkIcon class="join-close-icon" aria-hidden="true" />
      </RouterLink>
    </header>

    <div v-if="!loaded" class="join-skeleton" aria-hidden="true">
      <span class="skeleton join-skeleton-avatar" />
      <span class="skeleton skeleton-line" />
      <span class="skeleton join-skeleton-cover" />
    </div>

    <section v-else-if="loadFailed" class="join-dead">
      <h2 class="join-dead-title">{{ t("join.loadFailed") }}</h2>
      <button class="btn btn-primary btn-block" type="button" @click="load">{{ t("common.tryAgain") }}</button>
    </section>

    <section v-else-if="deadEnd" class="join-dead">
      <h2 class="join-dead-title">{{ deadEnd.title }}</h2>
      <p class="lede">{{ deadEnd.body }}</p>
      <RouterLink v-if="deadEnd.action" class="btn btn-ghost btn-block" :to="deadEnd.action.to">
        {{ deadEnd.action.label }}
      </RouterLink>
    </section>

    <section v-else-if="preview && club" class="join-invite">
      <div class="join-who">
        <Avatar
          v-if="inviter"
          class="join-avatar"
          :username="inviter.username"
          :src="inviter.avatar_url"
          size="lg"
        />
        <i18n-t
          :keypath="inviter ? 'join.invitedBy' : 'join.invitedTo'"
          tag="h2"
          class="join-title"
          scope="global"
        >
          <template #name>
            <span class="join-inviter">{{ inviter?.username }}</span>
          </template>
          <template #club>
            <span class="join-club">{{ club.name }}</span>
          </template>
        </i18n-t>
        <p v-if="preview.member_count != null" class="join-members fine muted nums">
          {{ tp("join.members", preview.member_count) }}
        </p>
      </div>

      <figure v-if="preview.pick" class="join-pick">
        <div class="join-cover">
          <BookCover :book="preview.pick" eager />
        </div>
        <figcaption>
          <span class="fine subtle">{{ t("join.readingNow") }}</span>
          <span class="join-pick-title">{{ preview.pick.title }}</span>
          <span v-if="preview.pick.authors" class="fine muted">{{ preview.pick.authors }}</span>
        </figcaption>
      </figure>

      <p v-if="personal && !signedIn" class="lede join-personal">{{ t("friends.personalInvite.signedOut") }}</p>
      <p v-if="error" class="error" role="alert">{{ error }}</p>

      <div class="join-actions">
        <button
          v-if="signedIn"
          class="btn btn-primary btn-block join-button"
          type="button"
          :disabled="pending"
          @click="join"
        >
          {{ pending ? t("join.joining") : t("join.joinNow") }}
        </button>
        <RouterLink v-else-if="personal" class="btn btn-primary btn-block" :to="loginLink">
          {{ t("join.signInAndJoin") }}
        </RouterLink>
        <template v-else>
          <RouterLink class="btn btn-primary btn-block" :to="signupLink">{{ t("join.createAndJoin") }}</RouterLink>
          <RouterLink class="text-btn join-login" :to="loginLink">{{ t("join.haveAccount") }}</RouterLink>
        </template>
      </div>
    </section>

    <!-- A sign-up invite (signed out: signed in, the page went home): it unlocks an account, not a club. -->
    <section v-else-if="preview" class="join-invite">
      <h2 class="join-title">{{ t("join.signupTitle") }}</h2>
      <p class="lede">{{ t("join.signupInvite") }}</p>
      <div class="join-actions">
        <RouterLink class="btn btn-primary btn-block" :to="signupLink">{{ t("auth.createAccount") }}</RouterLink>
        <RouterLink class="text-btn join-login" :to="loginLink">{{ t("join.haveAccount") }}</RouterLink>
      </div>
    </section>

    <div class="auth-locale">
      <LanguageSwitch compact />
    </div>
    <LegalLinks class="auth-legal" />
  </main>
</template>

<style scoped>
.join-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
}

.join-close {
  margin-right: calc(-1 * var(--space-2));
}

.join-close-icon {
  width: 22px;
  height: 22px;
}

/* --------------------------------------------------------- skeleton */

.join-skeleton {
  display: grid;
  justify-items: center;
  gap: var(--space-4);
  margin-top: var(--space-7);
}

.join-skeleton-avatar {
  width: 72px;
  height: 72px;
  border-radius: var(--radius-pill);
}

.join-skeleton .skeleton-line {
  width: 70%;
  height: 1.4rem;
}

.join-skeleton-cover {
  width: 132px;
  aspect-ratio: 2 / 3;
  border-radius: var(--radius-sm);
}

/* ----------------------------------------------------------- invite */

.join-invite {
  display: grid;
  justify-items: center;
  text-align: center;
  margin-top: var(--space-7);
}

.join-who {
  display: grid;
  justify-items: center;
  gap: var(--space-3);
}

.join-title {
  margin: 0;
  font-family: var(--serif);
  font-size: var(--text-2xl);
  font-weight: 600;
  line-height: var(--leading-snug);
  letter-spacing: var(--tracking-tight);
  overflow-wrap: anywhere;
  text-wrap: balance;
}

.join-inviter {
  font-weight: 700;
}

.join-club {
  color: var(--accent);
  font-weight: 700;
}

.join-members {
  margin: calc(-1 * var(--space-1)) 0 0;
}

.join-pick {
  display: grid;
  justify-items: center;
  gap: var(--space-4);
  margin: var(--space-6) 0 0;
}

.join-cover {
  width: 132px;
  transform: rotate(-2deg);
  box-shadow: var(--elev-cover);
  border-radius: var(--radius-sm);
}

.join-pick figcaption {
  display: grid;
  gap: 2px;
  max-width: 30ch;
}

.join-pick-title {
  font-family: var(--serif);
  font-size: var(--text-lg);
  font-weight: 600;
  line-height: var(--leading-snug);
  overflow-wrap: anywhere;
}

.join-personal {
  margin: var(--space-5) 0 0;
}

.join-invite > .lede {
  margin-top: var(--space-3);
}

.join-invite .error {
  margin: var(--space-4) 0 0;
}

.join-actions {
  display: grid;
  justify-items: center;
  gap: var(--space-2);
  width: 100%;
  margin-top: var(--space-6);
}

.join-login {
  font-size: var(--text-sm);
}

/* The one moment: the inviter appears, then their club's book lifts into place. */
@media (prefers-reduced-motion: no-preference) {
  .join-who {
    animation: join-arrive var(--dur-slow) var(--ease-out) both;
  }

  .join-cover {
    animation: join-lift 560ms var(--ease-out) 120ms both;
  }
}

@keyframes join-arrive {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
}

@keyframes join-lift {
  from {
    opacity: 0;
    transform: translateY(18px) rotate(0deg);
    box-shadow: none;
  }
}

/* -------------------------------------------------------- dead ends */

.join-dead {
  display: grid;
  gap: var(--space-3);
  margin-top: var(--space-7);
}

.join-dead-title {
  margin: 0;
  font-family: var(--serif);
  font-size: var(--text-2xl);
  font-weight: 600;
  line-height: var(--leading-snug);
  overflow-wrap: anywhere;
}

.join-dead .lede {
  margin: 0 0 var(--space-3);
}

.auth-locale {
  margin-top: var(--space-7);
}

.auth-legal {
  margin-top: var(--space-4);
}
</style>
