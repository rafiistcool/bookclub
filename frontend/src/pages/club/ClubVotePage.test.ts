import { flushPromises, mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import { describe, expect, it, vi } from "vitest";
import { createMemoryHistory, createRouter } from "vue-router";
import { bookOut, clubOut, user } from "../../test/fixtures";

const voteSuggestions = vi.fn();

vi.mock("../../api/client", () => ({
  ApiError: class ApiError extends Error {},
  api: { voteSuggestions: (...args: unknown[]) => voteSuggestions(...args) },
}));

import { i18n } from "../../i18n";
import { useClubs } from "../../stores/clubs";
import { useSession } from "../../stores/session";
import ClubVotePage from "./ClubVotePage.vue";

describe("ClubVotePage", () => {
  it("lists the suggestions with each one's own reason and no blurb over them", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const club = clubOut({ role: "member" });
    useSession().user = user({ clubs: [{ ...club }] });
    useClubs().current = club;
    voteSuggestions.mockResolvedValue({
      items: [
        {
          book: bookOut({ id: 3, title: "Piranesi" }),
          score: 2,
          reasons: ["On 2 shelves: ada, grace"],
          reason_codes: [{ code: "wanted", usernames: ["ada", "grace"] }],
        },
      ],
    });
    const router = createRouter({ history: createMemoryHistory(), routes: [{ path: "/:rest(.*)*", component: ClubVotePage }] });
    await router.push(`/c/${club.slug}/vote`);
    const wrapper = mount(ClubVotePage, {
      global: { plugins: [pinia, router, i18n], stubs: { BookCover: true, NextUpVote: true } },
    });
    await flushPromises();

    expect(voteSuggestions).toHaveBeenCalledWith(club.slug);
    expect(wrapper.get("#suggestions").text()).toBe(i18n.global.t("vote.suggestions"));
    expect(wrapper.find(".suggestions-blurb").exists()).toBe(false);
    expect(wrapper.text()).not.toContain("ranked by");
    const rows = wrapper.findAll(".suggestion");
    expect(rows).toHaveLength(1);
    expect(rows[0]!.get(".suggestion-meta").text()).toContain("Piranesi");
    // The row says why it is suggested, so the list needs no explanation of its ranking.
    expect(rows[0]!.get(".suggestion-meta").text()).toContain("ada, grace");
  });
});
