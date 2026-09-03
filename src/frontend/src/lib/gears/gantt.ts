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

  const first = dayNumber(start);
  const ticks: MonthTick[] = [];

  for (let index = 1; ; index += 1) {
    const month = monthIndex(start) + index;
    const offsetDays = firstOfMonthDay(month) - first;
    if (offsetDays >= totalDays) return ticks;

    ticks.push({ label: monthKey(month), offsetDays });
  }
}

/** Months of the window, `yyyy-MM`, starting at the window's own first month. */
export function monthLabels(windowStart: string, months: number): string[] {
  const first = monthIndex(windowStart);

  return Array.from({ length: months }, (_, index) => monthKey(first + index));
}

/**
 * Months since year zero. These dates are calendar days with no time and no
 * zone, so every step of the arithmetic stays in UTC: a `Date` built from a UTC
 * midnight but read through the host's local calendar is the previous day
 * anywhere west of UTC, which walks the labels back a month.
 */
function monthIndex(date: string): number {
  const [year, month] = date.split("-").map(Number);
  return year * 12 + (month - 1);
}

function monthKey(month: number): string {
  const year = Math.floor(month / 12);
  return `${String(year).padStart(4, "0")}-${String((month % 12) + 1).padStart(2, "0")}`;
}

function firstOfMonthDay(month: number): number {
  return Math.floor(Date.UTC(Math.floor(month / 12), month % 12, 1) / MS_PER_DAY);
}

export function dayNumber(date: string): number {
  return Math.floor(Date.parse(`${date}T00:00:00Z`) / MS_PER_DAY);
}

function isoDate(day: number): string {
  return new Date(day * MS_PER_DAY).toISOString().slice(0, 10);
}
