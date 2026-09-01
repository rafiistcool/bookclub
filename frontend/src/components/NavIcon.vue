<script setup lang="ts">
defineProps<{
  name:
    | "home"
    | "search"
    | "shelf"
    | "people"
    | "back"
    | "close"
    | "more"
    | "filter"
    | "camera"
    | "calendar"
    | "bell"
    | "check"
    | "plus"
    | "trash"
    | "edit"
    | "chart"
    | "quote"
    | "sun"
    | "moon";
  size?: number;
}>();
</script>

<template>
  <svg
    :width="size ?? 22"
    :height="size ?? 22"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.8"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <template v-if="name === 'home'">
      <path d="M4 11.5 12 5l8 6.5" />
      <path d="M6.5 10.5V19h11v-8.5" />
    </template>
    <template v-else-if="name === 'search'">
      <circle cx="11" cy="11" r="6.5" />
      <path d="M16 16.5 20 20.5" />
    </template>
    <template v-else-if="name === 'shelf'">
      <rect x="3.5" y="4.5" width="5" height="15" rx="1" />
      <rect x="9.5" y="4.5" width="5" height="15" rx="1" />
      <path d="M15.5 5.5l4.2-1 3 14.2-4.2 1z" transform="translate(-1 0) scale(.9)" />
    </template>
    <template v-else-if="name === 'people'">
      <circle cx="9" cy="9" r="3" />
      <circle cx="16.5" cy="10" r="2.4" />
      <path d="M4.5 18c.6-2.6 2.6-4 4.5-4s3.9 1.4 4.5 4" />
      <path d="M14 17.6c.5-1.8 1.9-2.8 3.3-2.8 1.3 0 2.5.8 3.2 2.4" />
    </template>
    <template v-else-if="name === 'back'">
      <path d="M15 5l-7 7 7 7" />
    </template>
    <template v-else-if="name === 'close'">
      <path d="M6 6l12 12M18 6L6 18" />
    </template>
    <template v-else-if="name === 'more'">
      <circle cx="6" cy="12" r="1.4" fill="currentColor" stroke="none" />
      <circle cx="12" cy="12" r="1.4" fill="currentColor" stroke="none" />
      <circle cx="18" cy="12" r="1.4" fill="currentColor" stroke="none" />
    </template>
    <template v-else-if="name === 'filter'">
      <path d="M4 7h16M7 12h10M10 17h4" />
    </template>
    <template v-else-if="name === 'camera'">
      <path d="M4 8.5A1.5 1.5 0 0 1 5.5 7H8l1.2-2h5.6L16 7h2.5A1.5 1.5 0 0 1 20 8.5v9A1.5 1.5 0 0 1 18.5 19h-13A1.5 1.5 0 0 1 4 17.5z" />
      <circle cx="12" cy="13" r="3.2" />
    </template>
    <template v-else-if="name === 'calendar'">
      <rect x="4" y="6" width="16" height="14" rx="2" />
      <path d="M8 4v4M16 4v4M4 11h16" />
    </template>
    <template v-else-if="name === 'bell'">
      <path d="M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15z" />
      <path d="M10 20a2 2 0 0 0 4 0" />
    </template>
    <template v-else-if="name === 'check'">
      <path d="M5 12.5l4.5 4.5L19 7.5" />
    </template>
    <template v-else-if="name === 'plus'">
      <path d="M12 5v14M5 12h14" />
    </template>
    <template v-else-if="name === 'trash'">
      <path d="M5 7h14M9 7V5h6v2M8 7l.8 12h6.4L16 7" />
    </template>
    <template v-else-if="name === 'edit'">
      <path d="M4 20h4l10.5-10.5a1.5 1.5 0 0 0 0-2.1l-1.9-1.9a1.5 1.5 0 0 0-2.1 0L4 16z" />
      <path d="M13.5 6.5l4 4" />
    </template>
    <template v-else-if="name === 'chart'">
      <path d="M4 20V10M10 20V4M16 20v-7M22 20H2" />
    </template>
    <template v-else-if="name === 'quote'">
      <path d="M7 16c-1.7 0-3-1.3-3-3s1.3-3 3-3V7c-3 0-5 2.2-5 6 0 2.8 1.6 5 4 5zM17 16c-1.7 0-3-1.3-3-3s1.3-3 3-3V7c-3 0-5 2.2-5 6 0 2.8 1.6 5 4 5z" />
    </template>
    <template v-else-if="name === 'sun'">
      <circle cx="12" cy="12" r="4" />
      <path d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M5.6 18.4 7 17M17 7l1.4-1.4" />
    </template>
    <template v-else-if="name === 'moon'">
      <path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z" />
    </template>
  </svg>
</template>
