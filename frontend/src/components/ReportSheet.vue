<script lang="ts">
/**
 * What a reader reports (#54, Q11 option B): a cover or a profile picture
 * (both by the picture's public id), or a book or an edition a reader added;
 * and (#55, Q11 option C) what another reader wrote: a post (a club post, a
 * reply or a diary entry), a quote, or the take on a book by its reader's
 * name. `author` names the writer under the sheet's title.
 */
export type ReportTarget =
  | { kind: "cover"; publicId: string; title: string }
  | { kind: "avatar"; publicId: string; username: string }
  | { kind: "work"; workId: number; title: string }
  | { kind: "edition"; editionId: number; title: string }
  | { kind: "post"; postId: number; author: string }
  | { kind: "quote"; quoteId: number; author: string }
  | { kind: "take"; workId: number; author: string };

/** A text another reader wrote, not a picture or a book. */
export function isTextTarget(target: ReportTarget): target is Extract<ReportTarget, { author: string }> {
  return target.kind === "post" || target.kind === "quote" || target.kind === "take";
}

/**
 * The stored title of a book a reader added whose text an admin removed on a
 * report (`catalog::reports::REMOVED_TITLE` on the server, which
 * `server/tests/catalog_reports.rs` holds to this line). The book page shows
 * it translated (`report.removedTitle`).
 */
export const REMOVED_TITLE = "[removed]";

/**
 * A book a reader added whose text an admin removed on a report: it shows
 * the placeholder translated, and offers no report (nothing is left to
 * remove; the server would close one on arrival).
 */
export function isWithdrawn(book: { custom: boolean; title: string } | null | undefined): boolean {
  return Boolean(book?.custom && book.title === REMOVED_TITLE);
}

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
 * Notice and takedown: tell an admin that a cover, a profile picture, a book
 * or edition a reader added, or a post, quote or take another reader wrote is
 * wrong, offensive or infringes someone's rights. "It shows a different book"
 * is offered for covers only (the server refuses it for texts). The admin's
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
/** The line under the title: the book, whose picture it is, or who wrote the text. */
const subject = computed(() => {
  const target = props.target;
  if (target.kind === "avatar") return t("report.avatarOf", { name: target.username });
  if (isTextTarget(target)) return t(`report.by.${target.kind}`, { name: target.author });
  return target.title;
});

function send(body: ReportIn) {
  const target = props.target;
  if (target.kind === "work") return api.reportWork(target.workId, body);
  if (target.kind === "edition") return api.reportEdition(target.editionId, body);
  if (target.kind === "post") return api.reportPost(target.postId, body);
  if (target.kind === "quote") return api.reportQuote(target.quoteId, body);
  if (target.kind === "take") return api.reportTake(target.workId, target.author, body);
  return api.reportCover(target.publicId, body);
}

async function submit() {
  if (!reason.value || busy.value) return;
  busy.value = true;
  error.value = "";
  try {
    const created = await send({ reason: reason.value, details: details.value.trim() || null });
    // A repeat names the reader's open report; a picture taken down or a text removed
    // meanwhile needs nobody.
    const picture = props.target.kind === "cover" || props.target.kind === "avatar";
    const done = picture ? t("cover.alreadyRemoved") : t("report.alreadyRemoved");
    toast.show(created.status === "removed" ? done : t("report.sent"));
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
