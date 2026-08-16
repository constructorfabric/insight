// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ recordUsageEvent: vi.fn() }));

vi.mock("@/telemetry", async () => {
  const actual = await vi.importActual<typeof import("@/telemetry")>("@/telemetry");
  return { ...actual, recordUsageEvent: mocks.recordUsageEvent };
});

import { usePortalNavActions } from "./portal-nav";

describe("filter usage", () => {
  beforeEach(() => {
    mocks.recordUsageEvent.mockClear();
  });

  it("reports the cohort a reader picked", () => {
    const { result } = renderHook(() => usePortalNavActions());

    act(() => result.current.setSlice("department"));

    expect(mocks.recordUsageEvent).toHaveBeenCalledWith("cohort", "department");
  });

  it("reports how a scope was narrowed, never who it is rooted at", () => {
    const person = "cccccccc-1111-4111-8111-111111111111";
    const { result } = renderHook(() => usePortalNavActions());

    act(() => result.current.setScope({ root: person, directOnly: true }));

    expect(mocks.recordUsageEvent).toHaveBeenCalledWith("scope", "subtree-direct");
    const reported = mocks.recordUsageEvent.mock.calls.flat().join(" ");
    expect(reported).not.toContain(person);
  });

});
