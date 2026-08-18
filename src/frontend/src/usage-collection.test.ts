import { beforeEach, describe, expect, it, vi } from "vitest";
import { createBrowserHistory } from "@tanstack/react-router";

const mocks = vi.hoisted(() => ({ recordPageView: vi.fn() }));

vi.mock("@/telemetry", () => ({ recordPageView: mocks.recordPageView }));

import { recordScreens } from "./usage-collection";

// WORKAROUND: the router calls `win.history.pushState` unbound, which jsdom
// rejects and real browsers accept.
window.history.pushState = window.history.pushState.bind(window.history);
window.history.replaceState = window.history.replaceState.bind(window.history);

describe("recordScreens", () => {
  beforeEach(() => {
    mocks.recordPageView.mockClear();
    window.history.replaceState(null, "", "/portal?zone=overview");
  });

  it("records the screen the reader arrives at, not the one they left", () => {
    const history = createBrowserHistory();
    const stop = recordScreens(history);
    mocks.recordPageView.mockClear();

    history.push("/portal?zone=manage&item=whats-new");

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/manage/whats-new");
    stop();
    history.destroy();
  });

  it("records the screen the reader lands on", () => {
    const history = createBrowserHistory();

    const stop = recordScreens(history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/overview");
    stop();
    history.destroy();
  });
});
