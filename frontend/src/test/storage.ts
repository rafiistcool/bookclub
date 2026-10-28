// The test runtime's own `localStorage` needs a backing file, so tests that
// touch storage stub it with one of these (`vi.stubGlobal`).

export function memoryStorage(): Storage {
  const data = new Map<string, string>();
  return {
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => void data.set(key, String(value)),
    removeItem: (key) => void data.delete(key),
    clear: () => data.clear(),
    key: (index) => [...data.keys()][index] ?? null,
    get length() {
      return data.size;
    },
  };
}

/** Storage that refuses everything, like a private window with storage blocked. */
export function blockedStorage(): Storage {
  const refuse = () => {
    throw new DOMException("blocked", "SecurityError");
  };
  return {
    getItem: refuse,
    setItem: refuse,
    removeItem: refuse,
    clear: refuse,
    key: refuse,
    get length(): number {
      return refuse();
    },
  };
}
