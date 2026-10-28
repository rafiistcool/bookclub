import { describe, expect, it } from "vitest";
import { ApiError } from "./api/client";
import { isDeadLink, LINK_EXPIRED, linkToken } from "./authToken";

describe("mail link tokens", () => {
  it("reads the token from the fragment, or from the query", () => {
    expect(linkToken("#token=abc_DEF-123")).toBe("abc_DEF-123");
    expect(linkToken("#other=1&token=xyz")).toBe("xyz");
    expect(linkToken("", "fromQuery")).toBe("fromQuery");
    expect(linkToken("#token=")).toBeNull();
    expect(linkToken("")).toBeNull();
  });

  it("tells a dead link from an error another try can fix", () => {
    expect(isDeadLink(new ApiError("Link expired", 400, LINK_EXPIRED))).toBe(true);
    const short = "New password must be at least 8 characters";
    expect(isDeadLink(new ApiError(short, 400, short))).toBe(false);
    expect(isDeadLink(new ApiError("Slow down", 429, LINK_EXPIRED))).toBe(false);
    expect(isDeadLink(new Error(LINK_EXPIRED))).toBe(false);
  });
});
