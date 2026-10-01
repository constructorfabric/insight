import type { SeriesKey } from "./adapters/series";

const CHART_TOKENS = 12;

export const OTHER_COLOR =
  "color-mix(in srgb, var(--muted-foreground) 55%, var(--card))";

export function seriesColor(index: number): string {
  return `var(--chart-${(index % CHART_TOKENS) + 1})`;
}

export function keyColor(key: SeriesKey, index: number): string {
  return key.key === "other" ? OTHER_COLOR : seriesColor(index);
}

export function colorKeys<K extends SeriesKey>(
  keys: K[]
): (K & { color: string })[] {
  return keys.map((key, index) => ({ ...key, color: keyColor(key, index) }));
}
