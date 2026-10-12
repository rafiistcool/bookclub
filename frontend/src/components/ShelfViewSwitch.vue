<script setup lang="ts">
import { Squares2X2Icon } from "@heroicons/vue/24/outline";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import NavIcon from "./NavIcon.vue";

// `base` points the switch at another member's shelves: /club/<name>.
const props = defineProps<{ active: "books" | "physical"; base?: string }>();
const booksPath = computed(() => props.base ?? "/shelf");
const shelfPath = computed(() => props.base ? `${props.base}/shelf` : "/shelf/physical");
const { t } = useI18n();
</script>

<template>
  <nav class="shelf-view-switch" :aria-label="t('physicalShelf.view')">
    <RouterLink :to="booksPath" :aria-current="active === 'books' ? 'page' : undefined"><Squares2X2Icon aria-hidden="true" />{{ t('physicalShelf.bookView') }}</RouterLink>
    <RouterLink :to="shelfPath" :aria-current="active === 'physical' ? 'page' : undefined"><NavIcon name="shelf" :size="17" />{{ t('physicalShelf.shelfView') }}</RouterLink>
  </nav>
</template>

<style scoped>
.shelf-view-switch { display: inline-flex; gap: 3px; padding: 4px; border: 1px solid var(--border); border-radius: var(--radius-pill); background: var(--surface-2); }
.shelf-view-switch a { display: flex; gap: 8px; align-items: center; justify-content: center; min-height: 36px; padding: 7px 17px; border-radius: var(--radius-pill); color: var(--text-muted); text-decoration: none; font-size: var(--text-sm); font-weight: 600; }
.shelf-view-switch a[aria-current="page"] { background: var(--surface); color: var(--text); box-shadow: var(--shadow-1); }
.shelf-view-switch a:hover { color: var(--accent); }
.shelf-view-switch svg { width: 17px; height: 17px; }
@media (max-width: 600px) { .shelf-view-switch a { padding: 7px 13px; } }
</style>
