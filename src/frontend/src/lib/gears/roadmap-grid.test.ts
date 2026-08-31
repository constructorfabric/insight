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
    placement: "none",
    assignees: [],
    closed: false,
    ...overrides,
  };
}

describe("buildRoadmap", () => {
  it("keeps an overdue gear out of the later column", () => {
    const rows = buildRoadmap([gear({ placement: "overdue" })], WINDOW_MONTHS);

    expect(rows[0].overdue).toHaveLength(1);
    expect(rows[0].later).toHaveLength(0);
  });

  it("places a scheduled gear in its own month slot", () => {
    const rows = buildRoadmap(
      [gear({ placement: "slot", slot: 2 })],
      WINDOW_MONTHS,
    );

    expect(rows[0].slots).toHaveLength(WINDOW_MONTHS);
    expect(rows[0].slots[2]).toHaveLength(1);
    expect(rows[0].slots[0]).toHaveLength(0);
  });

  it("collects backlog, future and unreadable milestones as later work", () => {
    const rows = buildRoadmap(
      [
        gear({ number: 1, placement: "backlog" }),
        gear({ number: 2, placement: "future" }),
        gear({ number: 3, placement: "unrecognized" }),
        gear({ number: 4, placement: "none" }),
      ],
      WINDOW_MONTHS,
    );

    expect(rows[0].later.map((entry) => entry.number)).toEqual([1, 2, 3, 4]);
  });

  it("groups by subsystem and sorts gears without one last", () => {
    const rows = buildRoadmap(
      [
        gear({ subsystem: null, placement: "backlog" }),
        gear({ subsystem: "OSS", placement: "backlog" }),
        gear({ subsystem: "BSS", placement: "backlog" }),
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
