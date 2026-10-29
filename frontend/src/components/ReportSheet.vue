<script lang="ts">
/**
 * What a reader reports (#54, Q11 option B): a cover or a profile picture
 * (both by the picture's public id), or a book or an edition a reader added.
 */
export type ReportTarget =
  | { kind: "cover"; publicId: string; title: string }
  | { kind: "avatar"; publicId: string; username: string }
  | { kind: "work"; workId: number; title: string }
  | { kind: "edition"; editionId: number; title: string };

/** The public id in a picture's address (`/media/{public_id}/a`); null for anything else. */
export function picturePublicId(url: string | null | undefined): string | null {
  const match = /^\/media\/([a-z0-9]{26})\//.exec(url ?? "");
  return match ? match[1]! : null;
}
</script>

<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import type { ReportIn } from "../api/generated";
import { useService } from "../stores/service";
import { useToast } from "../stores/toast";
import BottomSheet from "./BottomSheet.vue";

/**
 * Notice and takedown: tell an admin that a cover, a profile picture, or a
 * book or edition a reader added is wrong, offensive or infringes someone's
 * rights. "It shows a different book" is offered for covers only. The admin's
 * decision comes back by mail. A last line names the public contact address
 * for people without an account, when the server has one (`/api/config`).
 */
const props = defineProps<{ target: ReportTarget }>();
const emit = defineEmits<{ close: [] }>();

const COVER_REASONS = ["wrong_book", "offensive", "copyright", "other"] as const;
const OTHER_REASONS = ["offensive", "copyright", "other"] as const;
type Reason = (typeof COVER_REASONS)[number];

const { t } = useI18n();
const toast = useToast();
const service = useService();
const reason = ref<Reason | "">("");
const details = ref("");
const busy = ref(false);
const error = ref("");

const reasons = computed<readonly Reason[]>(() => (props.target.kind === "cover" ? COVER_REASONS : OTHER_REASONS));
/** The line under the title: the book, or whose picture it is. */
const subject = computed(() =>
  props.target.kind === "avatar" ? t("report.avatarOf", { name: props.target.username }) : props.target.title,
);

function send(body: ReportIn) {
  const target = props.target;
  if (target.kind === "work") return api.reportWork(target.workId, body);
  if (target.kind === "edition") return api.reportEdition(target.editionId, body);
  return api.reportCover(target.publicId, body);
}

async function submit() {
  if (!reason.value || busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    const created = await send({ reason: reason.value, details: details.value.trim() || null });
    // A repeat names the reader's open report; a picture taken down meanwhile needs nobody.
    toast.show(created.status === "removed" ? t("cover.alreadyRemoved") : t("report.sent"));
    emit("close");
  } catch (err) {
    error.value = err instanceof ApiError ? err.message : t("report.failed");
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <BottomSheet :title="t(`report.title.${target.kind}`)" @close="emit('close')">
    <p class="muted report-subject">{{ subject }}</p>
    <form class="stack" @submit.prevent="submit">
      <fieldset class="report-reasons">
        <legend class="fine">{{ t("report.why") }}</legend>
        <label v-for="option in reasons" :key="option" class="report-reason">
          <input v-model="reason" type="radio" name="reason" :value="option" :disabled="busy" />
          {{ t(`cover.reasons.${option}`) }}
        </label>
      </fieldset>
      <label class="field">
        <span>{{ t("report.details") }}</span>
        <textarea v-model="details" rows="3" maxlength="2000" :placeholder="t('common.optional')" :disabled="busy" />
      </label>
      <p class="fine subtle report-note">{{ t("report.next") }}</p>
      <p v-if="error" class="error">{{ error }}</p>
      <button class="btn btn-primary btn-block" type="submit" :disabled="busy || !reason">
        {{ busy ? t("report.sending") : t("report.send") }}
      </button>
      <button class="btn btn-ghost btn-block" type="button" :disabled="busy" @click="emit('close')">
        {{ t("common.cancel") }}
      </button>
      <p v-if="service.contact_email" class="fine subtle report-note report-contact">
        <i18n-t keypath="report.noAccount" tag="span" scope="global">
          <template #email>
            <a :href="`mailto:${service.contact_email}`">{{ service.contact_email }}</a>
          </template>
        </i18n-t>
      </p>
    </form>
  </BottomSheet>
</template>

<style scoped>
.report-subject { margin-bottom: var(--space-2); overflow-wrap: anywhere; }
.report-reasons { display: grid; gap: var(--space-2); margin: 0; padding: 0; border: 0; }
.report-reasons legend { margin-bottom: var(--space-2); }
.report-reason { display: flex; align-items: center; gap: var(--space-2); min-height: var(--tap); }
.report-note { margin: 0; }
.report-contact a { overflow-wrap: anywhere; }
</style>
