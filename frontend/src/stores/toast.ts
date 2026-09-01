import { defineStore } from "pinia";

export type ToastAction = {
  label: string;
  run: () => void | Promise<void>;
};

export const useToast = defineStore("toast", {
  state: () => ({
    message: "",
    visible: false,
    action: null as ToastAction | null,
    timer: 0 as number,
  }),
  actions: {
    show(message: string, action: ToastAction | null = null, duration = action ? 5200 : 3200) {
      this.message = message;
      this.action = action;
      this.visible = true;
      window.clearTimeout(this.timer);
      this.timer = window.setTimeout(() => this.hide(), duration);
    },
    hide() {
      this.visible = false;
      this.action = null;
      window.clearTimeout(this.timer);
    },
    async act() {
      const action = this.action;
      this.hide();
      if (action) await action.run();
    },
  },
});
