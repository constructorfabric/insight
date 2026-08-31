import { describe, expect, it } from "vitest";

import type { GearLane } from "@/api/gear-roadmap-client";
import { buildGantt, monthTicks } from "@/lib/gears/gantt";

const LANES: GearLane[] = [
  {
    assignee: "dev-one",
    spans: [
      { gear_number: 1, start: "2030-01-01", end: "2030-01-10" },
      { gear_number: 2, start: "2030-01-11", end: "2030-01-15" },
    ],
  },
  {
    assignee: null,
    spans: [{ gear_number: 3, start: "2030-01-06", end: "2030-01-20" }],
  },
];

describe("buildGantt", () => {
  it("spans from the earliest start to the latest end", () => {
    const chart = buildGantt(LANES);

    expect(chart.start).toBe("2030-01-01");
    expect(chart.totalDays).toBe(20);
  });

  it("measures each bar in days from the chart start", () => {
    const chart = buildGantt(LANES);

    expect(chart.lanes[0].bars[0]).toMatchObject({
      offsetDays: 0,
      lengthDays: 10,
    });
    expect(chart.lanes[0].bars[1]).toMatchObject({
      offsetDays: 10,
      lengthDays: 5,
    });
    expect(chart.lanes[1].bars[0]).toMatchObject({
      offsetDays: 5,
      lengthDays: 15,
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
