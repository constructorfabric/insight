// @vitest-environment jsdom
/**
 * What is left in the store after navigation moved to the URL and the portal
 * stopped being optional: one PREFERENCE. It belongs here because it describes
 * the reader rather than the view — a shared link must not reveal the
 * scaffolding they never asked to see.
 */
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { setPortalShowPlanned, usePortalShowPlanned } from "./portal-store";

beforeEach(() => {
  act(() => {
    setPortalShowPlanned(true);
  });
  window.localStorage.clear();
});

describe("portal preferences", () => {
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

describe("the retired opt-out", () => {
  it("is deleted from storage rather than read", async () => {
    window.localStorage.setItem("insight.portal", "false");
    vi.resetModules();
    const store = await import("./portal-store");

    // Gone from storage, and gone from the module: the portal is the
    // interface, so nothing can hold a reader off it.
    expect(window.localStorage.getItem("insight.portal")).toBeNull();
    for (const gone of ["readPortalEnabled", "usePortalEnabled", "setPortalEnabled"]) {
      expect(store, gone).not.toHaveProperty(gone);
    }
  });

  it("leaves the test-only legacy-shell hatch as the only way back", async () => {
    window.localStorage.setItem("insight.legacyShell", "true");
    vi.resetModules();
    const store = await import("./portal-store");

    // Read once, at load: the stand's UI journeys set it before any app code
    // runs, and nothing in the product writes it at all.
    expect(store.readLegacyShell()).toBe(true);
    expect(window.localStorage.getItem("insight.legacyShell")).toBe("true");
  });

  it("stays off for a reader who was never told to use it", async () => {
    vi.resetModules();
    const store = await import("./portal-store");

    expect(store.readLegacyShell()).toBe(false);
  });

  it("survives a throwing localStorage", async () => {
    // A sandboxed iframe or blocked third-party storage raises on property
    // access, not by returning null. This runs at module scope, so an
    // unguarded throw would take the whole bundle down over a stale key.
    const getItem = vi
      .spyOn(window.localStorage, "getItem")
      .mockImplementation(() => {
        throw new DOMException("blocked", "SecurityError");
      });
    const removeItem = vi
      .spyOn(window.localStorage, "removeItem")
      .mockImplementation(() => {
        throw new DOMException("blocked", "SecurityError");
      });
    try {
      vi.resetModules();
      const store = await import("./portal-store");
      expect(store).toHaveProperty("usePortalShowPlanned");
    } finally {
      getItem.mockRestore();
      removeItem.mockRestore();
    }
  });
});
