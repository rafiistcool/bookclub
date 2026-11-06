// A stand-in for stores/coverScan in tests that upload a cover photo: no
// scanner is mounted there, so the scan hands every photo back as it is,
// unless a test sets `scanAnswer` (null = the reader cancelled).
//
//   vi.mock("../stores/coverScan", () => import("../test/coverScan"));

let answer: ((file: File) => File | null) | null = null;
export const scanned: File[] = [];

/** Answer the next scans with this instead of the photo itself; `resetScan` undoes it. */
export function scanAnswer(next: (file: File) => File | null) {
  answer = next;
}

export function resetScan() {
  answer = null;
  scanned.length = 0;
}

export function useCoverScan() {
  return {
    file: null,
    opener: null,
    async request(file: File): Promise<File | null> {
      scanned.push(file);
      return answer ? answer(file) : file;
    },
    finish() {},
  };
}
