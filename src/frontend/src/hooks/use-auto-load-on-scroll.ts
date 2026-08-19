import { useCallback, useEffect, useRef } from "react";

/**
 * Load the next page when the end of a list scrolls into view.
 *
 * Attach the returned ref to a marker element after the last row. It is
 * observed rather than measured, so a list too short to scroll still asks for
 * its next page — a scroll handler would never fire there and the reader would
 * be left with a list that has more to give and no way to ask for it.
 *
 * `root` is the scrolling element, when the list scrolls inside one rather than
 * with the page; passing the wrong one only widens the margin at which the
 * marker counts as visible.
 */
export function useAutoLoadOnScroll({
  hasNextPage,
  isFetchingNextPage,
  fetchNextPage,
  root,
}: {
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  fetchNextPage: () => unknown;
  root?: React.RefObject<HTMLElement | null>;
}): (node: HTMLElement | null) => void {
  const marker = useRef<HTMLElement | null>(null);
  // Read through a ref inside the observer: the callback outlives the render it
  // was created in, and a stale `hasNextPage` there would either stop loading
  // for good or ask for a page past the last one.
  const state = useRef({ hasNextPage, isFetchingNextPage, fetchNextPage });
  useEffect(() => {
    state.current = { hasNextPage, isFetchingNextPage, fetchNextPage };
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  useEffect(() => {
    const node = marker.current;
    if (node === null) return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        const { hasNextPage: more, isFetchingNextPage: busy, fetchNextPage: next } =
          state.current;
        if (!more || busy) return;
        next();
      },
      // Ahead of the edge, so the next page is on its way before the reader
      // reaches the bottom rather than after they have already stopped there.
      { root: root?.current ?? null, rootMargin: "200px" },
    );
    observer.observe(node);
    return () => observer.disconnect();
    // `hasNextPage` re-runs it because the marker unmounts with the last page:
    // the observer has to pick the new node up when there is more to load.
  }, [hasNextPage, root]);

  return useCallback((node: HTMLElement | null) => {
    marker.current = node;
  }, []);
}
