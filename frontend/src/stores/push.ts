import { defineStore } from "pinia";
import { api, ApiError } from "../api/client";

function urlBase64ToUint8Array(base64: string): Uint8Array {
  const padding = "=".repeat((4 - (base64.length % 4)) % 4);
  const normalized = (base64 + padding).replace(/-/g, "+").replace(/_/g, "/");
  const raw = window.atob(normalized);
  return Uint8Array.from(raw, (char) => char.charCodeAt(0));
}

export const usePush = defineStore("push", {
  state: () => ({
    supported: false,
    permission: "default" as NotificationPermission | "unsupported",
    subscribed: false,
    endpoint: "" as string,
    busy: false,
    error: "",
  }),
  actions: {
    async init() {
      this.supported =
        typeof window !== "undefined" &&
        "serviceWorker" in navigator &&
        "PushManager" in window &&
        "Notification" in window;
      if (!this.supported) {
        this.permission = "unsupported";
        return;
      }
      this.permission = Notification.permission;
      try {
        const registration = await navigator.serviceWorker.ready;
        const existing = await registration.pushManager.getSubscription();
        this.subscribed = existing !== null;
        this.endpoint = existing?.endpoint ?? "";
      } catch {
        this.subscribed = false;
      }
    },
    async subscribe() {
      if (!this.supported) return;
      this.busy = true;
      this.error = "";
      try {
        const permission = await Notification.requestPermission();
        this.permission = permission;
        if (permission !== "granted") {
          this.error = "Notifications are blocked for this site.";
          return;
        }
        const registration = await navigator.serviceWorker.ready;
        const { public_key } = await api.vapidPublicKey();
        const subscription = await registration.pushManager.subscribe({
          userVisibleOnly: true,
          applicationServerKey: urlBase64ToUint8Array(public_key) as BufferSource,
        });
        const json = subscription.toJSON();
        await api.subscribePush({
          endpoint: subscription.endpoint,
          keys: { p256dh: json.keys?.p256dh ?? "", auth: json.keys?.auth ?? "" },
          user_agent: navigator.userAgent.slice(0, 200),
        });
        this.subscribed = true;
        this.endpoint = subscription.endpoint;
      } catch (err) {
        this.error = err instanceof ApiError ? err.message : "Could not turn notifications on.";
      } finally {
        this.busy = false;
      }
    },
    async unsubscribe() {
      if (!this.supported) return;
      this.busy = true;
      this.error = "";
      try {
        const registration = await navigator.serviceWorker.ready;
        const subscription = await registration.pushManager.getSubscription();
        if (subscription) {
          try {
            await api.unsubscribePush(subscription.endpoint);
          } catch {
            /* already gone server-side */
          }
          await subscription.unsubscribe();
        }
        this.subscribed = false;
        this.endpoint = "";
      } catch (err) {
        this.error = err instanceof ApiError ? err.message : "Could not turn notifications off.";
      } finally {
        this.busy = false;
      }
    },
  },
});
