import { describe, expect, it } from "vitest";
import { linkToken } from "./authToken";

describe("mail link tokens", () => {
  it("reads the token from the fragment, or from the query", () => {
    expect(linkToken("#token=abc_DEF-123")).toBe("abc_DEF-123");
    expect(linkToken("#other=1&token=xyz")).toBe("xyz");
    expect(linkToken("", "fromQuery")).toBe("fromQuery");
    expect(linkToken("#token=")).toBeNull();
    expect(linkToken("")).toBeNull();
  });
});
