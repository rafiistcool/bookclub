import { describe, expect, it } from "vitest";
import { activityLine, activityText } from "./activity";
import { activityItem, bookOut } from "./test/fixtures";

const book = bookOut({ id: 4, title: "Circe" });

describe("activity lines", () => {
  it.each([
    ["member_joined", {}, "ada joined Readers"],
    ["pick_set", { meeting_at: "2026-02-01T18:00:00Z" }, "ada made Circe the club pick"],
    ["pick_cleared", {}, "ada ended Circe as the club pick"],
    ["note_posted", { post_id: 9 }, "ada wrote about Circe"],
    ["nominated", {}, "ada nominated Circe"],
    ["voted", {}, "ada voted for Circe"],
    ["vote_closed", { votes: 3 }, "The vote chose Circe"],
    ["vote_closed", { votes: 1, auto: true }, "The vote closed at its deadline and chose Circe"],
    ["milestone_added", { title: "Part one" }, "ada added the milestone “Part one” for Circe"],
    ["shelf_added", { status: "want_to_read" }, "ada put Circe on Want to read"],
    ["shelf_moved", { status: "currently_reading" }, "ada moved Circe to Reading"],
    ["shelf_finished", { rating: 4, take: "Loved it" }, "ada finished Circe"],
    ["shelf_dnf", { reason: "Too slow" }, "ada stopped reading Circe"],
    ["shelf_removed", { status: "finished" }, "ada took Circe off their shelf"],
    ["progress", { progress: 40 }, "ada is 40% into Circe"],
    ["quote_added", { quote_id: 2, excerpt: "Witches are not gentle" }, "ada saved a quote from Circe"],
    ["reaction", { emoji: "🔥", post_id: 9, author: "grace" }, "ada reacted 🔥 to grace's note on Circe"],
    ["reaction", { emoji: "🔥", post_id: 9, author: null }, "ada reacted 🔥 to a note on Circe"],
  ])("%s %j", (kind, payload, text) => {
    const item = activityItem({ kind, payload, book, club: { slug: "readers-ab12", name: "Readers" } });
    expect(activityText(item)).toBe(text);
  });

  it("adds the take, the stars, the reason or the quote as a second line", () => {
    expect(activityLine(activityItem({ kind: "shelf_finished", payload: { rating: 4, take: "Loved it" } })).detail)
      .toBe("★★★★☆ · Loved it");
    // A redacted only-me rating leaves nothing behind.
    expect(activityLine(activityItem({ kind: "shelf_finished", payload: {} })).detail).toBe("");
    expect(activityLine(activityItem({ kind: "shelf_dnf", payload: { reason: "Too slow" } })).detail).toBe("Too slow");
    expect(activityLine(activityItem({ kind: "quote_added", payload: { excerpt: "Witches" } })).detail).toBe("“Witches”");
    expect(activityLine(activityItem({ kind: "vote_closed", payload: { votes: 3 } })).detail).toBe("3 votes");
  });

  it("names nobody for a system event and survives unknown kinds", () => {
    expect(activityText(activityItem({ kind: "shelf_finished", actor: null, book }))).toBe("Someone finished Circe");
    expect(activityText(activityItem({ kind: "something_new", book }))).toBe("ada · Circe");
  });
});
