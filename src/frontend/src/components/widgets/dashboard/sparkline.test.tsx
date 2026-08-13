// @vitest-environment jsdom
/**
 * The line beside a headline number.
 *
 * It says only which way and how steadily, so what these tests pin is what it
 * must never say: that a period with no reading was a period with none of the
 * thing, or that a flat series is a shape.
 */
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Sparkline } from "./sparkline";

function paths(container: HTMLElement) {
  return [...container.querySelectorAll("path")];
}

describe("Sparkline", () => {
  it("draws nothing that cannot be scaled", () => {
    // One reading has no range and no direction; the caller decides what is
    // worth drawing, and this only keeps the maths safe.
    expect(render(<Sparkline points={[5]} />).container).toBeEmptyDOMElement();
    expect(
      render(<Sparkline points={[null, null]} />).container
    ).toBeEmptyDOMElement();
  });

  it("breaks the line at a gap instead of dropping it to the floor", () => {
    // One continuous path across the gap would draw a dive to zero and back —
    // a month with no data read as a month with none of the thing.
    const { container } = render(<Sparkline points={[10, null, 12, 11]} />);
    expect(paths(container)).toHaveLength(1);

    const split = render(<Sparkline points={[10, 9, null, 12, 11]} />);
    expect(paths(split.container)).toHaveLength(2);
  });

  it("survives a series that never moves", () => {
    // A flat run has zero range; scaling to it must not divide by it.
    const { container } = render(<Sparkline points={[7, 7, 7]} />);
    const d = paths(container)[0]?.getAttribute("d") ?? "";
    expect(d).toMatch(/^M/);
    expect(d).not.toMatch(/NaN|Infinity/);
  });

  it("marks the end of the line, and marks the last reading when it ends in a gap", () => {
    const { container } = render(<Sparkline points={[1, 2, 3]} />);
    expect(container.querySelectorAll("circle")).toHaveLength(1);

    const trailing = render(<Sparkline points={[1, 2, 3, null]} />);
    const dot = trailing.container.querySelector("circle");
    // The dot says "this is now"; parked at the gap it would say the series
    // ended lower than its last reading.
    expect(dot).not.toBeNull();
    expect(Number(dot?.getAttribute("cx"))).toBeLessThan(72);
  });

  it("keeps its baseline out of the reading of values", () => {
    // The rule under the line spans the window so the line does not float. It
    // is drawn at a fixed depth, NOT at zero — the line is scaled to its own
    // range, and a zero baseline would be a lie for a percentage.
    const { container } = render(<Sparkline points={[80, 84, 82]} />);
    const rule = container.querySelector("line");
    expect(rule?.getAttribute("x1")).toBe("0");
    expect(rule?.getAttribute("x2")).toBe("72");
  });
});
