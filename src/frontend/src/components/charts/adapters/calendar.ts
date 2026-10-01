import type { MetricResult } from "@/api/custom-client";

import { columnReader, toNumber } from "./cells";

export const CALENDAR_WEEKS = 53;
const DAY_MS = 86_400_000;

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

  const days = [...totals.keys()].sort();
  const last = utc(days[days.length - 1] ?? "");
  const earliest =
    last - (CALENDAR_WEEKS * 7 - 1 - new Date(last).getUTCDay()) * DAY_MS;
  const first = Math.max(utc(days[0] ?? ""), earliest);
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
      week: Math.floor((at - firstWeekStart) / (7 * DAY_MS)),
      weekday: new Date(at).getUTCDay(),
    });
  }

  return cells;
}

function dayTotals(result: MetricResult, x: string, value: string) {
  const readDay = columnReader(result, x);
  const readValue = columnReader(result, value);

  const totals = new Map<string, number>();
  for (const row of result.rows) {
    const day = /^(\d{4}-\d{2}-\d{2})/.exec(String(readDay(row) ?? ""))?.[1];
    if (!day || Number.isNaN(utc(day))) continue;

    totals.set(day, (totals.get(day) ?? 0) + (toNumber(readValue(row)) ?? 0));
  }

  return totals;
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
