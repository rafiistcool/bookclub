import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { describe, expect, it, vi } from "vitest";

vi.mock("../api/client", () => ({ ApiError: class ApiError extends Error {}, api: {} }));

import { i18n } from "../i18n";
import { useSession } from "../stores/session";
import { user } from "../test/fixtures";
import type { ClubSummary } from "../types";
import ClubBookActions from "./ClubBookActions.vue";

const MeetingSheet = { name: "MeetingSheet", props: ["title", "bookTitle", "timezone", "blurb"], template: '<div class="meeting-stub">{{ blurb }}</div>' };
const BottomSheet = { name: "BottomSheet", props: ["title"], template: '<div class="sheet-stub"><slot /></div>' };

function mountActions(clubs: ClubSummary[]) {
  const pinia = createPinia();
  setActivePinia(pinia);
  i18n.global.locale.value = "en";
  useSession().user = user({ clubs });
  return mount(ClubBookActions, { global: { plugins: [pinia, i18n], stubs: { MeetingSheet, BottomSheet } } });
}

const readers: ClubSummary = { slug: "readers-ab12", name: "Readers", role: "admin", timezone: "UTC", member_count: 3 };
const owls: ClubSummary = { slug: "night-cd34", name: "Night owls", role: "owner", timezone: "UTC", member_count: 2 };

describe("ClubBookActions", () => {
  it("goes straight to the one club whose pick the book is not yet: the club the row named", async () => {
    const wrapper = mountActions([readers, owls]);
    (wrapper.vm as unknown as { start: (action: string, book: object) => void })
      .start("pick", { id: 4, title: "Circe", currentIn: [{ slug: readers.slug, name: readers.name }] });
    await flushPromises();
    expect(wrapper.find(".sheet-stub").exists()).toBe(false);
    expect(wrapper.get(".meeting-stub").text()).toContain("Night owls");
  });

  it("asks which club when the book can become the pick in several", async () => {
    const wrapper = mountActions([readers, owls]);
    (wrapper.vm as unknown as { start: (action: string, book: object) => void })
      .start("pick", { id: 4, title: "Circe", currentIn: [] });
    await flushPromises();
    expect(wrapper.find(".meeting-stub").exists()).toBe(false);
    expect(wrapper.get(".sheet-stub").text()).toContain("Which club?");
  });
});
