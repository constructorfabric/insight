import { describe, expect, it } from "vitest";

import type { GearLane } from "@/api/gear-roadmap-client";
import {
  barGeometry,
  buildGantt,
  monthLabels,
  monthTicks,
} from "@/lib/gears/gantt";

const LANES: GearLane[] = [
  {
    assignee: "dev-one",
    spans: [
      { gear_id: "example-org/example-repo#1", start: "2030-01-01", end: "2030-01-10" },
      { gear_id: "example-org/example-repo#2", start: "2030-01-11", end: "2030-01-15" },
    ],
  },
  {
    assignee: null,
    spans: [{ gear_id: "example-org/example-repo#3", start: "2030-01-06", end: "2030-01-20" }],
  },
];

describe("buildGantt", () => {
  it("spans from the earliest start to the latest end", () => {
    const chart = buildGantt(LANES);

    expect(chart.start).toBe("2030-01-01");
    expect(chart.totalDays).toBe(20);
  });

  it("keeps only the dates, leaving the geometry to the view", () => {
    const chart = buildGantt(LANES);

    expect(chart.lanes[0].bars[0]).toEqual({
      gearId: "example-org/example-repo#1",
      start: "2030-01-01",
      end: "2030-01-10",
    });
  });

  it("keeps the lane order and its assignee", () => {
    const chart = buildGantt(LANES);

    expect(chart.lanes.map((lane) => lane.assignee)).toEqual([
      "dev-one",
      null,
    ]);
  });

  it("has nothing to draw for an empty schedule", () => {
    const chart = buildGantt([]);

    expect(chart.totalDays).toBe(0);
    expect(chart.lanes).toHaveLength(0);
  });
});

describe("monthTicks", () => {
  it("marks the first day of every month the chart covers", () => {
    const ticks = monthTicks("2030-01-20", 45);

    expect(ticks).toEqual([
      { label: "2030-02", offsetDays: 12 },
      { label: "2030-03", offsetDays: 40 },
    ]);
  });

  it("has no ticks when the chart stays inside one month", () => {
    expect(monthTicks("2030-01-02", 5)).toEqual([]);
  });
})

describe("barGeometry", () => {
  it("measures a bar in days from the chart start", () => {
    const chart = buildGantt(LANES);

    expect(barGeometry(chart.lanes[0].bars[0], chart.start)).toEqual({
      offsetDays: 0,
      lengthDays: 10,
    });
    expect(barGeometry(chart.lanes[0].bars[1], chart.start)).toEqual({
      offsetDays: 10,
      lengthDays: 5,
    });
    expect(barGeometry(chart.lanes[1].bars[0], chart.start)).toEqual({
      offsetDays: 5,
      lengthDays: 15,
    });
  });
});

describe("monthLabels", () => {
  it("runs from the window's own first month", () => {
    expect(monthLabels("2030-01", 3)).toEqual([
      "2030-01",
      "2030-02",
      "2030-03",
    ]);
  });

  it("rolls into the next year", () => {
    expect(monthLabels("2030-11", 3)).toEqual([
      "2030-11",
      "2030-12",
      "2031-01",
    ]);
  });
});
