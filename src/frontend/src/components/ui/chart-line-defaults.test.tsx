// @vitest-environment jsdom
/**
 * The central line defaults every chart inherits: a smooth curve through the
 * readings, no per-point markers, and a dot only for a lone value that would
 * otherwise draw no line.
 */
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import {
  ChartContainer,
  ChartLine,
  LineChart,
  YAxis,
  type ChartConfig,
} from "./chart";

const CONFIG: ChartConfig = { v: { label: "V", color: "#f00" } };

function renderSeries(data: Array<{ d: string; v: number | null }>) {
  return render(
    <ChartContainer config={CONFIG} className="h-40 w-96">
      <LineChart data={data} width={384} height={160}>
        <ChartLine dataKey="v" stroke="#f00" />
      </LineChart>
    </ChartContainer>,
  );
}

const series = (n: number) =>
  Array.from({ length: n }, (_, i) => ({ d: `2026-07-${i + 1}`, v: i + 1 }));

describe("ChartLine defaults", () => {
  it.each([3, 7, 90])("marks no reading of a connected %i-point series", (n) => {
    const { container } = renderSeries(series(n));
    expect(container.querySelectorAll("circle"), `points: ${n}`).toHaveLength(0);
  });

  it("marks a lone reading between gaps, which draws no line", () => {
    const data = Array.from({ length: 50 }, (_, i) => ({
      d: `d${i}`,
      v: i === 15 ? 42 : null,
    }));
    const { container } = renderSeries(data);
    expect(container.querySelectorAll("circle")).toHaveLength(1);
  });

  it("marks neither side of a single gap, since both still join a neighbour", () => {
    const { container } = renderSeries([
      { d: "a", v: 1 },
      { d: "b", v: 2 },
      { d: "c", v: null },
      { d: "d", v: 3 },
      { d: "e", v: 4 },
    ]);
    expect(container.querySelectorAll("circle")).toHaveLength(0);
  });

  it("joins readings with a smooth curve", () => {
    const { container } = renderSeries(series(4));
    const path = container.querySelector("path.recharts-line-curve");
    // A monotone curve emits cubic commands ("C").
    expect(path?.getAttribute("d")).toMatch(/C/);
  });

  it("lets a caller override the curve for step-like counters", () => {
    const { container } = render(
      <ChartContainer config={CONFIG} className="h-40 w-96">
        <LineChart data={series(4)} width={384} height={160}>
          <ChartLine dataKey="v" stroke="#f00" type="stepAfter" />
        </LineChart>
      </ChartContainer>,
    );
    const d = container.querySelector("path.recharts-line-curve")?.getAttribute("d");
    // stepAfter holds a value then jumps: horizontal then vertical segments.
    expect(d).toMatch(/L/);
    expect(d).not.toMatch(/C/);
  });
});

describe("YAxis defaults", () => {
  const renderAxis = (domainMax: number) =>
    render(
      <ChartContainer config={CONFIG} className="h-40 w-96">
        <LineChart
          data={[
            { d: "a", v: 0 },
            { d: "b", v: domainMax },
          ]}
          width={384}
          height={160}
        >
          <YAxis />
          <ChartLine dataKey="v" stroke="#f00" />
        </LineChart>
      </ChartContainer>,
    );

  const ticks = (c: HTMLElement) =>
    [...c.querySelectorAll(".recharts-cartesian-axis-tick-value")].map(
      (t) => t.textContent ?? "",
    );

  it("abbreviates large ticks so they fit the axis gutter", () => {
    const { container } = renderAxis(40_000);
    // Whatever ticks recharts picks, none of them is a raw five-digit number.
    expect(ticks(container).some((t) => t.endsWith("k"))).toBe(true);
    expect(ticks(container).every((t) => !/^\d{5}/.test(t))).toBe(true);
  });

  it("leaves small ticks literal", () => {
    const { container } = renderAxis(300);
    expect(ticks(container).every((t) => !t.includes("k"))).toBe(true);
  });

  it("lets a caller keep raw numbers", () => {
    const { container } = render(
      <ChartContainer config={CONFIG} className="h-40 w-96">
        <LineChart data={[{ d: "a", v: 0 }, { d: "b", v: 40_000 }]} width={384} height={160}>
          <YAxis tickFormatter={(v: number) => String(v)} />
          <ChartLine dataKey="v" stroke="#f00" />
        </LineChart>
      </ChartContainer>,
    );
    expect(ticks(container).some((t) => /^\d{5}$/.test(t))).toBe(true);
  });
});
