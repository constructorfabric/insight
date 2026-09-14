import type { Widget } from "@/api/custom-client";

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
  switch (widget.type) {
    case "table":
      return widget.columns.includes(BUCKET);
    case "line":
    case "bar":
    case "area":
      return widget.x === BUCKET || widget.y === BUCKET;
    case "stat":
      return widget.value === BUCKET;
    case "pie":
      return widget.label === BUCKET || widget.value === BUCKET;
  }
}
