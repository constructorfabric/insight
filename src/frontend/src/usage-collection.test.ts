import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ recordPageView: vi.fn() }));

vi.mock("@/telemetry", () => ({ recordPageView: mocks.recordPageView }));

import { recordScreens } from "./usage-collection";

type Resolved = (event: { toLocation: { pathname: string } }) => void;

/** A router that resolves the navigations a test hands it. */
function routerStub() {
  const listeners: Resolved[] = [];
  return {
    subscribe: ((event: string, listener: Resolved) => {
      if (event === "onResolved") listeners.push(listener);
      return () => {};
    }) as never,
    resolve: (pathname: string) => {
      for (const listener of listeners) listener({ toLocation: { pathname } });
    },
  };
}

describe("recordScreens", () => {
  beforeEach(() => {
    mocks.recordPageView.mockClear();
  });

  it("records the screen a navigation resolves to", () => {
    const router = routerStub();
    recordScreens(router);

    router.resolve("/home/directions");

    expect(mocks.recordPageView).toHaveBeenCalledWith("/home/directions");
  });

  it("records nothing before the router resolves, so a redirected path never lands", () => {
    recordScreens(routerStub());

    expect(mocks.recordPageView).not.toHaveBeenCalled();
  });

  it("records a screen once however often it resolves", () => {
    const router = routerStub();
    recordScreens(router);

    router.resolve("/explore/dev/flow");
    router.resolve("/explore/dev/flow");

    expect(mocks.recordPageView).toHaveBeenCalledTimes(1);
  });

  it("records a page change as the screen it opened", () => {
    const router = routerStub();
    recordScreens(router);
    router.resolve("/home");
    mocks.recordPageView.mockClear();

    router.resolve("/manage/identities");

    expect(mocks.recordPageView).toHaveBeenCalledWith("/manage/identities");
  });
});
