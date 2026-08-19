// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { portalRouter } from "@/test/portal-router";

import { usePortalNavActions } from "./portal-nav";

describe("one navigation per intent", () => {
  beforeEach(() => {
    act(() => portalRouter.reset());
  });

  it("opens a direction, its lens and the zone together", () => {
    const { result } = renderHook(() => usePortalNavActions());

    act(() => result.current.openDirection("wiki", "Authoring"));

    expect(portalRouter.navigations).toHaveLength(1);
    expect(portalRouter.search).toMatchObject({
      zone: "directions",
      dir: "wiki",
      lens: "Authoring",
    });
  });

  it("changes a lens without a second navigation to drop the stale item", () => {
    act(() => portalRouter.set({ zone: "directions", dir: "dev", item: "trend" }));
    const { result } = renderHook(() => usePortalNavActions());
    portalRouter.navigations.length = 0;

    act(() => result.current.setLens("Delivery"));

    expect(portalRouter.navigations).toHaveLength(1);
    expect(portalRouter.search.lens).toBe("Delivery");
    expect(portalRouter.search.item).toBeUndefined();
  });
});
