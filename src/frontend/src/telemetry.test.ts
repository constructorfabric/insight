/**
 * A recorded path names a screen, never a person: `/ic/<id>/personal` is one
 * screen whoever it is about.
 */
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

import { recordPageView, screenPath, startUsageTelemetry } from "./telemetry";

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
    // The SDK cannot start until the instance says collection is on, and the
    // reader has already opened a page by then.
    recordPageView("/portal/people");
    await startUsageTelemetry(SESSION);

    expect(mocks.logEvent).toHaveBeenCalledWith("page_view", { path: "/portal/people" });
  });
});
