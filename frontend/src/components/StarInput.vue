<script setup lang="ts">
const props = defineProps<{
  modelValue: number | null;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: number | null];
}>();

function pick(value: number) {
  emit("update:modelValue", props.modelValue === value ? null : value);
}
</script>

<template>
  <div class="star-input" role="radiogroup" aria-label="Rating">
    <button
      v-for="n in 5"
      :key="n"
      class="star-btn"
      type="button"
      role="radio"
      :aria-checked="modelValue === n"
      :aria-label="`${n} of 5`"
      @click="pick(n)"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M12 3.6l2.6 5.5 6 .7-4.4 4.1 1.2 5.9L12 16.9l-5.4 2.9 1.2-5.9L3.4 9.8l6-.7z"
          :fill="modelValue && n <= modelValue ? 'currentColor' : 'none'"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linejoin="round"
        />
      </svg>
    </button>
  </div>
</template>
