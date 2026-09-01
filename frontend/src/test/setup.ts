import { config } from "@vue/test-utils";
import { vi } from "vitest";

// Components use <RouterLink> freely; stub it as a plain anchor in unit tests.
config.global.stubs = {
  RouterLink: {
    props: ["to"],
    template: '<a :href="typeof to === \'string\' ? to : \'#\'"><slot /></a>',
  },
};

if (!("matchMedia" in window)) {
  Object.defineProperty(window, "matchMedia", {
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
}

if (!("IntersectionObserver" in window)) {
  class FakeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.defineProperty(window, "IntersectionObserver", { writable: true, value: FakeObserver });
}
