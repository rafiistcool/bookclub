<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../../api/client";
import type { InviteOut, SignupInviteIn } from "../../api/generated";
import { tp } from "../../i18n";
import { formatDay } from "../../i18n/dates";
import { inviteState } from "../../invites";
import { useLocale } from "../../stores/locale";
import { useSession } from "../../stores/session";
import { useToast } from "../../stores/toast";

/**
 * Sign-up invites for the closed beta: they let someone create an account
 * without joining a club. Without a limit of uses or days an invite is for
 * one person and lasts 30 days (the server's default).
 */
const { t } = useI18n();
const locale = useLocale();
const session = useSession();
const toast = useToast();

const items = ref<InviteOut[]>([]);
const loaded = ref(false);
const error = ref("");
const busy = ref(false);
const uses = ref<number | "">("");
const days = ref<number | "">("");
const fresh = ref<InviteOut | null>(null);

const active = computed(() => items.value.filter((row) => row.active));
const past = computed(() => items.value.filter((row) => !row.active));

function failed(err: unknown, fallback: string): string {
  return err instanceof ApiError ? err.message : fallback;
}

async function load() {
  try {
    items.value = (await api.adminInvites()).items;
    error.value = "";
  } catch (err) {
    error.value = failed(err, t("invites.loadFailed"));
  } finally {
    loaded.value = true;
  }
}

function whole(value: number | ""): number | null {
  return value === "" || !Number.isFinite(Number(value)) ? null : Math.trunc(Number(value));
}

async function create() {
  if (busy.value) return;
  const body: SignupInviteIn = {};
  const maxUses = whole(uses.value);
  const expires = whole(days.value);
  if (maxUses != null) body.max_uses = maxUses;
  if (expires != null) body.expires_in_days = expires;
  busy.value = true;
  try {
    const invite = await api.createSignupInvite(body);
    items.value = [invite, ...items.value];
    fresh.value = invite;
    uses.value = "";
    days.value = "";
  } catch (err) {
    toast.show(failed(err, t("invites.createFailed")));
  } finally {
    busy.value = false;
  }
}

async function copy(invite: InviteOut) {
  try {
    await navigator.clipboard.writeText(invite.url);
    toast.show(t("invites.copied"));
  } catch {
    toast.show(t("invites.copyFailed"));
  }
}

function line(invite: InviteOut): string {
  const zone = session.user?.timezone || "UTC";
  const parts: string[] = [];
  parts.push(invite.max_uses == null ? t("admin.anyNumber") : tp("admin.forPeople", invite.max_uses));
  parts.push(tp("invites.uses", invite.use_count));
  if (invite.expires_at) {
    const day = formatDay(invite.expires_at, zone, locale.locale);
    parts.push(invite.active ? t("invites.expires", { day }) : t("invites.expired", { day }));
  } else {
    parts.push(t("admin.noExpiry"));
  }
  if (invite.created_by) parts.push(t("invites.by", { name: invite.created_by }));
  return parts.join(" · ");
}

onMounted(load);
</script>

<template>
  <section class="admin-queue" aria-labelledby="admin-invites">
    <div class="section-head">
      <h2 id="admin-invites">{{ t("admin.invites") }}</h2>
    </div>
    <p class="fine muted">{{ t("admin.invitesBlurb") }}</p>

    <form class="panel invite-form" @submit.prevent="create">
      <div class="field-pair">
        <label class="field">
          <span>{{ t("admin.maxUses") }}</span>
          <input v-model.number="uses" name="max-uses" type="number" min="1" max="100" inputmode="numeric" :placeholder="t('admin.noLimit')" />
        </label>
        <label class="field">
          <span>{{ t("admin.days") }}</span>
          <input v-model.number="days" name="days" type="number" min="1" max="90" inputmode="numeric" :placeholder="t('admin.noLimit')" />
        </label>
      </div>
      <p class="field-hint">{{ t("admin.inviteDefaults") }}</p>
      <button class="btn btn-primary btn-sm" type="submit" :disabled="busy">{{ t("admin.newInvite") }}</button>
    </form>

    <div v-if="fresh" class="panel fresh-invite">
      <p class="fine">{{ t("admin.freshInvite") }}</p>
      <code class="invite-url">{{ fresh.url }}</code>
      <div class="btn-row">
        <button class="btn btn-primary btn-sm" type="button" @click="copy(fresh)">{{ t("invites.copyLink") }}</button>
      </div>
    </div>

    <p v-if="error" class="error">{{ error }}</p>
    <div v-else-if="!loaded" class="skeleton admin-skeleton" aria-hidden="true" />
    <template v-else>
      <h3 class="invite-subhead">{{ t("invites.active") }}</h3>
      <p v-if="active.length === 0" class="fine subtle">{{ t("invites.noneActive") }}</p>
      <ul v-else class="invite-list">
        <li v-for="invite in active" :key="invite.id" class="invite-row">
          <span class="invite-meta">
            <code>{{ invite.code }}</code>
            <span class="finer subtle">{{ line(invite) }}</span>
            <span v-if="invite.used_by.length" class="finer subtle">{{ t("invites.usedBy", { names: invite.used_by.join(", ") }) }}</span>
          </span>
          <button class="text-btn" type="button" @click="copy(invite)">{{ t("common.copy") }}</button>
        </li>
      </ul>
      <details v-if="past.length" class="past-invites">
        <summary class="fine">{{ tp("admin.pastInvites", past.length) }}</summary>
        <ul class="invite-list">
          <li v-for="invite in past" :key="invite.id" class="invite-row past">
            <span class="invite-meta">
              <code>{{ invite.code }}</code>
              <span class="finer subtle">
                <span class="badge">{{ t(`invites.state.${inviteState(invite)}`) }}</span>
                {{ line(invite) }}
              </span>
              <span v-if="invite.used_by.length" class="finer subtle">{{ t("invites.usedBy", { names: invite.used_by.join(", ") }) }}</span>
            </span>
          </li>
        </ul>
      </details>
    </template>
  </section>
</template>

<style scoped>
.admin-skeleton {
  height: 120px;
}

.invite-form,
.fresh-invite {
  display: grid;
  gap: var(--space-3);
  margin-top: var(--space-3);
}

.field-pair {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(9rem, 1fr));
  gap: var(--space-3);
}

.field-pair .field,
.invite-form .field-hint {
  margin: 0;
}

.invite-url {
  display: block;
  padding: var(--space-2) var(--space-3);
  overflow-wrap: anywhere;
  font-size: var(--text-sm);
  background: var(--surface-2);
  border-radius: var(--radius-sm);
}

.invite-subhead {
  font-size: var(--text-md);
  margin: var(--space-5) 0 var(--space-2);
}

.invite-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: var(--space-2);
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
  opacity: 0.8;
}

.invite-meta {
  display: grid;
  gap: 2px;
  min-width: 0;
}

.past-invites {
  margin-top: var(--space-3);
}

.past-invites .invite-list {
  margin-top: var(--space-2);
}
</style>
