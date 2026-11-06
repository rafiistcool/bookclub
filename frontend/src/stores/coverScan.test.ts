import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it } from "vitest";
import { useCoverScan } from "./coverScan";

describe("useCoverScan", () => {
  beforeEach(() => setActivePinia(createPinia()));

  it("opens for a photo and resolves with what the scan screen chose", async () => {
    const scan = useCoverScan();
    const photo = new File(["p"], "photo.jpg", { type: "image/jpeg" });
    const flat = new File(["f"], "cover.jpg", { type: "image/jpeg" });
    const answer = scan.request(photo);
    expect(scan.file).toBe(photo);
    scan.finish(flat);
    expect(scan.file).toBeNull();
    await expect(answer).resolves.toBe(flat);
  });

  it("takes one photo at a time: a second request while open resolves null", async () => {
    const scan = useCoverScan();
    const first = new File(["1"], "one.jpg");
    const second = new File(["2"], "two.jpg");
    const answer = scan.request(first);
    await expect(scan.request(second)).resolves.toBeNull();
    expect(scan.file).toBe(first);
    scan.finish(null);
    await expect(answer).resolves.toBeNull();
    // Closed again, the next photo opens.
    const next = scan.request(second);
    expect(scan.file).toBe(second);
    scan.finish(second);
    await expect(next).resolves.toBe(second);
  });

  it("notes what had focus when the photo was requested, and forgets it on finish", async () => {
    const scan = useCoverScan();
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();
    const answer = scan.request(new File(["p"], "photo.jpg"));
    // The opener may be disabled or blurred by the time the scan screen opens.
    input.disabled = true;
    input.blur();
    expect(scan.opener).toBe(input);
    scan.finish(null);
    await answer;
    expect(scan.opener).toBeNull();
    input.remove();
  });

  it("notes no opener when nothing had focus", () => {
    const scan = useCoverScan();
    (document.activeElement as HTMLElement | null)?.blur();
    void scan.request(new File(["p"], "photo.jpg"));
    expect(scan.opener).toBeNull();
    scan.finish(null);
  });

  it("ignores a finish with nothing open", () => {
    const scan = useCoverScan();
    expect(() => scan.finish(null)).not.toThrow();
    expect(scan.file).toBeNull();
  });
});
