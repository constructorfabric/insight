import { addMonths, differenceInCalendarDays, format, startOfMonth } from "date-fns";

import type { GearLane } from "@/api/gear-roadmap-client";

const MS_PER_DAY = 86_400_000;

export interface GanttBar {
  gearNumber: number;
  start: string;
  end: string;
}

export interface GanttLane {
  assignee: string | null;
  assigneeUrl: string | null;
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
      assigneeUrl: lane.assignee_url ?? null,
      bars: lane.spans.map((span) => ({
        gearNumber: span.gear_number,
        start: span.start,
        end: span.end,
      })),
    })),
  };
}

export interface BarGeometry {
  offsetDays: number;
  lengthDays: number;
}

/** Where a bar sits on the track, in days from the chart's first day. */
export function barGeometry(bar: GanttBar, chartStart: string): BarGeometry {
  const start = dayNumber(bar.start);

  return {
    offsetDays: start - dayNumber(chartStart),
    lengthDays: dayNumber(bar.end) - start + 1,
  };
}

export interface MonthTick {
  label: string;
  offsetDays: number;
}

export function monthTicks(start: string, totalDays: number): MonthTick[] {
  if (start === "" || totalDays <= 0) return [];

  const first = utcDate(start);
  const ticks: MonthTick[] = [];

  for (let month = addMonths(startOfMonth(first), 1); ; month = addMonths(month, 1)) {
    const offsetDays = differenceInCalendarDays(month, first);
    if (offsetDays >= totalDays) return ticks;

    ticks.push({ label: format(month, "yyyy-MM"), offsetDays });
  }
}

/** Months of the window, `yyyy-MM`, starting at the window's own first month. */
export function monthLabels(windowStart: string, months: number): string[] {
  const first = utcDate(`${windowStart}-01`);

  return Array.from({ length: months }, (_, index) =>
    format(addMonths(first, index), "yyyy-MM"),
  );
}

function utcDate(date: string): Date {
  return new Date(`${date}T00:00:00Z`);
}

export function dayNumber(date: string): number {
  return Math.floor(Date.parse(`${date}T00:00:00Z`) / MS_PER_DAY);
}

function isoDate(day: number): string {
  return new Date(day * MS_PER_DAY).toISOString().slice(0, 10);
}
