<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import BookDiary from "../../components/BookDiary.vue";
import DiaryFeed from "../../components/DiaryFeed.vue";
import { useCurrentClub } from "../../stores/clubs";

const { t } = useI18n();
const route = useRoute();

const club = useCurrentClub();
const feed = ref<InstanceType<typeof DiaryFeed> | null>(null);
const slug = computed(() => club.value.slug);
/** `?pick=<id>`: an earlier pick's discussion (from the past picks on the club page). */
const pickId = computed(() => {
  const raw = Number(route.query.pick);
  return Number.isSafeInteger(raw) && raw > 0 ? raw : null;
});
</script>

<template>
  <div class="discussions">
    <section class="thread">
      <p class="lede">{{ pickId ? t("discussions.earlierLede") : t("discussions.lede") }}</p>
      <RouterLink v-if="pickId" class="fine" :to="`/c/${slug}/discussions`">
        {{ t("discussions.toCurrent") }}
      </RouterLink>
      <BookDiary :club="{ slug, pickId }" :heading="t('discussions.heading')" @posted="feed?.reload()" />
    </section>

    <section class="section">
      <DiaryFeed
        ref="feed"
        :club="slug"
        :heading="t('discussions.written')"
        :empty-text="t('discussions.nothingWritten')"
      />
    </section>
  </div>
</template>

<style scoped>
.thread {
  display: grid;
  gap: var(--space-3);
}

.thread > .fine {
  justify-self: start;
  font-weight: 600;
}

@media (min-width: 1024px) {
  .discussions {
    max-width: 760px;
  }
}
</style>
