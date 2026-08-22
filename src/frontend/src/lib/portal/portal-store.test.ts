// @vitest-environment jsdom
/**
 * What is left in the store after navigation moved to the URL: the two
 * PREFERENCES. They belong here precisely because they describe the reader
 * rather than the view — a shared link must not turn someone else's portal on,
 * or reveal the scaffolding they never asked to see.
 */
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  setPortalEnabled,
  setPortalShowPlanned,
  usePortalEnabled,
  usePortalShowPlanned,
} from "./portal-store";

beforeEach(() => {
  act(() => {
    setPortalEnabled(false);
    setPortalShowPlanned(true);
  });
  window.localStorage.clear();
});

describe("portal preferences", () => {
  it("enabled round-trips and persists", () => {
    const { result } = renderHook(() => usePortalEnabled());
    expect(result.current).toBe(false);
    act(() => setPortalEnabled(true));
    expect(result.current).toBe(true);
    expect(window.localStorage.getItem("insight.portal")).toBe("true");
  });

  it("enabled defaults ON when the key is absent", async () => {
    const { readPortalEnabled } = await import("./portal-store");
    expect(readPortalEnabled()).toBe(true);
  });

  it("an explicit opt-out is what the router reads back", async () => {
    window.localStorage.setItem("insight.portal", "false");
    const { readPortalEnabled } = await import("./portal-store");
    expect(readPortalEnabled()).toBe(false);
  });

  it("show-planned defaults OFF when the key is absent", async () => {
    window.localStorage.removeItem("insight.portal.showPlanned");
    vi.resetModules();
    const fresh = await import("./portal-store");
    const { result } = renderHook(() => fresh.usePortalShowPlanned());
    expect(result.current).toBe(false);
  });

  it("an explicit opt-in is what a fresh load reads back", async () => {
    window.localStorage.setItem("insight.portal.showPlanned", "true");
    vi.resetModules();
    const fresh = await import("./portal-store");
    const { result } = renderHook(() => fresh.usePortalShowPlanned());
    expect(result.current).toBe(true);
  });

  it("show-planned round-trips and persists", () => {
    const { result } = renderHook(() => usePortalShowPlanned());
    act(() => setPortalShowPlanned(false));
    expect(result.current).toBe(false);
    expect(window.localStorage.getItem("insight.portal.showPlanned")).toBe("false");
  });

  it("keeps no navigation state — that lives in the URL", async () => {
    const store = await import("./portal-store");
    for (const gone of [
      "setPortalZone",
      "setPortalItem",
      "setPortalDir",
      "setPortalLens",
      "setPortalSlice",
      "setPortalScope",
    ]) {
      expect(store, gone).not.toHaveProperty(gone);
    }
  });
});

describe("blocked storage", () => {
  it("reading a preference survives a throwing localStorage", async () => {
    // A sandboxed iframe or blocked third-party storage raises on property
    // access, not by returning null. The readers run at module scope, so an
    // unguarded throw would take the whole bundle down over a preview flag.
    const getItem = vi
      .spyOn(window.localStorage, "getItem")
      .mockImplementation(() => {
        throw new DOMException("blocked", "SecurityError");
      });
    try {
      const { readPortalEnabled } = await import("./portal-store");
      expect(readPortalEnabled()).toBe(true);
    } finally {
      getItem.mockRestore();
    }
  });
});
