import { describe, expect, it } from "vitest";

import type { Gear } from "@/api/gear-roadmap-client";
import { UNGROUPED, buildRoadmap } from "@/lib/gears/roadmap-grid";

const WINDOW_MONTHS = 9;

function gear(overrides: Partial<Gear>): Gear {
  return {
    number: 1,
    title: "CORE - Example Module",
    subsystem: "CORE",
    commitment: "committed",
    placement: { kind: "none" },
    assignees: [],
    closed: false,
    ...overrides,
  };
}

describe("buildRoadmap", () => {
  it("keeps an overdue gear out of the later column", () => {
    const rows = buildRoadmap([gear({ placement: { kind: "overdue", days: 12 } })], WINDOW_MONTHS);

    expect(rows[0].overdue).toHaveLength(1);
    expect(rows[0].later).toHaveLength(0);
  });

  it("places a scheduled gear in its own month slot", () => {
    const rows = buildRoadmap(
      [gear({ placement: { kind: "slot", slot: 2 } })],
      WINDOW_MONTHS,
    );

    expect(rows[0].slots).toHaveLength(WINDOW_MONTHS);
    expect(rows[0].slots[2]).toHaveLength(1);
    expect(rows[0].slots[0]).toHaveLength(0);
  });

  it("collects backlog, future and unreadable milestones as later work", () => {
    const rows = buildRoadmap(
      [
        gear({ number: 1, placement: { kind: "backlog" } }),
        gear({ number: 2, placement: { kind: "future" } }),
        gear({ number: 3, placement: { kind: "unrecognized" } }),
        gear({ number: 4, placement: { kind: "none" } }),
      ],
      WINDOW_MONTHS,
    );

    expect(rows[0].later.map((entry) => entry.number)).toEqual([1, 2, 3, 4]);
  });

  it("groups by subsystem and sorts gears without one last", () => {
    const rows = buildRoadmap(
      [
        gear({ subsystem: null, placement: { kind: "backlog" } }),
        gear({ subsystem: "OSS", placement: { kind: "backlog" } }),
        gear({ subsystem: "BSS", placement: { kind: "backlog" } }),
      ],
      WINDOW_MONTHS,
    );

    expect(rows.map((row) => row.subsystem)).toEqual([
      "BSS",
      "OSS",
      UNGROUPED,
    ]);
  });
});

describe("delivered gears", () => {
  it("keeps finished work out of the overdue column", () => {
    const rows = buildRoadmap(
      [gear({ placement: { kind: "delivered" } })],
      WINDOW_MONTHS,
    );

    expect(rows[0].overdue).toHaveLength(0);
    expect(rows[0].delivered).toHaveLength(1);
    expect(rows[0].later).toHaveLength(0);
  });
});
