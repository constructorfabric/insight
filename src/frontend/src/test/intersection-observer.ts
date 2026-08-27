/**
 * jsdom implements no `IntersectionObserver` at all, so anything that loads on
 * scroll would silently never fire under test. This is the smallest stand-in
 * that behaves like the real thing in the one way the loading code depends on:
 * **`observe()` reports the CURRENT state**, it does not wait for a crossing.
 * That is what lets a list keep loading while its end stays in view, and a stub
 * that only fired on demand would let that regress unnoticed.
 */
type Callback = (entries: Array<{ isIntersecting: boolean }>) => void;

const watching = new Set<Callback>();
let endInView = false;

/** The end of the list is on screen — and stays there until told otherwise. */
export function scrollEndIntoView(): void {
  endInView = true;
  for (const callback of [...watching]) {
    callback([{ isIntersecting: true }]);
  }
}

/** The reader scrolled away, or rows pushed the end past the edge. */
export function scrollEndOutOfView(): void {
  endInView = false;
  for (const callback of [...watching]) {
    callback([{ isIntersecting: false }]);
  }
}

export function installIntersectionObserver(): void {
  class Stub implements IntersectionObserver {
    readonly root = null;
    readonly rootMargin = "";
    readonly scrollMargin = "";
    readonly thresholds: ReadonlyArray<number> = [];
    #callback: Callback;

    constructor(callback: Callback) {
      this.#callback = callback;
    }
    observe(): void {
      watching.add(this.#callback);
      // The platform delivers an initial record for whatever the target's
      // state already is. Re-observing is therefore how the loading code
      // re-checks a marker that never moved.
      if (endInView) this.#callback([{ isIntersecting: true }]);
    }
    unobserve(): void {
      watching.delete(this.#callback);
    }
    disconnect(): void {
      watching.delete(this.#callback);
    }
    takeRecords(): IntersectionObserverEntry[] {
      return [];
    }
  }
  globalThis.IntersectionObserver = Stub as unknown as typeof IntersectionObserver;
}
