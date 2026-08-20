// @vitest-environment jsdom
/**
 * Loading a list as it scrolls. What matters: the end of the list coming into
 * view asks for the next page; a page already in flight is not asked for twice;
 * and — the one that is easy to get wrong — a list whose end STAYS in view keeps
 * loading. An observer reports a crossing, so a page that lands without pushing
 * the marker past the edge (every row filtered out, or a list shorter than its
 * own scroller) crosses nothing, and a hook that only listens for crossings
 * stops after exactly one page.
 */
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import {
  scrollEndIntoView,
  scrollEndOutOfView,
} from "@/test/intersection-observer";

import { useAutoLoadOnScroll } from "./use-auto-load-on-scroll";

interface Props {
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
}

function harness(initialProps: Props = {
  hasNextPage: true,
  isFetchingNextPage: false,
}) {
  const fetchNextPage = vi.fn();
  const view = renderHook(
    (props: Props) => useAutoLoadOnScroll({ ...props, fetchNextPage }),
    { initialProps },
  );
  // The marker the caller puts after its last row.
  act(() => view.result.current(document.createElement("div")));
  return { ...view, fetchNextPage };
}

describe("useAutoLoadOnScroll", () => {
  it("asks for the next page when the end of the list comes into view", () => {
    const { fetchNextPage } = harness();

    act(scrollEndIntoView);

    expect(fetchNextPage).toHaveBeenCalledTimes(1);
  });

  it("asks for nothing while the end is out of view", () => {
    const { fetchNextPage } = harness();

    act(scrollEndOutOfView);

    expect(fetchNextPage).not.toHaveBeenCalled();
  });

  // The case a crossing-only hook fails: nothing moved, so nothing crossed, and
  // the reader is left on a list that has more to give and no way to ask.
  it("keeps loading while the end stays in view", () => {
    const { rerender, fetchNextPage } = harness();

    act(scrollEndIntoView);
    expect(fetchNextPage).toHaveBeenCalledTimes(1);

    // The page it asked for is on its way, then lands — with the marker exactly
    // where it was.
    act(() => rerender({ hasNextPage: true, isFetchingNextPage: true }));
    expect(fetchNextPage).toHaveBeenCalledTimes(1);
    act(() => rerender({ hasNextPage: true, isFetchingNextPage: false }));
    expect(fetchNextPage).toHaveBeenCalledTimes(2);

    // And again, for as long as there is more to read.
    act(() => rerender({ hasNextPage: true, isFetchingNextPage: true }));
    act(() => rerender({ hasNextPage: true, isFetchingNextPage: false }));
    expect(fetchNextPage).toHaveBeenCalledTimes(3);
  });

  it("stops the moment the listing says there is no next page", () => {
    const { rerender, fetchNextPage } = harness();

    act(scrollEndIntoView);
    act(() => rerender({ hasNextPage: true, isFetchingNextPage: true }));
    act(() => rerender({ hasNextPage: false, isFetchingNextPage: false }));

    expect(fetchNextPage).toHaveBeenCalledTimes(1);
  });

  // A marker remounted with the list it sits in must be picked up, or the
  // observer is left watching a node no longer in the document.
  it("follows the marker to a new element", () => {
    const { result, fetchNextPage } = harness();

    act(scrollEndIntoView);
    expect(fetchNextPage).toHaveBeenCalledTimes(1);

    act(() => result.current(document.createElement("div")));

    expect(fetchNextPage).toHaveBeenCalledTimes(2);
  });
});
