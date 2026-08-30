import { defineStore } from "pinia";

export type ToastAction = {
  label: string;
  run: () => void | Promise<void>;
};

export type Toast = {
  id: number;
  message: string;
  action: ToastAction | null;
};

const PLAIN_MS = 3200;
const ACTIONABLE_MS = 7000;
const MAX_VISIBLE = 3;

// Timers live outside the store so the state stays plain data.
const timers = new Map<number, number>();
let nextId = 0;

export const useToast = defineStore("toast", {
  state: () => ({
    items: [] as Toast[],
  }),
  actions: {
    show(message: string, action?: ToastAction) {
      const id = ++nextId;
      this.items.push({ id, message, action: action ?? null });
      while (this.items.length > MAX_VISIBLE) {
        const dropped = this.items.shift();
        if (dropped) this.clearTimer(dropped.id);
      }
      timers.set(
        id,
        window.setTimeout(() => this.dismiss(id), action ? ACTIONABLE_MS : PLAIN_MS),
      );
      return id;
    },
    dismiss(id: number) {
      this.clearTimer(id);
      this.items = this.items.filter((toast) => toast.id !== id);
    },
    async run(id: number) {
      const toast = this.items.find((row) => row.id === id);
      this.dismiss(id);
      await toast?.action?.run();
    },
    clearTimer(id: number) {
      const timer = timers.get(id);
      if (timer !== undefined) {
        window.clearTimeout(timer);
        timers.delete(id);
      }
    },
  },
});
