import type { DateRange } from "@/api/period-to-date-range";

export { downloadBlob } from "@/lib/download";

export function metricTimeseriesFilename(
  id: string,
  range: DateRange,
  extension: "csv" | "xlsx"
): string {
  const safeId =
    id
      .trim()
      .replace(/[^a-zA-Z0-9._-]+/g, "-")
      .replace(/^-+|-+$/g, "") || "timeseries";
  return `${safeId}_${range.from}_${range.to}.${extension}`;
}
