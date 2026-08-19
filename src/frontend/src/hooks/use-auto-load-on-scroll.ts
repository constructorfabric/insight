import { useEffect, useRef, useState } from "react";

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
  // State, not a ref: the observer has to be re-created for a marker element
  // that was replaced — remounted with the list it sits in, say — and a ref
  // would leave it watching a node no longer in the document, which reports
  // nothing and stops the list loading for good.
  const [marker, setMarker] = useState<HTMLElement | null>(null);
  // Read through a ref inside the observer: the callback outlives the render it
  // was created in, and a stale `hasNextPage` there would either stop loading
  // for good or ask for a page past the last one.
  const state = useRef({ hasNextPage, isFetchingNextPage, fetchNextPage });
  useEffect(() => {
    state.current = { hasNextPage, isFetchingNextPage, fetchNextPage };
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  useEffect(() => {
    if (marker === null) return;

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
    observer.observe(marker);
    return () => observer.disconnect();
    // INVARIANT: every dep here exists to RE-OBSERVE, which is the only way to
    // re-read a marker that never moved — an observer reports a crossing, and
    // `observe()` reporting the current state is what stands in for one.
    //
    // `isFetchingNextPage` is the load-bearing one. A page that lands without
    // pushing the marker out of view — every row filtered out by `excludeIds`,
    // or a list shorter than its own scroller — crosses nothing, so without
    // re-observing here the list would stop after exactly one page.
  }, [marker, hasNextPage, isFetchingNextPage, root]);

  return setMarker;
}
