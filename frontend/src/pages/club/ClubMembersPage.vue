<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { api, ApiError } from "../../api/client";
import Avatar from "../../components/Avatar.vue";
import BookCover from "../../components/BookCover.vue";
import BottomSheet from "../../components/BottomSheet.vue";
import { formatDay } from "../../i18n/dates";
import { tp } from "../../i18n";
import { inviteState } from "../../invites";
import { clubLanding, isClubAdmin, isClubGone, isClubOwner, useClubs, useCurrentClub } from "../../stores/clubs";
import { useLocale } from "../../stores/locale";
import { useToast } from "../../stores/toast";
import { timeZoneOptions } from "../../timezone";
import type { Invite, Member } from "../../types";

const { t } = useI18n();
const locale = useLocale();
const clubs = useClubs();
const toast = useToast();
const router = useRouter();

const club = useCurrentClub();
const slug = computed(() => club.value.slug);
const canManage = computed(() => isClubAdmin(club.value.role));
const isOwner = computed(() => isClubOwner(club.value.role));

const members = ref<Member[]>([]);
const membersError = ref("");
const membersLoaded = ref(false);

const invites = ref<Invite[]>([]);
const invitesLoaded = ref(false);
const invitesError = ref("");
const creatingInvite = ref(false);
const fresh = ref<Invite | null>(null);

const managing = ref<Member | null>(null);
const confirmAction = ref<"remove" | "transfer" | null>(null);
const busy = ref(false);

const settingsName = ref(club.value.name);
const settingsZone = ref(club.value.timezone);
const zones = computed(() => timeZoneOptions(club.value.timezone));

const leaving = ref(false);
const deleting = ref(false);
const deleteConfirm = ref("");

const activeInvites = computed(() => invites.value.filter((row) => row.active));
const pastInvites = computed(() => invites.value.filter((row) => !row.active));

function failed(err: unknown, fallback: string): string {
  if (isClubGone(err)) clubs.markGone(slug.value);
  return err instanceof ApiError ? err.message : fallback;
}

async function loadMembers() {
  try {
    members.value = (await api.clubMembers(slug.value)).items;
    membersError.value = "";
  } catch (err) {
    membersError.value = failed(err, t("club.loadFailed"));
  } finally {
    membersLoaded.value = true;
  }
}

async function loadInvites() {
  if (!canManage.value) return;
  try {
    invites.value = (await api.clubInvites(slug.value)).items;
    invitesError.value = "";
  } catch (err) {
    invitesError.value = failed(err, t("invites.loadFailed"));
  } finally {
    invitesLoaded.value = true;
  }
}

/** What the viewer may do to this member: the owner manages everyone, admins remove members. */
function actionsFor(member: Member): { role: boolean; transfer: boolean; remove: boolean } {
  if (member.you || member.role === "owner") return { role: false, transfer: false, remove: false };
  if (isOwner.value) return { role: true, transfer: true, remove: true };
  if (canManage.value && member.role === "member") return { role: false, transfer: false, remove: true };
  return { role: false, transfer: false, remove: false };
}

/** The row's facts after the badges: "Since {day}", then "reading {n}" when they read anything. */
function memberFacts(member: Member): string[] {
  const parts = [t("members.joined", { day: formatDay(member.joined_at, club.value.timezone, locale.locale) })];
  if (member.currently_reading_count > 0) parts.push(t("club.currentlyReading", { n: member.currently_reading_count }));
  return parts;
}

function openManage(member: Member) {
  managing.value = member;
  confirmAction.value = null;
}

async function setRole(member: Member, role: "admin" | "member") {
  busy.value = true;
  try {
    const updated = await api.changeMemberRole(slug.value, member.username, role);
    members.value = members.value.map((row) => (row.username === updated.username ? updated : row));
    toast.show(t(role === "admin" ? "members.madeAdmin" : "members.madeMember", { name: member.username }));
    managing.value = null;
    await loadMembers();
  } catch (err) {
    toast.show(failed(err, t("members.actionFailed")));
  } finally {
    busy.value = false;
  }
}

async function remove(member: Member) {
  if (confirmAction.value !== "remove") {
    confirmAction.value = "remove";
    return;
  }
  busy.value = true;
  try {
    await api.removeMember(slug.value, member.username);
    toast.show(t("members.removed", { name: member.username }));
    managing.value = null;
    await Promise.all([loadMembers(), loadInvites()]);
    clubs.upsert({ ...club.value, member_count: members.value.length });
  } catch (err) {
    toast.show(failed(err, t("members.actionFailed")));
  } finally {
    busy.value = false;
  }
}

async function transfer(member: Member) {
  if (confirmAction.value !== "transfer") {
    confirmAction.value = "transfer";
    return;
  }
  busy.value = true;
  try {
    const updated = await api.transferClub(slug.value, member.username);
    clubs.upsert(updated);
    toast.show(t("members.transferred", { name: member.username }));
    managing.value = null;
    await loadMembers();
  } catch (err) {
    toast.show(failed(err, t("members.actionFailed")));
  } finally {
    busy.value = false;
  }
}

async function createInvite(kind: "single_use" | "link") {
  creatingInvite.value = true;
  try {
    const invite = await api.createInvite(slug.value, { kind });
    invites.value = [invite, ...invites.value];
    fresh.value = invite;
  } catch (err) {
    toast.show(failed(err, t("invites.createFailed")));
  } finally {
    creatingInvite.value = false;
  }
}

async function revoke(invite: Invite) {
  try {
    await api.revokeInvite(slug.value, invite.id);
    invites.value = invites.value.map((row) =>
      row.id === invite.id ? { ...row, revoked: true, active: false } : row,
    );
    if (fresh.value?.id === invite.id) fresh.value = null;
    toast.show(t("invites.revoked"));
  } catch (err) {
    toast.show(failed(err, t("invites.revokeFailed")));
  }
}

async function copy(invite: Invite) {
  try {
    await navigator.clipboard.writeText(invite.url);
    toast.show(t("invites.copied"));
  } catch {
    toast.show(t("invites.copyFailed"));
  }
}

const canShare = typeof navigator !== "undefined" && typeof navigator.share === "function";

async function share(invite: Invite) {
  try {
    await navigator.share({ title: club.value.name, text: t("invites.shareText", { club: club.value.name }), url: invite.url });
  } catch {
    /* Dismissed. */
  }
}

/** A friend invite names its friend to its creator only; other admins see that it is personal. */
function inviteKind(invite: Invite): string {
  if (invite.personal) {
    return invite.invitee ? t("invites.personalFor", { name: invite.invitee }) : t("invites.personal");
  }
  return invite.max_uses === 1 ? t("invites.singleUse") : t("invites.link");
}

function inviteFacts(invite: Invite): string[] {
  const parts: string[] = [];
  parts.push(inviteKind(invite));
  if (invite.max_uses == null || invite.max_uses > 1) parts.push(tp("invites.uses", invite.use_count));
  if (invite.expires_at) {
    const day = formatDay(invite.expires_at, club.value.timezone, locale.locale);
    parts.push(invite.active ? t("invites.expires", { day }) : t("invites.expired", { day }));
  }
  if (invite.created_by) parts.push(t("invites.by", { name: invite.created_by }));
  return parts;
}

async function saveSettings() {
  const name = settingsName.value.trim();
  if (!name) return;
  busy.value = true;
  try {
    const updated = await api.updateClub(slug.value, { name, timezone: settingsZone.value });
    clubs.upsert(updated);
    settingsName.value = updated.name;
    settingsZone.value = updated.timezone;
    toast.show(t("clubSettings.saved"));
  } catch (err) {
    toast.show(failed(err, t("clubSettings.saveFailed")));
  } finally {
    busy.value = false;
  }
}

async function leave() {
  if (!leaving.value) {
    leaving.value = true;
    return;
  }
  busy.value = true;
  try {
    await api.leaveClub(slug.value);
    const name = club.value.name;
    clubs.forget(slug.value);
    await clubs.refresh().catch(() => undefined);
    toast.show(t("clubSettings.left", { club: name }));
    await router.replace(clubLanding(clubs.items, clubs.lastSlug));
  } catch (err) {
    toast.show(failed(err, t("clubSettings.leaveFailed")));
    leaving.value = false;
  } finally {
    busy.value = false;
  }
}

async function deleteClub() {
  if (deleteConfirm.value.trim() !== club.value.name) return;
  busy.value = true;
  try {
    await api.deleteClub(slug.value);
    const name = club.value.name;
    clubs.forget(slug.value);
    await clubs.refresh().catch(() => undefined);
    toast.show(t("clubSettings.deleted", { club: name }));
    await router.replace(clubLanding(clubs.items, clubs.lastSlug));
  } catch (err) {
    toast.show(failed(err, t("clubSettings.deleteFailed")));
  } finally {
    busy.value = false;
  }
}

onMounted(() => {
  void loadMembers();
  void loadInvites();
});
</script>

<template>
  <div class="members-page">
    <section aria-labelledby="members">
      <div class="section-head">
        <h2 id="members">{{ t("club.members") }}</h2>
        <span v-if="membersLoaded" class="fine subtle nums">{{ members.length }}</span>
      </div>

      <p v-if="membersError" class="error">{{ membersError }}</p>
      <div v-else-if="!membersLoaded" class="skeleton member-skeleton" aria-hidden="true" />

      <template v-else>
        <ul class="member-list">
          <li v-for="member in members" :key="member.username" class="member-row" :class="{ 'is-you': member.you }">
            <RouterLink class="member-link" :to="member.you ? '/me' : `/u/${member.username}`">
              <Avatar :username="member.username" :src="member.avatar_url" />
              <span class="member-meta">
                <strong>{{ member.username }}</strong>
                <span class="finer subtle member-line">
                  <span v-if="member.role !== 'member'" class="badge role-badge">{{ t(`roles.${member.role}`) }}</span>
                  <span v-if="member.you" class="badge you-badge">{{ t("club.thatsYou") }}</span>
                  <span class="member-facts facts nums">
                    <span v-for="fact in memberFacts(member)" :key="fact" class="fact">{{ fact }}</span>
                  </span>
                </span>
              </span>
              <span v-if="member.currently_reading_preview.length" class="preview-row">
                <BookCover
                  v-for="book in member.currently_reading_preview"
                  :key="book.id"
                  :book="book"
                  size="xs"
                />
              </span>
            </RouterLink>
            <button
              v-if="Object.values(actionsFor(member)).some(Boolean)"
              class="btn btn-ghost btn-sm manage-btn"
              type="button"
              @click="openManage(member)"
            >
              {{ t("members.manage") }}
            </button>
          </li>
        </ul>

        <div v-if="members.length === 1" class="empty only-you">
          <h3>{{ t("club.onlyYouTitle") }}</h3>
          <p>{{ canManage ? t("club.onlyYouBody") : t("club.onlyYouMemberBody") }}</p>
        </div>
      </template>
    </section>

    <section v-if="canManage" class="section invites" aria-labelledby="invites">
      <div class="section-head">
        <h2 id="invites">{{ t("invites.heading") }}</h2>
      </div>
      <p class="fine muted invites-blurb">{{ t("invites.blurb") }}</p>
      <div class="btn-row invite-buttons">
        <button class="btn btn-primary btn-sm" type="button" :disabled="creatingInvite" @click="createInvite('single_use')">
          {{ t("invites.newSingle") }}
        </button>
        <button class="btn btn-ghost btn-sm" type="button" :disabled="creatingInvite" @click="createInvite('link')">
          {{ t("invites.newLink") }}
        </button>
      </div>

      <div v-if="fresh" class="panel fresh-invite">
        <p class="fine">{{ fresh.max_uses === 1 ? t("invites.freshSingle") : t("invites.freshLink") }}</p>
        <code class="invite-url">{{ fresh.url }}</code>
        <div class="btn-row">
          <button class="btn btn-primary btn-sm" type="button" @click="copy(fresh)">{{ t("invites.copyLink") }}</button>
          <button v-if="canShare" class="btn btn-ghost btn-sm" type="button" @click="share(fresh)">{{ t("invites.share") }}</button>
        </div>
      </div>

      <p v-if="invitesError" class="error">{{ invitesError }}</p>
      <div v-else-if="!invitesLoaded" class="skeleton invite-skeleton" aria-hidden="true" />
      <template v-else>
        <h3 class="invite-subhead">{{ t("invites.active") }}</h3>
        <p v-if="activeInvites.length === 0" class="fine subtle">{{ t("invites.noneActive") }}</p>
        <ul v-else class="invite-list">
          <li v-for="invite in activeInvites" :key="invite.id" class="invite-row">
            <span class="invite-meta">
              <code>{{ invite.code }}</code>
              <span class="finer subtle facts">
                <span v-for="fact in inviteFacts(invite)" :key="fact" class="fact">{{ fact }}</span>
              </span>
              <span v-if="invite.used_by.length" class="finer subtle">
                {{ t("invites.usedBy", { names: invite.used_by.join(", ") }) }}
              </span>
            </span>
            <span class="invite-actions">
              <button class="text-btn" type="button" @click="copy(invite)">{{ t("common.copy") }}</button>
              <button class="text-btn danger" type="button" @click="revoke(invite)">{{ t("invites.revoke") }}</button>
            </span>
          </li>
        </ul>
        <details v-if="pastInvites.length" class="past-invites">
          <summary class="fine">{{ tp("invites.past", pastInvites.length) }}</summary>
          <ul class="invite-list">
            <li v-for="invite in pastInvites" :key="invite.id" class="invite-row past">
              <span class="invite-meta">
                <code>{{ invite.code }}</code>
                <span class="finer subtle facts">
                  <span class="badge">{{ t(`invites.state.${inviteState(invite)}`) }}</span>
                  <span v-for="fact in inviteFacts(invite)" :key="fact" class="fact">{{ fact }}</span>
                </span>
                <span v-if="invite.used_by.length" class="finer subtle">
                  {{ t("invites.usedBy", { names: invite.used_by.join(", ") }) }}
                </span>
              </span>
            </li>
          </ul>
        </details>
      </template>
    </section>

    <section v-if="canManage" class="section" aria-labelledby="club-settings">
      <div class="section-head">
        <h2 id="club-settings">{{ t("clubSettings.heading") }}</h2>
      </div>
      <form class="stack settings-form" @submit.prevent="saveSettings">
        <label class="field">
          <span>{{ t("clubSettings.name") }}</span>
          <input v-model="settingsName" maxlength="80" required />
        </label>
        <label class="field">
          <span>{{ t("clubSettings.timezone") }}</span>
          <select v-model="settingsZone">
            <option v-for="zone in zones" :key="zone" :value="zone">{{ zone }}</option>
          </select>
          <span class="field-hint">{{ t("clubSettings.timezoneHint") }}</span>
        </label>
        <button class="btn btn-primary btn-sm" type="submit" :disabled="busy || !settingsName.trim()">
          {{ t("common.save") }}
        </button>
      </form>
    </section>

    <section class="section danger-zone" aria-labelledby="leave">
      <div class="section-head">
        <h2 id="leave">{{ isOwner ? t("clubSettings.ownerHeading") : t("clubSettings.leaveHeading") }}</h2>
      </div>
      <template v-if="!isOwner">
        <p class="fine muted">{{ t("clubSettings.leaveBlurb") }}</p>
        <div class="btn-row">
          <button class="btn btn-danger btn-sm" type="button" :disabled="busy" @click="leave">
            {{ leaving ? t("clubSettings.leaveConfirm") : t("clubSettings.leave") }}
          </button>
          <button v-if="leaving" class="btn btn-ghost btn-sm" type="button" @click="leaving = false">
            {{ t("common.cancel") }}
          </button>
        </div>
      </template>
      <template v-else>
        <p class="fine muted">{{ t("clubSettings.ownerBlurb") }}</p>
        <div v-if="!deleting" class="btn-row">
          <button class="btn btn-danger btn-sm" type="button" @click="deleting = true">
            {{ t("clubSettings.delete") }}
          </button>
        </div>
        <form v-else class="panel delete-form" @submit.prevent="deleteClub">
          <p class="fine">{{ t("clubSettings.deleteBlurb", { club: club.name }) }}</p>
          <label class="field">
            <span>{{ t("clubSettings.deleteType", { club: club.name }) }}</span>
            <input v-model="deleteConfirm" autocomplete="off" spellcheck="false" />
          </label>
          <div class="btn-row">
            <button
              class="btn btn-danger btn-sm"
              type="submit"
              :disabled="busy || deleteConfirm.trim() !== club.name"
            >
              {{ t("clubSettings.deleteFinal") }}
            </button>
            <button class="btn btn-ghost btn-sm" type="button" @click="deleting = false; deleteConfirm = ''">
              {{ t("common.cancel") }}
            </button>
          </div>
        </form>
      </template>
    </section>

    <BottomSheet v-if="managing" :title="t('members.manageTitle', { name: managing.username })" @close="managing = null">
      <p class="fine subtle manage-role">{{ t("members.currentRole", { role: t(`roles.${managing.role}`) }) }}</p>
      <div class="stack">
        <template v-if="actionsFor(managing).role">
          <button
            v-if="managing.role === 'member'"
            class="btn btn-ghost btn-block"
            type="button"
            :disabled="busy"
            @click="setRole(managing, 'admin')"
          >
            {{ t("members.makeAdmin") }}
          </button>
          <button
            v-else
            class="btn btn-ghost btn-block"
            type="button"
            :disabled="busy"
            @click="setRole(managing, 'member')"
          >
            {{ t("members.makeMember") }}
          </button>
        </template>
        <template v-if="actionsFor(managing).transfer">
          <p v-if="confirmAction === 'transfer'" class="fine">{{ t("members.transferBlurb", { name: managing.username }) }}</p>
          <button class="btn btn-ghost btn-block" type="button" :disabled="busy" @click="transfer(managing)">
            {{ confirmAction === "transfer" ? t("members.transferConfirm") : t("members.transfer") }}
          </button>
        </template>
        <template v-if="actionsFor(managing).remove">
          <p v-if="confirmAction === 'remove'" class="fine">{{ t("members.removeBlurb", { name: managing.username }) }}</p>
          <button class="btn btn-danger btn-block" type="button" :disabled="busy" @click="remove(managing)">
            {{ confirmAction === "remove" ? t("members.removeConfirm") : t("members.remove") }}
          </button>
        </template>
        <button class="btn btn-ghost btn-block" type="button" @click="managing = null">
          {{ t("common.cancel") }}
        </button>
      </div>
    </BottomSheet>
  </div>
</template>

<style scoped>
.member-skeleton,
.invite-skeleton {
  height: 130px;
}

/* Light rows split by hairlines. The viewer's own row is marked by its
   "You" badge only. */
.member-list,
.invite-list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.member-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  padding: var(--space-3) 0;
}

.member-row + .member-row,
.invite-row + .invite-row {
  border-top: 1px solid var(--border);
}

.member-link {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2) var(--space-3);
  min-height: var(--tap);
  color: inherit;
  text-decoration: none;
}

@media (hover: hover) {
  .member-link:hover strong {
    color: var(--accent);
  }
}

.member-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
  flex: 1 1 7rem;
}

.member-meta strong {
  font-family: var(--serif);
  font-size: var(--text-lg);
  overflow-wrap: anywhere;
}

/* One line: the badges, then "Since {day} · reading {n}". It wraps on a narrow row. */
.member-line {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 2px var(--space-1);
}

/* Facts joined by dots. Each fact wraps as a unit, and a dot stays at the
   end of its line instead of starting the next one. */
.facts {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 2px 0.3em;
}

.fact:has(+ .fact)::after {
  content: " ·";
}

/* The covers a member reads always sit on their own line under the name,
   lined up with it past the avatar, so every row has the same shape. */
.preview-row {
  display: flex;
  gap: var(--space-1);
  flex: 0 0 100%;
  padding-inline-start: calc(34px + var(--space-3));
}

.manage-btn {
  flex: 0 0 auto;
}

.only-you {
  margin-top: var(--space-3);
}

.invites-blurb {
  max-width: 62ch;
  margin-bottom: var(--space-3);
}

.fresh-invite {
  display: grid;
  gap: var(--space-2);
  margin-top: var(--space-3);
}

.invite-url {
  overflow-wrap: anywhere;
  font-size: var(--text-sm);
}

.invite-subhead {
  font-size: var(--text-md);
  margin: var(--space-5) 0 var(--space-2);
}

/* One width for both buttons: stacked on a phone, side by side with room. */
.invite-buttons {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(13rem, 1fr));
  max-width: 480px;
}

.invite-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-2) 0;
}

.invite-row.past {
  opacity: 0.75;
}

.invite-meta {
  display: grid;
  gap: 2px;
  /* What the actions leave, but never less than the code. */
  flex: 1 1 0;
  min-width: min-content;
}

.invite-meta .finer {
  overflow-wrap: anywhere;
}

.invite-meta code {
  font-size: var(--text-md);
  letter-spacing: 0.05em;
}

/*
 * The actions wrap onto two lines when they do not fit beside the code: at
 * 360 px, German "Kopieren" and "Zurückziehen" are wider than what is left.
 * The padding only widens the targets; the last text lines up with the edge.
 */
.invite-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  flex: 0 1 auto;
  margin-inline-end: calc(-1 * var(--space-2));
}

.past-invites {
  margin-top: var(--space-3);
}

/* display: flex drops the marker, so a chevron says the line opens. */
.past-invites summary {
  cursor: pointer;
  min-height: var(--tap);
  display: flex;
  align-items: center;
  gap: var(--space-2);
  list-style: none;
  color: var(--text-muted);
  font-weight: 600;
}

.past-invites summary::-webkit-details-marker {
  display: none;
}

.past-invites summary::after {
  content: "";
  width: 7px;
  height: 7px;
  border-right: 2px solid currentColor;
  border-bottom: 2px solid currentColor;
  transform: translateY(-2px) rotate(45deg);
}

.past-invites[open] summary::after {
  transform: translateY(2px) rotate(-135deg);
}

@media (prefers-reduced-motion: no-preference) {
  .past-invites summary::after {
    transition: transform var(--dur-fast) var(--ease-out);
  }
}

@media (hover: hover) {
  .past-invites summary:hover {
    color: var(--text);
  }
}

.settings-form,
.delete-form {
  max-width: 480px;
}

.settings-form > .btn {
  justify-self: start;
}

.danger-zone .btn-row {
  margin-top: var(--space-3);
}

.manage-role {
  margin-bottom: var(--space-3);
}

/* Two columns of rows: the first row of each column has no hairline above it. */
@media (min-width: 1024px) {
  .member-list {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    column-gap: var(--space-6);
  }

  .member-row:nth-child(2) {
    border-top: 0;
  }
}
</style>
