export type TimeseriesPresentation = "table" | "chart";

export function parseTimeseriesPresentation(
  value: string
): TimeseriesPresentation | undefined {
  return value === "table" || value === "chart" ? value : undefined;
}

export function serializeTimeseriesPresentation(
  value: TimeseriesPresentation
): string {
  return value;
}
