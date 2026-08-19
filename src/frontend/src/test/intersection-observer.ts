/**
 * jsdom implements no `IntersectionObserver` at all, so anything that loads on
 * scroll would silently never fire under test. This is the smallest stand-in
 * that lets a test say "the end of the list came into view".
 */
type Callback = (entries: Array<{ isIntersecting: boolean }>) => void;

const watching = new Set<Callback>();

/** Report every observed element as visible — the end of a list scrolled to. */
export function scrollEndIntoView(): void {
  for (const callback of [...watching]) {
    callback([{ isIntersecting: true }]);
  }
}

export function installIntersectionObserver(): void {
  class Stub implements IntersectionObserver {
    readonly root = null;
    readonly rootMargin = "";
    readonly thresholds: ReadonlyArray<number> = [];
    #callback: Callback;

    constructor(callback: Callback) {
      this.#callback = callback;
    }
    observe(): void {
      watching.add(this.#callback);
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
