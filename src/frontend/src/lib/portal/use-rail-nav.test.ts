import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  zones: [] as string[],
  activeZone: "overview",
  selected: [] as string[],
}));

vi.mock("@/lib/portal/use-zone-nav", async () => {
  const { ZONES } = await import("@/lib/portal/nav-model");
  return {
    useZoneNav: () => ({
      zones: ZONES.filter((zone) => mocks.zones.includes(zone.id)),
      activeZone: mocks.activeZone,
      selectZone: (zone: { id: string }) => mocks.selected.push(zone.id),
    }),
  };
});

import { RAIL } from "./rail-model";
import { useRailNav } from "./use-rail-nav";

const rail = (id: string) => RAIL.find((item) => item.id === id)!;

beforeEach(() => {
  mocks.zones = [];
  mocks.activeZone = "overview";
  mocks.selected = [];
});

describe("useRailNav", () => {
  it("offers the rail items whose zones the viewer can see", () => {
    mocks.zones = ["person", "custom", "manage"];

    const { result } = renderHook(() => useRailNav());

    expect(result.current.items.map((item) => item.id)).toEqual([
      "dashboards",
      "people",
      "manage",
    ]);
  });

  it("highlights Explore while AI & Cost is open", () => {
    mocks.activeZone = "aicost";

    const { result } = renderHook(() => useRailNav());

    expect(result.current.activeItem).toBe("explore");
  });

  it("opens Explore on Directions", () => {
    mocks.zones = ["directions", "aicost"];

    const { result } = renderHook(() => useRailNav());
    result.current.selectItem(rail("explore"));

    expect(mocks.selected).toEqual(["directions"]);
  });

  it("opens Explore on AI & Cost when Directions is hidden", () => {
    mocks.zones = ["aicost"];

    const { result } = renderHook(() => useRailNav());
    result.current.selectItem(rail("explore"));

    expect(mocks.selected).toEqual(["aicost"]);
  });

  it("opens People on the person view", () => {
    mocks.zones = ["person", "people"];

    const { result } = renderHook(() => useRailNav());
    result.current.selectItem(rail("people"));

    expect(mocks.selected).toEqual(["person"]);
  });
});
