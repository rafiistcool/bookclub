<script setup lang="ts">
import ReportSheet from "./ReportSheet.vue";

/**
 * Notice-and-takedown for a book's cover: the report sheet (`ReportSheet.vue`,
 * which serves every kind of #54) with `kind: "cover"`. The book page opens
 * it from "Report cover".
 */
defineProps<{ publicId: string; title: string }>();
const emit = defineEmits<{ close: [] }>();
</script>

<template>
  <ReportSheet :target="{ kind: 'cover', publicId, title }" @close="emit('close')" />
</template>
