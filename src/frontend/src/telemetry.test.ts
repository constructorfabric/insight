import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ logEvent: vi.fn() }));

vi.mock("@gears-frontx/telemetry", () => {
  const service = {
    identify: () => service,
    start: () => service,
    logEvent: mocks.logEvent,
    destroy: () => {},
  };
  return { createTelemetry: () => service };
});

vi.mock("@/api/usage-client", () => ({
  getUsageConfig: () => Promise.resolve({ enabled: true }),
}));

import {
  recordPageView,
  recordUsageEvent,
  screenPath,
  scopeLabel,
  startUsageTelemetry,
} from "./telemetry";

const SESSION = {
  personId: "p1",
  impersonatorEmail: null,
} as unknown as Parameters<typeof startUsageTelemetry>[0];

describe("screenPath", () => {
  it("drops the person a page is about", () => {
    expect(
      screenPath("/ic/cccccccc-0000-0000-0000-000000000001/personal/git_output"),
    ).toBe("/ic/:id/personal/git_output");
  });

  it("drops a person key whatever shape it arrives in", () => {
    expect(screenPath("/ic/alice.kim@example.com/personal")).toBe("/ic/:id/personal");
    expect(screenPath("/ic/alice.kim%40example.com/team")).toBe("/ic/:id/team");
  });

  it("leaves a path that names no one alone", () => {
    expect(screenPath("/portal/manage/platform-usage")).toBe(
      "/portal/manage/platform-usage",
    );
  });
});

describe("startUsageTelemetry", () => {
  beforeEach(() => {
    mocks.logEvent.mockClear();
  });

  it("keeps the page views that happen while it is still starting", async () => {
    recordPageView("/portal/people");
    await startUsageTelemetry(SESSION);

    expect(mocks.logEvent).toHaveBeenCalledWith("page_view", { path: "/portal/people" });
  });
});

describe("recordUsageEvent", () => {
  beforeEach(async () => {
    await startUsageTelemetry(SESSION);
    mocks.logEvent.mockClear();
  });

  it("reports the screen the action happened on", () => {
    recordPageView("/portal/overview/trend");
    recordUsageEvent("drill", "git.commits");

    expect(mocks.logEvent).toHaveBeenCalledWith("drill", {
      target: "git.commits",
      path: "/portal/overview/trend",
    });
  });

  it("reports the screen without the person it is about", () => {
    recordPageView("/ic/cccccccc-0000-0000-0000-000000000001/personal/git_output");
    recordUsageEvent("period", "month");

    expect(mocks.logEvent).toHaveBeenCalledWith("period", {
      target: "month",
      path: "/ic/:id/personal/git_output",
    });
  });
});

describe("scopeLabel", () => {
  it("reports the shape of a scope, never who it is rooted at", () => {
    const person = "cccccccc-0000-0000-0000-000000000001";
    expect(scopeLabel({ root: person, directOnly: false })).toBe("subtree");
    expect(scopeLabel({ root: person, directOnly: true })).toBe("subtree-direct");
    expect(scopeLabel({ root: null, directOnly: false })).toBe("whole-org");
  });

  it("names the attribute a filter is on, not the person behind it", () => {
    expect(
      scopeLabel({
        root: null,
        directOnly: false,
        attrFilter: { key: "department", value: "Engineering" },
      }),
    ).toBe("attr:department");
  });
});
