<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import ActivityFeed from "../../components/ActivityFeed.vue";
import { useCurrentClub } from "../../stores/clubs";

const { t } = useI18n();
const club = useCurrentClub();
const slug = computed(() => club.value.slug);
</script>

<template>
  <div class="events-page">
    <ActivityFeed :club="slug" :heading="t('activity.clubHeading')" :page-size="30" />
  </div>
</template>

<style scoped>
@media (min-width: 1024px) {
  .events-page {
    max-width: 760px;
  }
}
</style>
