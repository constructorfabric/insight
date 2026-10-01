import type { Widget } from "@/api/custom-client";
import { widgetColumns } from "@/lib/custom/widget-columns";

/** The column a clocked metric injects when its run is bucketed. */
const BUCKET = "bucket";

/**
 * Whether this widget reads the injected time bucket.
 *
 * A run is bucketed only for a widget that draws the bucket. Bucketing one
 * that does not hands it a row per time bucket instead of a row per category,
 * so the same category appears many times and every total is wrong.
 */
export function drawsBucket(widget: Widget): boolean {
  return widgetColumns(widget).includes(BUCKET);
}
