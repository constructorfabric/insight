import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RouterHistory } from "@tanstack/react-router";

const mocks = vi.hoisted(() => ({ recordPageView: vi.fn() }));

vi.mock("@/telemetry", () => ({ recordPageView: mocks.recordPageView }));

import { recordScreens } from "./usage-collection";

function historyStub(): { history: RouterHistory; fire: () => void } {
  const subscribers = new Set<() => void>();
  const history = {
    subscribe: (cb: () => void) => {
      subscribers.add(cb);
      return () => subscribers.delete(cb);
    },
  } as unknown as RouterHistory;
  return { history, fire: () => subscribers.forEach((cb) => cb()) };
}

describe("recordScreens", () => {
  beforeEach(() => {
    mocks.recordPageView.mockClear();
  });

  it("records the screen it starts on", () => {
    window.history.pushState({}, "", "/portal");
    recordScreens(historyStub().history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal");
  });

  it("composes the portal zone and item into the screen", () => {
    window.history.pushState({}, "", "/portal?zone=overview&item=trend");
    recordScreens(historyStub().history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/overview/trend");
  });

  it("records a screen once however often history fires", () => {
    window.history.pushState({}, "", "/portal?zone=directions");
    const { history, fire } = historyStub();
    recordScreens(history);
    fire();
    fire();

    expect(mocks.recordPageView).toHaveBeenCalledTimes(1);
  });
});
