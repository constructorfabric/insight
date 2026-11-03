import { describe, expect, it, vi } from "vitest";
import type { MouseHandlerDataParam } from "recharts";

import { pointClick } from "./chart-data";

const at = (activeTooltipIndex: unknown) =>
  ({ activeTooltipIndex }) as MouseHandlerDataParam;

describe("pointClick", () => {
  it.each([
    ["a numeric index", 1, 1],
    ["a string index", "1", 1],
    ["no point under the pointer", null, 0],
    ["an index Recharts left out", undefined, 0],
  ])("reports %s accordingly", (_case, index, calls) => {
    const onPointClick = vi.fn();
    pointClick(onPointClick)?.(at(index));
    expect(onPointClick, _case).toHaveBeenCalledTimes(calls);
  });

  it("gives a chart no click handler when nothing listens", () => {
    expect(pointClick(undefined)).toBeUndefined();
  });
});
