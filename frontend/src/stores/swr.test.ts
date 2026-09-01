import { describe, expect, it, vi } from "vitest";
import { ApiError } from "../api/client";
import { STALE_AFTER, swrLoad, swrState } from "./swr";

describe("swrLoad", () => {
  it("fetches when empty and caches", async () => {
    const state = swrState<number>();
    const fetcher = vi.fn().mockResolvedValue(1);
    expect(await swrLoad(state, fetcher)).toBe(1);
    expect(state.data).toBe(1);
    expect(state.pending).toBe(false);
    expect(await swrLoad(state, fetcher)).toBe(1);
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("returns stale data immediately and revalidates in the background", async () => {
    const state = swrState<number>();
    const fetcher = vi.fn().mockResolvedValueOnce(1).mockResolvedValueOnce(2);
    await swrLoad(state, fetcher);
    state.loadedAt = Date.now() - STALE_AFTER - 1;
    const result = await swrLoad(state, fetcher);
    expect(result).toBe(1);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(state.data).toBe(2);
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("force always awaits the network", async () => {
    const state = swrState<number>();
    const fetcher = vi.fn().mockResolvedValueOnce(1).mockResolvedValueOnce(5);
    await swrLoad(state, fetcher);
    expect(await swrLoad(state, fetcher, true)).toBe(5);
  });

  it("surfaces the error when nothing is cached, keeps data otherwise", async () => {
    const state = swrState<number>();
    const boom = vi.fn().mockRejectedValue(new ApiError("nope", 500, null));
    await expect(swrLoad(state, boom)).rejects.toBeInstanceOf(ApiError);
    expect(state.error).toBe("nope");

    state.data = 3;
    state.loadedAt = 0;
    const fallbackRun = vi.fn().mockRejectedValue(new Error("network"));
    expect(await swrLoad(state, fallbackRun, false, "fallback text")).toBe(3);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(state.error).toBe("fallback text");
    expect(state.data).toBe(3);
  });
});
