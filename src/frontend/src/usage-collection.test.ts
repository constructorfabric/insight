import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RouterHistory } from "@tanstack/react-router";

const mocks = vi.hoisted(() => ({ recordPageView: vi.fn() }));

vi.mock("@/telemetry", () => ({ recordPageView: mocks.recordPageView }));

import { recordScreens } from "./usage-collection";

function historyStub(
  pathname: string,
  search = "",
): {
  history: RouterHistory;
  navigateRouterOnly: (pathname: string, search?: string) => void;
} {
  window.history.pushState({}, "", `${pathname}${search}`);
  const subscribers = new Set<() => void>();
  const location = { pathname, search };
  const history = {
    location,
    subscribe: (cb: () => void) => {
      subscribers.add(cb);
      return () => subscribers.delete(cb);
    },
  } as unknown as RouterHistory;
  return {
    history,
    navigateRouterOnly: (nextPathname: string, nextSearch = "") => {
      location.pathname = nextPathname;
      location.search = nextSearch;
      subscribers.forEach((cb) => cb());
    },
  };
}

describe("recordScreens", () => {
  beforeEach(() => {
    mocks.recordPageView.mockClear();
  });

  it("records the screen it starts on", () => {
    recordScreens(historyStub("/home").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/home");
  });

  it("records a screen once however often history fires", () => {
    const { history, navigateRouterOnly } = historyStub("/explore/dev/flow");
    recordScreens(history);
    navigateRouterOnly("/explore/dev/flow");

    expect(mocks.recordPageView).toHaveBeenCalledTimes(1);
  });

  it("records a page change as the screen it opened, not the one it left", () => {
    const { history, navigateRouterOnly } = historyStub("/home/directions");
    recordScreens(history);
    mocks.recordPageView.mockClear();

    navigateRouterOnly("/manage/identities");

    expect(mocks.recordPageView).toHaveBeenCalledWith("/manage/identities");
  });

  it("does not count a filter change as a new screen", () => {
    const { history, navigateRouterOnly } = historyStub("/explore/dev/repositories");
    recordScreens(history);
    navigateRouterOnly("/explore/dev/repositories", "?repo=src-a%3Aacme%2Fapi&period=quarter");

    expect(mocks.recordPageView).toHaveBeenCalledTimes(1);
  });
});
