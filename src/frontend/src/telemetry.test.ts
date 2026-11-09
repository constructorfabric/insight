import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ logEvent: vi.fn(), createTelemetry: vi.fn() }));

vi.mock("@gears-frontx/telemetry", () => {
  const service = {
    identify: () => service,
    start: () => service,
    logEvent: mocks.logEvent,
    destroy: () => {},
  };
  mocks.createTelemetry.mockImplementation(() => service);
  return { createTelemetry: mocks.createTelemetry };
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
      screenPath("/people/cccccccc-0000-0000-0000-000000000001/git-output"),
    ).toBe("/people/:id/git-output");
  });

  it("drops a person key whatever shape it arrives in", () => {
    expect(screenPath("/people/alice.kim@example.com")).toBe("/people/:id");
    expect(screenPath("/people/alice.kim%40example.com/team")).toBe("/people/:id/team");
  });

  it("leaves a path that names no one alone", () => {
    expect(screenPath("/manage/usage")).toBe("/manage/usage");
  });

  it.each([
    ["/dashboards", "/dashboards"],
    ["/dashboards/", "/dashboards/"],
    ["/dashboards/q3-hiring-plan", "/dashboards/:id"],
    ["/exp/preview/dashboards/q3-hiring-plan", "/exp/preview/dashboards/:id"],
    ["/dashboards/Board%20for%20Jane", "/dashboards/:id"],
    ["/dashboards/metrics", "/dashboards/metrics"],
    ["/dashboards/widgets", "/dashboards/widgets"],
    ["/dashboards/datasets", "/dashboards/datasets"],
    ["/dashboards/alerts", "/dashboards/alerts"],
    ["/dashboards/destinations", "/dashboards/destinations"],
    ["/dashboards/new/dashboards", "/dashboards/new/dashboards"],
    ["/dashboards/edit/metrics/m", "/dashboards/edit/metrics/m"],
    ["/dashboards/edit/dashboards/q3-plan", "/dashboards/edit/dashboards/:id"],
    [
      "/dashboards/alerts/cccccccc-0000-0000-0000-000000000001",
      "/dashboards/alerts/:id",
    ],
  ])("names a custom page %s as %s", (path, shown) => {
    expect(screenPath(path), `path: ${path}`).toBe(shown);
  });
});

describe("startUsageTelemetry", () => {
  beforeEach(() => {
    mocks.logEvent.mockClear();
  });

  it("keeps the page views that happen while it is still starting", async () => {
    recordPageView("/people");
    await startUsageTelemetry(SESSION);

    expect(mocks.logEvent).toHaveBeenCalledWith("page_view", { path: "/people" });
  });
});

describe("recordUsageEvent", () => {
  beforeEach(async () => {
    await startUsageTelemetry(SESSION);
    mocks.logEvent.mockClear();
  });

  it("reports the screen the action happened on", () => {
    recordPageView("/home/directions");
    recordUsageEvent("drill", "git.commits");

    expect(mocks.logEvent).toHaveBeenCalledWith("drill", {
      target: "git.commits",
      path: "/home/directions",
    });
  });

  it("reports the screen without the person it is about", () => {
    recordPageView("/people/cccccccc-0000-0000-0000-000000000001/git-output");
    recordUsageEvent("period", "month");

    expect(mocks.logEvent).toHaveBeenCalledWith("period", {
      target: "month",
      path: "/people/:id/git-output",
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

describe("a session opened on somebody else's behalf", () => {
  // #2573 scenario 11 — a fresh module, because a collector another test already
  // started would short-circuit the guard under test and pass for that reason.
  it("starts no collector and sends nothing, for either person", async () => {
    vi.resetModules();
    mocks.createTelemetry.mockClear();
    mocks.logEvent.mockClear();

    const telemetry = await import("./telemetry");
    await telemetry.startUsageTelemetry({
      personId: "p1",
      impersonatorEmail: "operator@example.com",
    } as unknown as Parameters<typeof startUsageTelemetry>[0]);
    telemetry.recordPageView("/people");
    telemetry.recordUsageEvent("drill", "git.commits");

    expect(mocks.createTelemetry).not.toHaveBeenCalled();
    expect(mocks.logEvent).not.toHaveBeenCalled();
  });
});
