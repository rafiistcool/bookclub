<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { ApiError } from "../api/client";
import type { FriendOut } from "../api/generated";
import Avatar from "../components/Avatar.vue";
import BookCover from "../components/BookCover.vue";
import BottomSheet from "../components/BottomSheet.vue";
import InviteFriendSheet from "../components/InviteFriendSheet.vue";
import { bookPath } from "../constants";
import { formatDay } from "../i18n/dates";
import { isClubAdmin, useClubs } from "../stores/clubs";
import { isStale, useFriends } from "../stores/friends";
import { useLocale } from "../stores/locale";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";

/**
 * `/friends`: add a friend by exact username, answer requests, open or
 * decline invitations into clubs, the friends with what they are reading,
 * the requests one sent, and the blocks. Every action answers with the
 * whole lists (the friends store), so the page only renders the store.
 */
const { t } = useI18n();
const friends = useFriends();
const clubs = useClubs();
const session = useSession();
const locale = useLocale();
const toast = useToast();

const lists = computed(() => friends.lists);
/** Clubs the viewer may invite a friend into. */
const canInvite = computed(() => clubs.items.some((club) => isClubAdmin(club.role)));

const name = ref("");
const sending = ref(false);
const sendError = ref("");
/** The action running (one at a time), e.g. `accept:grace`. */
const busy = ref<string | null>(null);
/** The open sheet: a friend's options, or the confirmation of a removal or a block. */
const sheet = ref<{ username: string; mode: "menu" | "remove" | "block" } | null>(null);
/** The friend the invite sheet is open for. */
const inviting = ref<string | null>(null);

function day(iso: string): string {
  return formatDay(iso, session.user?.timezone ?? "UTC", locale.locale);
}

function failure(err: unknown): string {
  return err instanceof ApiError ? err.message : t("friends.actionFailed");
}

/**
 * Runs one action and toasts `done` (or the error). "stale": the other side
 * changed the pair first (the store loads the lists again); "failed": any
 * other error, or another action runs.
 */
async function act(key: string, run: () => Promise<unknown>, done: string): Promise<"done" | "stale" | "failed"> {
  if (busy.value) return "failed";
  busy.value = key;
  try {
    await run();
    toast.show(done);
    return "done";
  } catch (err) {
    toast.show(failure(err));
    return isStale(err) ? "stale" : "failed";
  } finally {
    busy.value = null;
  }
}

async function send() {
  const typed = name.value.trim();
  if (!typed || sending.value) return;
  // The server compares names in lower case; the answer says what became of the request.
  const key = typed.toLowerCase();
  const wasFriend = lists.value?.friends.some((row) => row.user.username === key) ?? false;
  sending.value = true;
  sendError.value = "";
  try {
    const out = await friends.send(typed);
    const friend = out.friends.find((row) => row.user.username === key);
    if (friend) {
      const copy = wasFriend ? "friends.add.alreadyFriends" : "friends.add.nowFriends";
      toast.show(t(copy, { name: friend.user.username }));
    } else {
      const shown = out.outgoing.find((row) => row.user.username === key)?.user.username ?? typed;
      toast.show(t("friends.add.sent", { name: shown }));
    }
    name.value = "";
  } catch (err) {
    sendError.value = failure(err);
  } finally {
    sending.value = false;
  }
}

function accept(username: string) {
  return act(`accept:${username}`, () => friends.accept(username), t("friends.add.nowFriends", { name: username }));
}

function decline(username: string) {
  return act(`decline:${username}`, () => friends.deleteRequest(username), t("friends.incoming.declined"));
}

function withdraw(username: string) {
  return act(`withdraw:${username}`, () => friends.deleteRequest(username), t("friends.outgoing.withdrawn"));
}

function unblock(username: string) {
  return act(`unblock:${username}`, () => friends.unblock(username), t("friends.blocked.unblocked", { name: username }));
}

function declineInvite(id: number) {
  return act(`invite:${id}`, () => friends.declineClubInvite(id), t("friends.invites.declined"));
}

function openMenu(friend: FriendOut) {
  sheet.value = { username: friend.user.username, mode: "menu" };
}

function ask(mode: "remove" | "block", username: string) {
  sheet.value = { username, mode };
}

function openInvite(username: string) {
  sheet.value = null;
  inviting.value = username;
}

async function confirm() {
  const open = sheet.value;
  if (!open || open.mode === "menu") return;
  const { username, mode } = open;
  const outcome =
    mode === "remove"
      ? await act(`remove:${username}`, () => friends.remove(username), t("friends.remove.done", { name: username }))
      : await act(`block:${username}`, () => friends.block(username), t("friends.block.done", { name: username }));
  // A stale refusal closes it too: the friend or the account is gone, and the lists follow.
  if (outcome !== "failed") sheet.value = null;
}

const sheetTitle = computed(() => {
  const open = sheet.value;
  if (!open) return "";
  if (open.mode === "remove") return t("friends.remove.title", { name: open.username });
  if (open.mode === "block") return t("friends.block.title", { name: open.username });
  return open.username;
});

onMounted(() => {
  void friends.load();
});
</script>

<template>
  <section class="friends-page">
    <div class="page-head">
      <h1>{{ t("friends.title") }}</h1>
      <p class="lede">{{ t("friends.lede") }}</p>
    </div>

    <section class="panel add-friend" aria-labelledby="add-friend">
      <h2 id="add-friend">{{ t("friends.add.heading") }}</h2>
      <form class="add-form" @submit.prevent="send">
        <label class="field">
          <span>{{ t("friends.add.label") }}</span>
          <input
            v-model="name"
            name="username"
            autocomplete="off"
            autocapitalize="none"
            spellcheck="false"
            maxlength="64"
            :disabled="sending"
          />
          <span class="field-hint">{{ t("friends.add.hint") }}</span>
        </label>
        <p v-if="sendError" class="error" role="alert">{{ sendError }}</p>
        <div class="btn-row">
          <button class="btn btn-primary btn-sm" type="submit" :disabled="sending || !name.trim()">
            {{ sending ? t("friends.add.sending") : t("friends.add.send") }}
          </button>
        </div>
      </form>
    </section>

    <div v-if="!lists && friends.failed" class="section load-failed">
      <p class="error">{{ t("friends.loadFailed") }}</p>
      <button class="btn btn-ghost btn-sm" type="button" @click="friends.load()">{{ t("common.tryAgain") }}</button>
    </div>
    <div v-else-if="!lists" class="section skeleton lists-skeleton" aria-hidden="true" />

    <template v-else>
      <section class="section" aria-labelledby="friend-requests">
        <div class="section-head">
          <h2 id="friend-requests">{{ t("friends.incoming.heading") }}</h2>
        </div>
        <p v-if="lists.incoming.length === 0" class="fine subtle empty-line">{{ t("friends.incoming.empty") }}</p>
        <ul v-else class="rows">
          <li v-for="request in lists.incoming" :key="request.user.id" class="row incoming-row">
            <span class="person">
              <Avatar :username="request.user.username" :src="request.user.avatar_url" />
              <span class="person-meta">
                <strong>{{ request.user.username }}</strong>
                <span class="finer subtle">{{ t("friends.incoming.asked", { day: day(request.created_at) }) }}</span>
              </span>
            </span>
            <span class="btn-row row-actions">
              <button class="btn btn-primary btn-sm" type="button" :disabled="!!busy" @click="accept(request.user.username)">
                {{ t("friends.incoming.accept") }}
              </button>
              <button class="btn btn-ghost btn-sm" type="button" :disabled="!!busy" @click="decline(request.user.username)">
                {{ t("friends.incoming.decline") }}
              </button>
              <button class="text-btn danger" type="button" :disabled="!!busy" @click="ask('block', request.user.username)">
                {{ t("friends.block.action") }}
              </button>
            </span>
          </li>
        </ul>
      </section>

      <section class="section" aria-labelledby="club-invitations">
        <div class="section-head">
          <h2 id="club-invitations">{{ t("friends.invites.heading") }}</h2>
        </div>
        <p v-if="lists.club_invites.length === 0" class="fine subtle empty-line">{{ t("friends.invites.empty") }}</p>
        <ul v-else class="rows">
          <li v-for="invite in lists.club_invites" :key="invite.id" class="row club-invite-row">
            <span class="person-meta">
              <strong class="invite-line">
                {{
                  invite.invited_by
                    ? t("friends.invites.from", { name: invite.invited_by, club: invite.club.name })
                    : t("friends.invites.fromGone", { club: invite.club.name })
                }}
              </strong>
              <span class="finer subtle">{{ t("friends.invites.until", { day: day(invite.expires_at) }) }}</span>
            </span>
            <span class="btn-row row-actions">
              <RouterLink class="btn btn-primary btn-sm" :to="`/join/${invite.code}`">{{ t("friends.invites.join") }}</RouterLink>
              <button class="btn btn-ghost btn-sm" type="button" :disabled="!!busy" @click="declineInvite(invite.id)">
                {{ t("friends.invites.decline") }}
              </button>
            </span>
          </li>
        </ul>
      </section>

      <section class="section" aria-labelledby="your-friends">
        <div class="section-head">
          <h2 id="your-friends">{{ t("friends.list.heading") }}</h2>
          <span v-if="lists.friends.length" class="fine subtle nums">{{ lists.friends.length }}</span>
        </div>
        <p v-if="lists.friends.length === 0" class="fine subtle empty-line">{{ t("friends.list.empty") }}</p>
        <ul v-else class="friend-list">
          <li v-for="friend in lists.friends" :key="friend.user.id" class="friend-card">
            <div class="friend-head">
              <RouterLink class="person friend-link" :to="`/u/${friend.user.username}`">
                <Avatar :username="friend.user.username" :src="friend.user.avatar_url" />
                <span class="person-meta">
                  <strong>{{ friend.user.username }}</strong>
                  <span class="finer subtle">{{ t("friends.list.since", { day: day(friend.since) }) }}</span>
                </span>
              </RouterLink>
              <button
                class="btn btn-ghost btn-sm options-btn"
                type="button"
                :aria-label="t('friends.list.optionsFor', { name: friend.user.username })"
                @click="openMenu(friend)"
              >
                {{ t("friends.list.options") }}
              </button>
            </div>
            <ul v-if="friend.currently_reading.length" class="friend-books">
              <li v-for="row in friend.currently_reading" :key="row.book.id">
                <RouterLink class="friend-book" :to="bookPath(row.book)">
                  <BookCover :book="row.book" size="xs" />
                  <span class="friend-book-title">{{ row.book.title }}</span>
                  <span v-if="row.progress != null" class="finer subtle nums">{{ row.progress }}%</span>
                </RouterLink>
              </li>
            </ul>
            <p v-else class="finer subtle">{{ t("friends.list.notReading") }}</p>
            <RouterLink
              v-if="friend.currently_reading_count > friend.currently_reading.length"
              class="fine more-link"
              :to="`/u/${friend.user.username}`"
            >
              {{ t("friends.list.more", { n: friend.currently_reading_count - friend.currently_reading.length }) }}
            </RouterLink>
          </li>
        </ul>
      </section>

      <section class="section" aria-labelledby="sent-requests">
        <div class="section-head">
          <h2 id="sent-requests">{{ t("friends.outgoing.heading") }}</h2>
        </div>
        <p v-if="lists.outgoing.length === 0" class="fine subtle empty-line">{{ t("friends.outgoing.empty") }}</p>
        <ul v-else class="rows">
          <li v-for="request in lists.outgoing" :key="request.user.id" class="row outgoing-row">
            <span class="person">
              <Avatar :username="request.user.username" :src="request.user.avatar_url" />
              <span class="person-meta">
                <strong>{{ request.user.username }}</strong>
                <span class="finer subtle">{{ t("friends.outgoing.sentOn", { day: day(request.created_at) }) }}</span>
              </span>
            </span>
            <span class="row-actions">
              <button class="text-btn" type="button" :disabled="!!busy" @click="withdraw(request.user.username)">
                {{ t("friends.outgoing.withdraw") }}
              </button>
            </span>
          </li>
        </ul>
      </section>

      <section class="section" aria-labelledby="blocked-accounts">
        <div class="section-head">
          <h2 id="blocked-accounts">{{ t("friends.blocked.heading") }}</h2>
        </div>
        <p v-if="lists.blocked.length === 0" class="fine subtle empty-line">{{ t("friends.blocked.empty") }}</p>
        <ul v-else class="rows">
          <li v-for="block in lists.blocked" :key="block.username" class="row blocked-row">
            <span class="person-meta">
              <strong>{{ block.username }}</strong>
              <span class="finer subtle">{{ t("friends.blocked.since", { day: day(block.created_at) }) }}</span>
            </span>
            <span class="row-actions">
              <button class="text-btn" type="button" :disabled="!!busy" @click="unblock(block.username)">
                {{ t("friends.blocked.unblock") }}
              </button>
            </span>
          </li>
        </ul>
        <p class="fine muted block-note">{{ t("friends.blocked.note") }}</p>
      </section>
    </template>

    <BottomSheet v-if="sheet" :key="sheet.mode" :title="sheetTitle" @close="sheet = null">
      <div v-if="sheet.mode === 'menu'" class="stack">
        <button v-if="canInvite" class="btn btn-ghost btn-block" type="button" @click="openInvite(sheet.username)">
          {{ t("friends.list.invite") }}
        </button>
        <button class="btn btn-ghost btn-block" type="button" @click="ask('remove', sheet.username)">
          {{ t("friends.list.remove") }}
        </button>
        <button class="btn btn-danger btn-block" type="button" @click="ask('block', sheet.username)">
          {{ t("friends.block.action") }}
        </button>
        <button class="btn btn-ghost btn-block" type="button" @click="sheet = null">{{ t("common.cancel") }}</button>
      </div>
      <div v-else class="stack">
        <p class="fine confirm-blurb">
          {{
            sheet.mode === "block"
              ? t("friends.block.blurb", { name: sheet.username })
              : t("friends.remove.blurb")
          }}
        </p>
        <button class="btn btn-danger btn-block" type="button" :disabled="!!busy" @click="confirm">
          {{ sheet.mode === "block" ? t("friends.block.confirm") : t("friends.remove.confirm") }}
        </button>
        <button class="btn btn-ghost btn-block" type="button" :disabled="!!busy" @click="sheet = null">
          {{ t("common.cancel") }}
        </button>
      </div>
    </BottomSheet>

    <InviteFriendSheet v-if="inviting" :username="inviting" @close="inviting = null" />
  </section>
</template>

<style scoped>
.add-friend h2 {
  font-size: var(--text-lg);
  margin-bottom: var(--space-3);
}

.add-form {
  display: grid;
  gap: var(--space-3);
  max-width: 420px;
}

.load-failed {
  display: grid;
  gap: var(--space-2);
  justify-items: start;
}

.lists-skeleton {
  height: 160px;
}

.empty-line {
  margin: 0;
}

.rows,
.friend-list,
.friend-books {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2) var(--space-3);
  padding: var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.person {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
  flex: 1 1 12rem;
}

.person-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
  flex: 1 1 12rem;
}

.person-meta strong {
  overflow-wrap: anywhere;
}

.invite-line {
  font-weight: 600;
}

.row-actions {
  flex: 0 1 auto;
  align-items: center;
}

.friend-card {
  display: grid;
  gap: var(--space-3);
  padding: var(--space-3) var(--space-4);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  min-width: 0;
}

.friend-head {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}

.friend-link {
  color: inherit;
  text-decoration: none;
}

.friend-link strong {
  font-family: var(--serif);
  font-size: var(--text-lg);
}

.options-btn {
  flex: 0 0 auto;
}

.friend-book {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  min-width: 0;
  color: inherit;
  text-decoration: none;
}

.friend-book-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--serif);
  font-weight: 600;
}

.more-link {
  justify-self: start;
  font-weight: 600;
}

.block-note {
  max-width: 62ch;
  margin-top: var(--space-3);
}

.confirm-blurb {
  margin: 0;
}

@media (min-width: 1024px) {
  .friend-list {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
