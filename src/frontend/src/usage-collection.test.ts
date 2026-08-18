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
  navigate: (pathname: string, search?: string) => void;
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
    // `window` stays on the previous screen, as it does in the real router.
    navigate: (nextPathname: string, nextSearch = "") => {
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
    const { history, navigate } = historyStub("/portal", "?zone=directions");
    recordScreens(history);
    navigate("/portal", "?zone=directions");

    expect(mocks.recordPageView).toHaveBeenCalledTimes(1);
  });

  it("records a zone change as the screen it opened, not the one it left", () => {
    const { history, navigate } = historyStub("/portal", "?zone=overview&item=trend");
    recordScreens(history);
    mocks.recordPageView.mockClear();

    navigate("/portal", "?zone=manage&item=identities");

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/manage/identities");
  });
});
