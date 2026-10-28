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

function inviteLine(invite: Invite): string {
  const parts: string[] = [];
  parts.push(invite.max_uses === 1 ? t("invites.singleUse") : t("invites.link"));
  if (invite.max_uses == null || invite.max_uses > 1) parts.push(tp("invites.uses", invite.use_count));
  if (invite.expires_at) {
    const day = formatDay(invite.expires_at, club.value.timezone, locale.locale);
    parts.push(invite.active ? t("invites.expires", { day }) : t("invites.expired", { day }));
  }
  if (invite.created_by) parts.push(t("invites.by", { name: invite.created_by }));
  return parts.join(" · ");
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
          <li v-for="member in members" :key="member.username" class="member-card" :class="{ 'is-you': member.you }">
            <RouterLink class="member-link" :to="member.you ? '/shelf' : `/u/${member.username}`">
              <Avatar :username="member.username" :src="member.avatar_url" />
              <span class="member-meta">
                <strong>{{ member.username }}</strong>
                <span class="finer subtle">
                  <span v-if="member.role !== 'member'" class="badge role-badge">{{ t(`roles.${member.role}`) }}</span>
                  <template v-if="member.you">{{ t("club.thatsYou") }} · </template>
                  <span class="nums">{{ t("club.currentlyReading", { n: member.currently_reading_count }) }}</span>
                </span>
                <span class="finer subtle">{{ t("members.joined", { day: formatDay(member.joined_at, club.timezone, locale.locale) }) }}</span>
              </span>
              <span class="preview-row">
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
      <div class="btn-row">
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
              <span class="finer subtle">{{ inviteLine(invite) }}</span>
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
                <span class="finer subtle">
                  <span class="badge">{{ t(`invites.state.${inviteState(invite)}`) }}</span>
                  {{ inviteLine(invite) }}
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
      <form class="panel settings-form" @submit.prevent="saveSettings">
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

.member-list,
.invite-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
}

.member-card {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-3) var(--space-4);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
}

.member-card:hover {
  border-color: var(--border-strong);
}

.is-you {
  border-style: dashed;
}

.member-link {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: var(--space-3);
  color: inherit;
  text-decoration: none;
}

.member-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
  flex: 1;
}

.member-meta strong {
  font-family: var(--serif);
  font-size: var(--text-lg);
  overflow-wrap: anywhere;
}

.role-badge {
  margin-right: var(--space-1);
}

.preview-row {
  display: flex;
  gap: var(--space-1);
  flex: 0 0 auto;
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

.invite-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-3);
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}

.invite-row.past {
  opacity: 0.75;
}

.invite-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.invite-meta code {
  font-size: var(--text-md);
  letter-spacing: 0.05em;
}

.invite-actions {
  display: flex;
  flex: 0 0 auto;
}

.past-invites {
  margin-top: var(--space-3);
}

.past-invites summary {
  cursor: pointer;
  min-height: var(--tap);
  display: flex;
  align-items: center;
  font-weight: 600;
}

.settings-form,
.delete-form {
  max-width: 480px;
}

.danger-zone .btn-row {
  margin-top: var(--space-3);
}

.manage-role {
  margin-bottom: var(--space-3);
}

@media (min-width: 1024px) {
  .member-list {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
