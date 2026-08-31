import type { GearLane } from "@/api/gear-roadmap-client";

const MS_PER_DAY = 86_400_000;

export interface GanttBar {
  gearNumber: number;
  offsetDays: number;
  lengthDays: number;
  start: string;
  end: string;
}

export interface GanttLane {
  assignee: string | null;
  bars: GanttBar[];
}

export interface GanttChart {
  start: string;
  totalDays: number;
  lanes: GanttLane[];
}

export function buildGantt(lanes: GearLane[]): GanttChart {
  const days = lanes
    .flatMap((lane) => lane.spans)
    .flatMap((span) => [dayNumber(span.start), dayNumber(span.end)]);

  if (days.length === 0) {
    return { start: "", totalDays: 0, lanes: [] };
  }

  const first = Math.min(...days);
  const last = Math.max(...days);

  return {
    start: isoDate(first),
    totalDays: last - first + 1,
    lanes: lanes.map((lane) => ({
      assignee: lane.assignee ?? null,
      bars: lane.spans.map((span) => ({
        gearNumber: span.gear_number,
        offsetDays: dayNumber(span.start) - first,
        lengthDays: dayNumber(span.end) - dayNumber(span.start) + 1,
        start: span.start,
        end: span.end,
      })),
    })),
  };
}

export interface MonthTick {
  label: string;
  offsetDays: number;
}

export function monthTicks(start: string, totalDays: number): MonthTick[] {
  if (start === "" || totalDays <= 0) return [];

  const first = dayNumber(start);
  const ticks: MonthTick[] = [];
  const cursor = new Date(first * MS_PER_DAY);
  cursor.setUTCDate(1);

  for (;;) {
    cursor.setUTCMonth(cursor.getUTCMonth() + 1);
    const offsetDays = Math.floor(cursor.getTime() / MS_PER_DAY) - first;
    if (offsetDays >= totalDays) return ticks;

    ticks.push({
      label: cursor.toISOString().slice(0, 7),
      offsetDays,
    });
  }
}

function dayNumber(date: string): number {
  return Math.floor(Date.parse(`${date}T00:00:00Z`) / MS_PER_DAY);
}

function isoDate(day: number): string {
  return new Date(day * MS_PER_DAY).toISOString().slice(0, 10);
}
