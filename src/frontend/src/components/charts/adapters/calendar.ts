import type { MetricResult } from "@/api/custom-client";
import { toNumber } from "@/components/custom/chart-format";

import { columnReader } from "./cells";

const CALENDAR_WEEKS = 53;
const DAY_MS = 86_400_000;
const WEEK_MS = 7 * DAY_MS;

export interface CalendarCell {
  date: string;
  value: number;
  level: 0 | 1 | 2 | 3 | 4;
  week: number;
  weekday: number;
}

export function calendarCells(
  result: MetricResult,
  x: string,
  value: string
): CalendarCell[] {
  const totals = dayTotals(result, x, value);
  if (totals.size === 0) return [];

  const days = [...totals.keys()];
  const last = utc(days.reduce((a, b) => (b > a ? b : a)));
  const earliest =
    last - (CALENDAR_WEEKS * 7 - 1 - new Date(last).getUTCDay()) * DAY_MS;
  const first = Math.max(utc(days.reduce((a, b) => (b < a ? b : a))), earliest);
  const firstWeekStart = first - new Date(first).getUTCDay() * DAY_MS;

  const busiest = Math.max(0, ...totals.values());
  const cells: CalendarCell[] = [];
  for (let at = first; at <= last; at += DAY_MS) {
    const date = new Date(at).toISOString().slice(0, 10);
    const amount = totals.get(date) ?? 0;

    cells.push({
      date,
      value: amount,
      level: level(amount, busiest),
      week: Math.floor((at - firstWeekStart) / WEEK_MS),
      weekday: new Date(at).getUTCDay(),
    });
  }

  return cells;
}

export function calendarGrain(
  result: MetricResult,
  x: string
): "day" | "week" | "month" {
  const readDay = columnReader(result, x);
  const days = [...new Set(result.rows.map((row) => dayOf(readDay(row))))]
    .filter((day): day is string => day !== undefined)
    .sort();
  if (days.length < 2) return "day";

  if (days.every((day) => day.endsWith("-01"))) return "month";

  const gaps = days
    .slice(1)
    .map((day, index) => utc(day) - utc(days[index] ?? ""));
  if (gaps.every((gap) => gap >= WEEK_MS && gap % WEEK_MS === 0)) {
    return "week";
  }

  return "day";
}

function dayTotals(result: MetricResult, x: string, value: string) {
  const readDay = columnReader(result, x);
  const readValue = columnReader(result, value);

  const totals = new Map<string, number>();
  for (const row of result.rows) {
    const day = dayOf(readDay(row));
    if (!day) continue;

    totals.set(day, (totals.get(day) ?? 0) + (toNumber(readValue(row)) ?? 0));
  }

  return totals;
}

function dayOf(cell: unknown): string | undefined {
  const day = /^(\d{4}-\d{2}-\d{2})/.exec(String(cell ?? ""))?.[1];

  return day && !Number.isNaN(utc(day)) ? day : undefined;
}

function utc(day: string): number {
  return Date.parse(`${day}T00:00:00Z`);
}

function level(amount: number, busiest: number): CalendarCell["level"] {
  if (amount <= 0 || busiest <= 0) return 0;

  return Math.min(
    4,
    Math.ceil((amount / busiest) * 4)
  ) as CalendarCell["level"];
}
