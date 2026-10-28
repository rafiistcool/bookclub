import { describe, expect, it } from "vitest";
import { ApiError } from "./api/client";
import { worthRetrying } from "./retry";

describe("worthRetrying", () => {
  it("retries a failed network, a body that is not ours, and a server failure", () => {
    expect(worthRetrying(new TypeError("Failed to fetch"))).toBe(true);
    expect(worthRetrying(new SyntaxError("Unexpected token <"))).toBe(true);
    expect(worthRetrying(new ApiError("Bad gateway", 502, null))).toBe(true);
    expect(worthRetrying(new ApiError("Down", 503, "Down"))).toBe(true);
    expect(worthRetrying(new ApiError("Slow down", 429, "Slow down"))).toBe(true);
    expect(worthRetrying(new ApiError("Timeout", 408, null))).toBe(true);
  });

  it("does not retry an answer that will be the same next time", () => {
    expect(worthRetrying(new ApiError("No such book.", 404, "No such book."))).toBe(false);
    expect(worthRetrying(new ApiError("Bad id", 400, "Bad id"))).toBe(false);
    expect(worthRetrying(new ApiError("Not found", 403, "Not found"))).toBe(false);
  });
});
