<script setup lang="ts">
import { CameraIcon, ChevronRightIcon, Cog6ToothIcon, PhotoIcon, TrashIcon } from "@heroicons/vue/24/outline";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import type { PersonalStatsOut } from "../api/generated";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import BottomSheet from "../components/BottomSheet.vue";
import FavoritePortrait from "../components/FavoritePortrait.vue";
import FavoritesEditor from "../components/FavoritesEditor.vue";
import { tp } from "../i18n";
import { useClubs } from "../stores/clubs";
import { useFriends } from "../stores/friends";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import { formatCount } from "../stats";
import type { Favorite } from "../types";

/**
 * `/me`, the Profile tab: the reader as others meet them (picture, name,
 * favourites) and the ways on from there: the year, the shelf, the friends
 * (with what waits), the clubs. The picture and the favourites are edited
 * here, in sheets; the gear leads to Settings. Everything comes from calls
 * that exist: the shelf (count and favourites), the year's stats, the clubs
 * (each one's current book), and the friends lists the shell keeps.
 */
const { t } = useI18n();
const session = useSession();
const friends = useFriends();
const clubs = useClubs();
const locale = useLocale();
const toast = useToast();

const username = computed(() => session.user?.username ?? "");
const avatarUrl = computed(() => session.user?.avatar_url ?? null);

const shelfCount = ref<number | null>(null);
const favorites = ref<Favorite[]>([]);
const shelfLoaded = ref(false);
const shelfFailed = ref(false);
const stats = ref<PersonalStatsOut | null>(null);

/* ------------------------------------------------------------ the numbers */

const friendCount = computed(() => friends.lists?.friends.length ?? null);

function shown(n: number | null): string {
  return n == null ? "–" : formatCount(n, locale.locale);
}

const numbers = computed(() => [
  {
    key: "read",
    to: "/stats",
    value: stats.value ? stats.value.finished : null,
    label: t("profile.readIn", { year: stats.value?.year ?? new Date().getFullYear() }),
  },
  { key: "shelf", to: "/shelf", value: shelfCount.value, label: t("profile.onShelf") },
  { key: "friends", to: "/friends", value: friendCount.value, label: t("friends.title") },
]);

/* ------------------------------------------------------------ the friends */

/** The faces in the pile: the first few friends. */
const faces = computed(() => friends.lists?.friends.slice(0, 5) ?? []);
const readingNow = computed(() => friends.lists?.friends.filter((row) => row.currently_reading_count > 0).length ?? 0);
/** No friend and nothing waiting: an invitation to add one instead of the pile. */
const noFriends = computed(() => friends.lists != null && faces.value.length === 0 && friends.badge === 0);

/* -------------------------------------------------------------- the clubs */

const clubRows = computed(() =>
  clubs.items.map((club) => ({
    ...club,
    book: clubs.list.find((row) => row.slug === club.slug)?.current_pick?.book ?? null,
  })),
);

/* ------------------------------------------------------------ the picture */

const photoSheet = ref(false);
const avatarBusy = ref(false);

async function onAvatarFile(event: Event) {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  photoSheet.value = false;
  if (!file || avatarBusy.value) return;
  if (file.size > 50 * 1024 * 1024) {
    toast.show(t("errors.pictureTooLarge"));
    return;
  }
  avatarBusy.value = true;
  try {
    session.user = await api.uploadAvatar(file);
    toast.show(t("profile.photoSaved"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("profile.photoFailed"));
  } finally {
    avatarBusy.value = false;
  }
}

async function removeAvatar() {
  photoSheet.value = false;
  if (avatarBusy.value) return;
  avatarBusy.value = true;
  try {
    session.user = await api.deleteAvatar();
    toast.show(t("profile.photoRemoved"));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("profile.photoFailed"));
  } finally {
    avatarBusy.value = false;
  }
}

/* --------------------------------------------------------- the favourites */

const favoritesSheet = ref(false);
const favoritesBusy = ref(false);

async function replaceFavorites(workIds: number[]) {
  if (favoritesBusy.value) return;
  favoritesBusy.value = true;
  try {
    favorites.value = (await api.replaceFavorites(workIds)).items;
    toast.show(workIds.length ? t("favorites.saved") : t("favorites.cleared"));
    if (favorites.value.length === 0) favoritesSheet.value = false;
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("favorites.saveFailed"));
  } finally {
    favoritesBusy.value = false;
  }
}

/* ---------------------------------------------------------------- loading */

async function loadShelf() {
  shelfFailed.value = false;
  try {
    const shelf = await api.myShelf();
    shelfCount.value = shelf.items.length;
    favorites.value = shelf.favorites;
  } catch {
    // The shelf count stays a dash. The favourites say so rather than
    // claim there are none.
    shelfFailed.value = true;
  } finally {
    shelfLoaded.value = true;
  }
}

async function loadStats() {
  try {
    stats.value = await api.personalStats({});
  } catch {
    stats.value = null;
  }
}

async function loadClubs() {
  if (!clubs.hasClubs) return;
  try {
    await clubs.loadList();
  } catch {
    // The rows stay, without their books.
  }
}

onMounted(() => {
  void loadShelf();
  void loadStats();
  void loadClubs();
});
</script>

<template>
  <section class="profile">
    <div class="profile-head">
      <div class="profile-picture" :class="{ busy: avatarBusy }">
        <Avatar :username="username" :src="avatarUrl" size="lg" />
        <button
          class="photo-badge"
          type="button"
          :aria-label="t('profile.photoChange')"
          :disabled="avatarBusy"
          @click="photoSheet = true"
        >
          <span class="photo-badge-dot"><CameraIcon aria-hidden="true" /></span>
        </button>
      </div>
      <h1 class="profile-name">{{ username }}</h1>
      <RouterLink class="icon-btn profile-settings" to="/settings" :aria-label="t('nav.settings')">
        <Cog6ToothIcon aria-hidden="true" />
      </RouterLink>
    </div>

    <ul class="numbers">
      <li v-for="(number, index) in numbers" :key="number.key">
        <RouterLink class="number" :class="`number-${number.key}`" :to="number.to">
          <span v-if="number.value != null" class="number-value nums" :style="{ '--i': index }">
            {{ shown(number.value) }}
          </span>
          <span v-else class="number-value nums pending" aria-hidden="true">–</span>
          <span class="number-label">{{ number.label }}</span>
        </RouterLink>
      </li>
    </ul>

    <section class="section" aria-labelledby="profile-favorites">
      <div class="section-head">
        <h2 id="profile-favorites">{{ t("favorites.heading") }}</h2>
        <button
          v-if="favorites.length"
          class="text-btn edit-favorites"
          type="button"
          @click="favoritesSheet = true"
        >
          {{ t("common.edit") }}
        </button>
      </div>
      <div v-if="!shelfLoaded" class="favorites-skeleton" aria-hidden="true">
        <span v-for="n in 3" :key="n" class="skeleton favorite-skeleton" />
      </div>
      <p v-else-if="shelfFailed" class="fine subtle favorites-failed" role="alert">
        {{ t("shelf.loadFailed") }}
        <button class="text-btn fine" type="button" @click="loadShelf">{{ t("common.tryAgain") }}</button>
      </p>
      <FavoritePortrait v-else :items="favorites" :empty-hint="t('favorites.emptyOwn')" />
    </section>

    <RouterLink v-if="noFriends" class="light-row invite-row friends-invite" to="/friends?add=1">
      <span class="light-row-label">{{ t("friends.homeAdd") }}</span>
      <ChevronRightIcon class="chevron" aria-hidden="true" />
    </RouterLink>
    <RouterLink v-else-if="friends.lists" class="light-row friends-row" to="/friends">
      <span v-if="faces.length" class="faces" aria-hidden="true">
        <Avatar v-for="row in faces" :key="row.user.id" :username="row.user.username" :src="row.user.avatar_url" size="sm" />
      </span>
      <span class="light-row-text">
        <span class="light-row-label">{{ t("profile.yourFriends") }}</span>
        <span v-if="readingNow" class="finer subtle">{{ tp("profile.friendsReading", readingNow) }}</span>
      </span>
      <span v-if="friends.badge" class="waiting nums">
        <span aria-hidden="true">{{ friends.badge }}</span>
        <span class="visually-hidden">{{ tp("friends.waiting", friends.badge) }}</span>
      </span>
      <ChevronRightIcon class="chevron" aria-hidden="true" />
    </RouterLink>

    <section class="section" aria-labelledby="profile-clubs">
      <div class="section-head">
        <h2 id="profile-clubs">{{ t("profile.clubs") }}</h2>
      </div>
      <ul v-if="clubRows.length" class="club-rows">
        <li v-for="club in clubRows" :key="club.slug">
          <RouterLink class="light-row club-row" :to="`/c/${club.slug}`">
            <BookCover v-if="club.book" :book="club.book" size="xs" />
            <span v-else class="no-book" aria-hidden="true" />
            <span class="light-row-text">
              <span class="club-name">{{ club.name }}</span>
              <span class="finer subtle">{{ tp("club.memberCount", club.member_count) }}</span>
            </span>
            <ChevronRightIcon class="chevron" aria-hidden="true" />
          </RouterLink>
        </li>
      </ul>
      <ul v-else class="club-rows clubs-invite">
        <li>
          <RouterLink class="light-row invite-row" to="/clubs/new">
            <span class="light-row-label">{{ t("clubs.create") }}</span>
            <ChevronRightIcon class="chevron" aria-hidden="true" />
          </RouterLink>
        </li>
        <li>
          <RouterLink class="light-row invite-row" to="/clubs">
            <span class="light-row-label">{{ t("home.joinWithCode") }}</span>
            <ChevronRightIcon class="chevron" aria-hidden="true" />
          </RouterLink>
        </li>
      </ul>
    </section>

    <BottomSheet v-if="photoSheet" :title="t('profile.photo')" @close="photoSheet = false">
      <ul class="sheet-rows">
        <li>
          <label class="sheet-row">
            <input
              class="visually-hidden"
              type="file"
              accept="image/jpeg,image/png,image/webp,image/gif"
              :disabled="avatarBusy"
              @change="onAvatarFile"
            />
            <PhotoIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("profile.photoChoose") }}</span>
          </label>
        </li>
        <li v-if="avatarUrl">
          <button class="sheet-row danger" type="button" :disabled="avatarBusy" @click="removeAvatar">
            <TrashIcon class="sheet-icon" aria-hidden="true" />
            <span class="sheet-label">{{ t("profile.photoRemove") }}</span>
          </button>
        </li>
      </ul>
    </BottomSheet>

    <BottomSheet v-if="favoritesSheet" :title="t('favorites.heading')" @close="favoritesSheet = false">
      <p class="fine muted sheet-blurb">{{ t("favorites.settingsBlurb") }}</p>
      <FavoritesEditor :items="favorites" :busy="favoritesBusy" @replace="replaceFavorites" />
    </BottomSheet>
  </section>
</template>

<style scoped>
/* ------------------------------------------------------------------ head */

.profile-head {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--space-4);
  padding-right: var(--tap);
  margin-bottom: var(--space-5);
}

.profile-picture {
  position: relative;
  flex: none;
  animation: picture-in var(--dur-slow) var(--ease-out) both;
}

.profile-picture :deep(.avatar) {
  width: 84px;
  height: 84px;
  font-size: var(--text-3xl);
  transition: opacity var(--dur-base) var(--ease);
}

.profile-picture.busy :deep(.avatar) {
  opacity: 0.5;
}

/* A 44px target; the visible badge is the small round button in its corner. */
.photo-badge {
  position: absolute;
  right: -12px;
  bottom: -12px;
  display: grid;
  place-items: center;
  width: var(--tap);
  height: var(--tap);
  padding: 0;
  border: 0;
  background: transparent;
  color: var(--text);
  transition: transform var(--dur-fast) var(--ease-out);
}

.photo-badge:active:not(:disabled) {
  transform: scale(0.9);
  transition-duration: var(--dur-press);
}

.photo-badge-dot {
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: var(--radius-pill);
  background: var(--surface);
  border: 1px solid var(--border-strong);
  box-shadow: 0 0 0 2px var(--bg);
}

.photo-badge-dot svg {
  width: 17px;
  height: 17px;
}

.profile-name {
  min-width: 0;
  font-size: clamp(1.5rem, 6.7vw, 2rem);
  overflow-wrap: anywhere;
}

.profile-settings {
  position: absolute;
  top: 50%;
  right: calc(-1 * var(--space-2));
  margin-top: calc(var(--tap) / -2);
  text-decoration: none;
}

.profile-settings svg {
  width: 24px;
  height: 24px;
}

/* On a wide screen the sidebar's gear sits right beside this one. */
@media (min-width: 1024px) {
  .profile-head {
    padding-right: 0;
  }

  .profile-settings {
    display: none;
  }
}

/* --------------------------------------------------------------- numbers */

.numbers {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  margin: 0;
  padding: 0;
  list-style: none;
  border-block: 1px solid var(--border);
}

.numbers li + li {
  border-left: 1px solid var(--border);
}

.number {
  display: grid;
  justify-items: center;
  align-content: center;
  gap: 2px;
  min-height: 72px;
  padding: var(--space-3) var(--space-1);
  color: inherit;
  text-align: center;
  text-decoration: none;
  transition: transform var(--dur-fast) var(--ease-out);
}

.number:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

.number-value {
  font-family: var(--serif);
  font-size: var(--text-2xl);
  font-weight: 600;
  line-height: var(--leading-tight);
  animation: number-in var(--dur-slow) var(--ease-out) both;
  animation-delay: calc(var(--i, 0) * 60ms);
}

.number-value.pending {
  color: var(--text-subtle);
  animation: none;
}

.number-label {
  color: var(--text-muted);
  font-size: var(--text-xs);
  font-weight: 600;
}

@media (hover: hover) {
  .number:hover .number-label {
    color: var(--accent);
  }
}

/* The page's one moment: the picture and the numbers settle in once, as they arrive. */
@keyframes picture-in {
  from {
    opacity: 0;
    transform: scale(0.92);
  }
}

@keyframes number-in {
  from {
    opacity: 0;
    transform: translateY(6px);
  }
}

@media (prefers-reduced-motion: reduce) {
  .profile-picture,
  .number-value {
    animation: none;
  }
}

/* ------------------------------------------------------------ favourites */

/* "Edit" is a section-head action (components.css): flush right, small,
   with its 44px hit area outside the text. */

/* Three covers the size of the portrait's, under the room of its ranks. */
.favorites-skeleton {
  display: flex;
  gap: var(--space-3);
  padding-top: calc(var(--text-2xs) * 1.5 + var(--space-1));
  margin-bottom: var(--space-5);
}

.favorite-skeleton {
  width: 72px;
  aspect-ratio: 2 / 3;
}

.sheet-blurb {
  margin: 0 0 var(--space-3);
}

/* ------------------------------------------------------------ light rows */

.light-row {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-height: 56px;
  padding: var(--space-2) 0;
  color: inherit;
  text-decoration: none;
  transition: transform var(--dur-fast) var(--ease-out);
}

.light-row:active {
  transform: scale(var(--press-scale));
  transition-duration: var(--dur-press);
}

.friends-row,
.invite-row.friends-invite {
  margin-top: var(--space-2);
  border-block: 1px solid var(--border);
}

.invite-row .light-row-label {
  color: var(--accent);
}

.light-row-text {
  flex: 1;
  display: grid;
  gap: 2px;
  min-width: 0;
}

.light-row-label {
  flex: 1;
  font-weight: 600;
}

@media (hover: hover) {
  .light-row:hover .chevron {
    color: var(--text);
  }
}

.chevron {
  flex: none;
  width: 18px;
  height: 18px;
  color: var(--text-subtle);
}

.faces {
  display: flex;
  flex: none;
}

.faces :deep(.avatar) {
  width: 30px;
  height: 30px;
  box-shadow: 0 0 0 2px var(--bg);
}

.faces :deep(.avatar + .avatar) {
  margin-left: -8px;
}

.waiting {
  flex: none;
  min-width: 1.5rem;
  padding: 0 var(--space-2);
  border-radius: var(--radius-pill);
  background: var(--accent);
  color: var(--accent-ink);
  font-size: var(--text-xs);
  font-weight: 700;
  line-height: 1.5rem;
  text-align: center;
}

.club-rows {
  margin: 0;
  padding: 0;
  list-style: none;
  border-top: 1px solid var(--border);
}

.club-rows li {
  border-bottom: 1px solid var(--border);
}

.club-row {
  min-height: 64px;
}

.club-row :deep(.cover),
.no-book {
  flex: none;
}

.no-book {
  width: 32px;
  height: 48px;
  border-radius: 3px;
  background: var(--surface-2);
  border: 1px solid var(--border);
}

.club-name {
  font-family: var(--serif);
  font-size: var(--text-lg);
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}


@media (min-width: 1024px) {
  .profile {
    max-width: 720px;
  }
}
</style>
