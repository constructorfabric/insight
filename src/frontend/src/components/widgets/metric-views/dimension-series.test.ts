import { describe, expect, it } from "vitest";

import {
  dimensionColorSeed,
  dimensionLabel,
  dimensionSeriesKey,
  safeSeriesKey,
} from "@/components/widgets/metric-views/dimension-series";

const TOOL = [{ key: "tool", value: "cursor", label: "Cursor" }];
const TOOL_SURFACE = [
  { key: "tool", value: "claude", label: "Claude" },
  { key: "surface", value: "chat", label: "Chat" },
];

describe("dimension-series", () => {
  it("seeds a single dimension by its bare value — every chart in a drilldown must hash the same string to the same hue", () => {
    expect(dimensionColorSeed(TOOL)).toBe("cursor");
    expect(dimensionColorSeed([])).toBe("total");
    expect(dimensionColorSeed(TOOL_SURFACE)).toBe("tool:claude|surface:chat");
  });

  it("labels from response labels with value fallback", () => {
    expect(dimensionLabel(TOOL)).toBe("Cursor");
    expect(dimensionLabel([{ key: "tool", value: "cursor" }])).toBe("cursor");
    expect(dimensionLabel([])).toBe("Total");
  });

  it("produces dataKey-safe, collision-proof series keys", () => {
    expect(dimensionSeriesKey(TOOL)).toMatch(/^tool_cursor_[a-z0-9]+$/);
    expect(dimensionSeriesKey(TOOL_SURFACE)).toMatch(
      /^tool_claude_surface_chat_[a-z0-9]+$/,
    );
    // Deterministic across calls.
    expect(dimensionSeriesKey(TOOL)).toBe(dimensionSeriesKey(TOOL));
    // Sanitization alone maps these to the same string; the hash keeps
    // distinct raw values distinct so charts never merge series silently.
    expect(safeSeriesKey("cursor.ai")).not.toBe(safeSeriesKey("cursor_ai"));
    expect(safeSeriesKey("cursor.ai")).not.toBe(safeSeriesKey("cursor-ai"));
  });

  it("does not collide distinct dimension groups that share a sanitized form", () => {
    // `:`/`|` composition keeps the raw strings distinct where a `_`/`__`
    // scheme would have produced the identical raw string for both.
    const a = dimensionSeriesKey([{ key: "tool_x", value: "y" }]);
    const b = dimensionSeriesKey([{ key: "tool", value: "x_y" }]);
    expect(a).not.toBe(b);
  });
});
