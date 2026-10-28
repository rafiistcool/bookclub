<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { api, ApiError } from "../api/client";
import { clubsFor, type ClubAction } from "../clubActions";
import { useSession } from "../stores/session";
import { useToast } from "../stores/toast";
import type { ClubRef, ClubSummary } from "../types";
import BottomSheet from "./BottomSheet.vue";
import MeetingSheet from "./MeetingSheet.vue";

/** The book a club action is about. `currentIn` lists the clubs whose pick it already is. */
export type ClubActionBook = { id: number; title: string; currentIn?: readonly ClubRef[] };

const emit = defineEmits<{ picked: [club: ClubRef] }>();

const { t } = useI18n();
const session = useSession();
const toast = useToast();

const action = ref<ClubAction | null>(null);
const book = ref<ClubActionBook | null>(null);
const choosing = ref(false);
const pickClub = ref<ClubSummary | null>(null);

const choices = computed(() => (action.value ? clubsFor(action.value, session.user?.clubs) : []));

function isCurrentPick(club: ClubSummary): boolean {
  return Boolean(book.value?.currentIn?.some((row) => row.slug === club.slug));
}

/** Set the pick or nominate in one of the viewer's clubs; asks which club when there are several. */
function start(next: ClubAction, target: ClubActionBook) {
  action.value = next;
  book.value = target;
  const clubs = clubsFor(next, session.user?.clubs);
  if (clubs.length === 1) choose(clubs[0]!);
  else if (clubs.length > 1) choosing.value = true;
}

function choose(club: ClubSummary) {
  choosing.value = false;
  if (action.value === "pick") pickClub.value = club;
  else void nominate(club);
}

async function nominate(club: ClubSummary) {
  const target = book.value;
  if (!target) return;
  try {
    await api.nominate(club.slug, target.id);
    toast.show(t("book.nominatedIn", { club: club.name }));
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.nominateFailed"));
  }
}

async function confirmPick(meetingAt: string | null) {
  const club = pickClub.value;
  const target = book.value;
  pickClub.value = null;
  if (!club || !target) return;
  try {
    await api.setClubPick(club.slug, { work_id: target.id, meeting_at: meetingAt });
    toast.show(t("book.setPickIn", { club: club.name }));
    emit("picked", { slug: club.slug, name: club.name });
  } catch (err) {
    toast.show(err instanceof ApiError ? err.message : t("book.setPickFailed"));
  }
}

defineExpose({ start });
</script>

<template>
  <BottomSheet
    v-if="choosing && book"
    :title="t(action === 'pick' ? 'book.setAsPick' : 'book.nominate')"
    @close="choosing = false"
  >
    <p class="muted club-choice-book">{{ book.title }}</p>
    <p class="fine subtle">{{ t("book.whichClub") }}</p>
    <div class="stack">
      <button
        v-for="club in choices"
        :key="club.slug"
        type="button"
        class="btn btn-ghost btn-block"
        :disabled="action === 'pick' && isCurrentPick(club)"
        @click="choose(club)"
      >
        {{ club.name }}
        <span v-if="action === 'pick' && isCurrentPick(club)" class="fine subtle">· {{ t("book.alreadyPick") }}</span>
      </button>
      <button type="button" class="btn btn-ghost btn-block" @click="choosing = false">
        {{ t("common.cancel") }}
      </button>
    </div>
  </BottomSheet>

  <MeetingSheet
    v-if="pickClub && book"
    :title="t('book.setAsPick')"
    :book-title="book.title"
    :timezone="pickClub.timezone"
    :blurb="t('book.pickBlurbIn', { club: pickClub.name })"
    @confirm="confirmPick"
    @close="pickClub = null"
  />
</template>

<style scoped>
.club-choice-book { margin-bottom: var(--space-2); }
</style>
