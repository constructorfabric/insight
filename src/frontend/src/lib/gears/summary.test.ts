import { describe, expect, it } from "vitest";

import type { Gear } from "@/api/gear-roadmap-client";
import { summariseBySubsystem } from "@/lib/gears/summary";

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

describe("summariseBySubsystem", () => {
  it("counts a gear as done only at full implementation", () => {
    const rows = summariseBySubsystem([
      gear({ number: 1, status_percent: 100 }),
      gear({ number: 2, status_percent: 90 }),
    ]);

    expect(rows[0].items).toBe(2);
    expect(rows[0].done).toBe(1);
    expect(rows[0].donePercent).toBe(50);
  });

  it("averages readiness over the gears that carry a value", () => {
    const rows = summariseBySubsystem([
      gear({ number: 1, design_percent: 100 }),
      gear({ number: 2, design_percent: 50 }),
      gear({ number: 3, design_percent: null }),
    ]);

    expect(rows[0].specReadiness).toBe(75);
  });

  it("has no readiness to report when no gear carries one", () => {
    const rows = summariseBySubsystem([gear({ design_percent: null })]);

    expect(rows[0].specReadiness).toBeNull();
  });

  it("totals effort and what is left of it", () => {
    const rows = summariseBySubsystem([
      gear({ number: 1, effort_man_days: 30, remaining_man_days: 6 }),
      gear({ number: 2, effort_man_days: 10, remaining_man_days: 10 }),
    ]);

    expect(rows[0].effortManDays).toBe(40);
    expect(rows[0].remainingManDays).toBe(16);
  });

  it("counts how many gears carry no estimate", () => {
    const rows = summariseBySubsystem([
      gear({ number: 1, effort_man_days: 30 }),
      gear({ number: 2, effort_man_days: null }),
    ]);

    expect(rows[0].unestimated).toBe(1);
  });
});
