/**
 * Unit coverage for the period store (`src/hooks/use-period.ts`):
 * module-init validation of persisted localStorage values, the usePeriod
 * hook, persistence on set, and the best-effort catch when localStorage
 * writes throw.
 *
 * The store keeps module-level state, so each test re-imports a fresh
 * copy via `vi.resetModules()` + dynamic import.
 */

import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const PERIOD_KEY = "insight.period";
const CUSTOM_KEY = "insight.period.custom";
const ISO = /^\d{4}-\d{2}-\d{2}$/;

async function freshModule() {
  vi.resetModules();
  return import("@/hooks/use-period");
}

/** Module-init state as the only consumer sees it — through the hook. */
async function freshState() {
  const mod = await freshModule();
  const { result } = renderHook(() => mod.usePeriod());
  return result.current;
}

describe("use-period store init", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.restoreAllMocks();
  });

  it("defaults to month / no custom range with empty storage", async () => {
    const state = await freshState();
    expect(state.period).toBe("month");
    expect(state.customRange).toBeNull();
  });

  it("restores valid persisted period and custom range", async () => {
    window.localStorage.setItem(PERIOD_KEY, "quarter");
    window.localStorage.setItem(
      CUSTOM_KEY,
      JSON.stringify({ from: "2026-01-01", to: "2026-01-31" }),
    );
    const state = await freshState();
    expect(state.period).toBe("quarter");
    expect(state.customRange).toEqual({
      from: "2026-01-01",
      to: "2026-01-31",
    });
  });

  it("ignores an invalid persisted period", async () => {
    window.localStorage.setItem(PERIOD_KEY, "decade");
    expect((await freshState()).period).toBe("month");
  });

  it("rejects a custom range that is not valid JSON", async () => {
    window.localStorage.setItem(CUSTOM_KEY, "{nope");
    expect((await freshState()).customRange).toBeNull();
  });

  it("rejects a custom range with malformed dates", async () => {
    window.localStorage.setItem(
      CUSTOM_KEY,
      JSON.stringify({ from: "01/02/2026", to: "2026-02-01" }),
    );
    expect((await freshState()).customRange).toBeNull();
  });

  it("rejects a custom range where from is after to", async () => {
    window.localStorage.setItem(
      CUSTOM_KEY,
      JSON.stringify({ from: "2026-03-01", to: "2026-02-01" }),
    );
    expect((await freshState()).customRange).toBeNull();
  });
});

describe("usePeriod", () => {
  beforeEach(() => {
    window.localStorage.clear();
    vi.restoreAllMocks();
  });

  it("exposes the current period and a resolved date range", async () => {
    const mod = await freshModule();
    const { result } = renderHook(() => mod.usePeriod());
    expect(result.current.period).toBe("month");
    expect(result.current.customRange).toBeNull();
    expect(result.current.dateRange.from).toMatch(ISO);
    expect(result.current.dateRange.to).toMatch(ISO);
    expect(result.current.dateRange.from <= result.current.dateRange.to).toBe(
      true,
    );
  });

  it("setPeriod updates state, clears the custom range, and persists", async () => {
    window.localStorage.setItem(
      CUSTOM_KEY,
      JSON.stringify({ from: "2026-01-01", to: "2026-01-31" }),
    );
    const mod = await freshModule();
    const { result } = renderHook(() => mod.usePeriod());
    expect(result.current.customRange).not.toBeNull();

    act(() => {
      result.current.setPeriod("week");
    });
    expect(result.current.period).toBe("week");
    expect(result.current.customRange).toBeNull();
    expect(window.localStorage.getItem(PERIOD_KEY)).toBe("week");
    // Clearing the range removes the persisted custom key.
    expect(window.localStorage.getItem(CUSTOM_KEY)).toBeNull();
  });

  it("setCustomRange stores the range and drives dateRange from it", async () => {
    const mod = await freshModule();
    const { result } = renderHook(() => mod.usePeriod());
    const range = { from: "2026-05-01", to: "2026-05-10" };

    act(() => {
      result.current.setCustomRange(range);
    });
    expect(result.current.customRange).toEqual(range);
    expect(result.current.dateRange).toEqual(range);
    expect(JSON.parse(window.localStorage.getItem(CUSTOM_KEY) ?? "")).toEqual(
      range,
    );

    act(() => {
      result.current.setCustomRange(null);
    });
    expect(result.current.customRange).toBeNull();
    expect(window.localStorage.getItem(CUSTOM_KEY)).toBeNull();
  });

  it("keeps the in-memory state when localStorage.setItem throws", async () => {
    const mod = await freshModule();
    const { result } = renderHook(() => mod.usePeriod());
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota exceeded");
    });
    act(() => {
      result.current.setPeriod("year");
    });
    expect(result.current.period).toBe("year");
  });
});

