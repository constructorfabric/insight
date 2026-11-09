// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/auth", () => ({ useViewer: () => ({ personId: "me-1" }) }));

import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { portalRouter } from "@/test/portal-router";

import { usePortalNavActions } from "./portal-nav";

describe("one navigation per intent", () => {
  beforeEach(() => {
    act(() => portalRouter.reset());
  });

  it("opens a direction on a lens at its own path", () => {
    const { result } = renderHook(() => usePortalNavActions());

    act(() => result.current.openDirection("wiki", "Authoring"));

    expect(portalRouter.navigations).toHaveLength(1);
    expect(portalRouter.pathname).toBe("/explore/wiki/authoring");
  });

  it("closes a direction back to Explore", () => {
    act(() => portalRouter.go("/explore/dev/delivery"));
    const { result } = renderHook(() => usePortalNavActions());

    act(() => result.current.openDirection(""));

    expect(portalRouter.pathname).toBe("/explore");
  });

  it.each([
    ["/home", "attention", "/home/attention"],
    ["/home/attention", null, "/home"],
    ["/explore/ai", "idle-seats", "/explore/ai/seats"],
    ["/manage/assistant", "whats-new", "/manage/news"],
    ["/people/p-1", "git_output", "/people/p-1/git-output"],
    ["/people/p-1/team", "metrics", "/people/p-1/team/metrics"],
  ])("from %s, opens item %s at %s", (from, item, to) => {
    act(() => portalRouter.go(from));
    const { result } = renderHook(() => usePortalNavActions());

    act(() => result.current.setItem(item));

    expect(portalRouter.pathname, `${from} + ${item}`).toBe(to);
  });

  it("keeps the reader's place when only the page's setup changes", () => {
    const { result } = renderHook(() => usePortalNavActions());

    act(() => result.current.setSlice("division"));

    expect(portalRouter.navigations.at(-1)).toMatchObject({ resetScroll: false });
  });
});
