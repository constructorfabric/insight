import type { Gear } from "@/api/gear-roadmap-client";
import { UNGROUPED } from "@/lib/gears/roadmap-grid";

export interface SubsystemSummary {
  subsystem: string;
  items: number;
  done: number;
  donePercent: number;
  /** Null where no gear in the group carries a value for that ladder. */
  specReadiness: number | null;
  sdkReadiness: number | null;
  implReadiness: number | null;
  effortManDays: number;
  remainingManDays: number;
  unestimated: number;
}

export function summariseBySubsystem(gears: Gear[]): SubsystemSummary[] {
  const groups = new Map<string, Gear[]>();

  for (const gear of gears) {
    const key = gear.subsystem ?? UNGROUPED;
    const members = groups.get(key);

    if (members) members.push(gear);
    else groups.set(key, [gear]);
  }

  return [...groups.entries()]
    .map(([subsystem, members]) => summarise(subsystem, members))
    .sort((left, right) => left.subsystem.localeCompare(right.subsystem));
}

function summarise(subsystem: string, gears: Gear[]): SubsystemSummary {
  const done = gears.filter((gear) => gear.status_percent === 100).length;

  return {
    subsystem,
    items: gears.length,
    done,
    donePercent: gears.length === 0 ? 0 : (done / gears.length) * 100,
    specReadiness: average(gears.map((gear) => gear.design_percent)),
    sdkReadiness: average(gears.map((gear) => gear.sdk_percent)),
    implReadiness: average(gears.map((gear) => gear.status_percent)),
    effortManDays: total(gears.map((gear) => gear.effort_man_days)),
    remainingManDays: total(gears.map((gear) => gear.remaining_man_days)),
    unestimated: gears.filter((gear) => typeof gear.effort_man_days !== "number")
      .length,
  };
}

function average(values: (number | null | undefined)[]): number | null {
  const known = values.filter((value): value is number =>
    typeof value === "number",
  );
  if (known.length === 0) return null;
  return known.reduce((sum, value) => sum + value, 0) / known.length;
}

function total(values: (number | null | undefined)[]): number {
  return values.reduce(
    (sum: number, value) => sum + (typeof value === "number" ? value : 0),
    0,
  );
}
