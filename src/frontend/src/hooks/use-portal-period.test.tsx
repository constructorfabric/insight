// @vitest-environment jsdom
/**
 * The period lives in the URL like the rest of the portal's navigation state,
 * with one exception: the LAST preset a reader picked is remembered, so a link
 * that names no period opens on their habit rather than on a hardcoded week.
 * A custom range that is not a range at all must degrade, not throw.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { renderHook } from "@testing-library/react";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { portalRouter } from "@/test/portal-router";

import { writePeriodPreference } from "./use-period";
import { usePortalPeriod } from "./use-portal-period";

beforeEach(() => {
  window.localStorage.clear();
  act(() => {
    portalRouter.reset();
    // The preference is an in-memory store as well as a storage key, so
    // clearing localStorage alone leaves the previous test's choice behind and
    // the next test inherits it as the default for a URL naming no period.
    writePeriodPreference("week");
  });
});

describe("usePortalPeriod", () => {
  it("puts a chosen preset in the URL and clears any custom range", () => {
    const { result } = renderHook(() => usePortalPeriod());
    act(() => result.current.setPeriod("month"));
    expect(portalRouter.search.period).toBe("month");
    expect(portalRouter.search.from).toBeUndefined();
    expect(portalRouter.search.to).toBeUndefined();
  });

  it("writes a valid custom range to the URL", () => {
    const { result } = renderHook(() => usePortalPeriod());
    act(() => result.current.setCustomRange({ from: "2026-07-01", to: "2026-07-15" }));
    expect(portalRouter.search.from).toBe("2026-07-01");
    expect(portalRouter.search.to).toBe("2026-07-15");
  });

  it("drops an inverted range instead of throwing out of the handler", () => {
    // This runs in an event handler, where no error boundary is watching: a
    // throw here takes the screen down. The picker keeps its own message.
    const { result } = renderHook(() => usePortalPeriod());
    expect(() =>
      act(() => result.current.setCustomRange({ from: "2026-07-30", to: "2026-01-01" })),
    ).not.toThrow();
    expect(portalRouter.search.from).toBeUndefined();
    expect(portalRouter.search.to).toBeUndefined();
  });
});
