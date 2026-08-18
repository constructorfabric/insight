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
    recordScreens(historyStub("/portal").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal");
  });

  it("composes the portal zone and item into the screen", () => {
    recordScreens(historyStub("/portal", "?zone=overview&item=trend").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/overview/trend");
  });

  it("records a screen once however often history fires", () => {
    const { history, navigateRouterOnly } = historyStub("/portal", "?zone=directions");
    recordScreens(history);
    navigateRouterOnly("/portal", "?zone=directions");

    expect(mocks.recordPageView).toHaveBeenCalledTimes(1);
  });

  it("records a zone change as the screen it opened, not the one it left", () => {
    const { history, navigateRouterOnly } = historyStub("/portal", "?zone=overview&item=trend");
    recordScreens(history);
    mocks.recordPageView.mockClear();

    navigateRouterOnly("/portal", "?zone=manage&item=identities");

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/manage/identities");
  });

  it("composes a person group item into the screen", () => {
    recordScreens(historyStub("/ic/:id/personal", "?item=git_output").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/ic/:id/personal/git_output");
  });

  it("drops a zone that names no screen in the rail", () => {
    recordScreens(historyStub("/portal", "?zone=user@example.com").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal");
  });

  it("drops an item that names no screen in the rail", () => {
    recordScreens(historyStub("/portal", "?zone=manage&item=user@example.com").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/manage");
  });
});
