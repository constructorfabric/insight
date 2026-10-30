// @vitest-environment jsdom
/**
 * The central line defaults every chart inherits: straight segments between
 * real readings (no invented curve), dots while they are readable, and an
 * always-visible dot for a lone value that would otherwise draw no line.
 */
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import {
  ChartContainer,
  ChartLine,
  DOT_DENSITY_LIMIT,
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
  it("draws a dot per reading on a short series", () => {
    const { container } = renderSeries(series(7));
    expect(container.querySelectorAll("circle")).toHaveLength(7);
  });

  it("drops per-point dots once the series gets dense", () => {
    const { container } = renderSeries(series(DOT_DENSITY_LIMIT + 10));
    expect(container.querySelectorAll("circle")).toHaveLength(0);
  });

  it("keeps the boundary readable — exactly at the limit dots still show", () => {
    const { container } = renderSeries(series(DOT_DENSITY_LIMIT));
    expect(container.querySelectorAll("circle")).toHaveLength(DOT_DENSITY_LIMIT);
  });

  it("always marks a lone reading between gaps, however dense the series", () => {
    // One value surrounded by nulls draws no line segment at all: without a dot
    // the chart would look empty rather than "one measurement".
    const data = Array.from({ length: DOT_DENSITY_LIMIT + 20 }, (_, i) => ({
      d: `d${i}`,
      v: i === 15 ? 42 : null,
    }));
    const { container } = renderSeries(data);
    expect(container.querySelectorAll("circle")).toHaveLength(1);
  });

  it("renders no dot for a null value", () => {
    const { container } = renderSeries([
      { d: "a", v: 1 },
      { d: "b", v: null },
      { d: "c", v: 3 },
    ]);
    // a and c are dotted; b contributes nothing.
    expect(container.querySelectorAll("circle")).toHaveLength(2);
  });

  it("joins readings with straight segments, not an interpolated curve", () => {
    const { container } = renderSeries(series(4));
    const path = container.querySelector("path.recharts-line-curve");
    // A monotone curve emits cubic commands ("C"); linear segments only "L".
    expect(path?.getAttribute("d")).not.toMatch(/C/);
    expect(path?.getAttribute("d")).toMatch(/L/);
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
