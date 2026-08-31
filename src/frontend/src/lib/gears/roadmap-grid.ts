import type { Gear } from "@/api/gear-roadmap-client";

export const UNGROUPED = "Ungrouped";

export interface RoadmapRow {
  subsystem: string;
  overdue: Gear[];
  slots: Gear[][];
  later: Gear[];
}

export function buildRoadmap(
  gears: Gear[],
  windowMonths: number,
): RoadmapRow[] {
  const rows = new Map<string, RoadmapRow>();

  for (const gear of gears) {
    const key = gear.subsystem ?? UNGROUPED;
    const row = rows.get(key) ?? emptyRow(key, windowMonths);
    rows.set(key, row);

    if (gear.placement === "overdue") {
      row.overdue.push(gear);
    } else if (gear.placement === "slot" && typeof gear.slot === "number") {
      row.slots[gear.slot]?.push(gear);
    } else {
      row.later.push(gear);
    }
  }

  return [...rows.values()].sort(bySubsystem);
}

function emptyRow(subsystem: string, windowMonths: number): RoadmapRow {
  return {
    subsystem,
    overdue: [],
    slots: Array.from({ length: windowMonths }, () => []),
    later: [],
  };
}

function bySubsystem(left: RoadmapRow, right: RoadmapRow): number {
  if (left.subsystem === UNGROUPED) return 1;
  if (right.subsystem === UNGROUPED) return -1;
  return left.subsystem.localeCompare(right.subsystem);
}
