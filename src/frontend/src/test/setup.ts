/**
 * Vitest setup. Loaded once per test process per `vitest.config.ts`.
 *
 * Brings in `@testing-library/jest-dom` matchers so component tests can use
 * `expect(node).toBeInTheDocument()` and friends. Other global setup goes
 * here as it accrues.
 */

import "@testing-library/jest-dom/vitest";
import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// jsdom in vitest's worker pool can race against module init: modules
// that read `window.localStorage` at top level (e.g. `use-settings.ts`)
// occasionally see `window.localStorage` as undefined when imported
// before jsdom finishes wiring the Storage globals. Define a minimal
// in-memory Storage shim before any user module loads so component
// tests that pull in `useSettings()` transitively don't crash.
if (typeof window !== "undefined" && !window.localStorage) {
  const store = new Map<string, string>();
  const shim: Storage = {
    get length() {
      return store.size;
    },
    clear: () => store.clear(),
    getItem: (k) => store.get(k) ?? null,
    key: (i) => Array.from(store.keys())[i] ?? null,
    removeItem: (k) => {
      store.delete(k);
    },
    setItem: (k, v) => {
      store.set(k, String(v));
    },
  };
  Object.defineProperty(window, "localStorage", {
    value: shim,
    configurable: true,
  });
}

afterEach(() => {
  cleanup();
});
