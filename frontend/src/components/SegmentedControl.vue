<script setup lang="ts">
export type Segment<T extends string> = { value: T; label: string; count?: number };

const props = defineProps<{
  modelValue: string;
  options: Segment<string>[];
  label?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

function onKey(event: KeyboardEvent, index: number) {
  const delta = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
  if (!delta) return;
  event.preventDefault();
  const next = props.options[(index + delta + props.options.length) % props.options.length];
  emit("update:modelValue", next.value);
  const target = (event.currentTarget as HTMLElement).parentElement?.children[
    (index + delta + props.options.length) % props.options.length
  ] as HTMLElement | undefined;
  target?.focus();
}
</script>

<template>
  <div class="segmented" role="tablist" :aria-label="label ?? 'Section'">
    <button
      v-for="(option, index) in options"
      :key="option.value"
      type="button"
      role="tab"
      :aria-selected="option.value === modelValue"
      :tabindex="option.value === modelValue ? 0 : -1"
      @click="emit('update:modelValue', option.value)"
      @keydown="onKey($event, index)"
    >
      <span>{{ option.label }}</span>
      <span v-if="option.count !== undefined" class="count">{{ option.count }}</span>
    </button>
  </div>
</template>
