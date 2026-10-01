import { cloneElement, isValidElement, type ReactNode } from "react";
import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { MetricResult } from "@/api/custom-client";

import { CATEGORY_AXIS_WIDTH, LABEL_CHARS } from "../chart-style";
import { ChartKind, type ChartWidget } from "./index";

vi.mock("recharts", async (importOriginal) => {
  const actual = await importOriginal<typeof import("recharts")>();

  return {
    ...actual,
    ResponsiveContainer: ({ children }: { children: ReactNode }) =>
      isValidElement(children)
        ? cloneElement(children, { width: 600, height: 300 } as never)
        : null,
  };
});

const BY_DAY: MetricResult = {
  columns: ["day", "repo", "n", "goal", "files"],
  rows: [
    ["2026-09-01", "api", 40, 50, 3],
    ["2026-09-01", "web", 20, 50, 5],
    ["2026-09-02", "api", 60, 50, 4],
    ["2026-09-02", "web", 10, 50, 2],
  ],
};

const BY_REPO: MetricResult = {
  columns: ["repo", "n", "goal"],
  rows: [
    ["api", 120, 150],
    ["web", 80, 100],
    ["docs", -10, 20],
  ],
};

function drawn(widget: ChartWidget, result: MetricResult) {
  const view = render(<ChartKind widget={widget} result={result} />);
  const figure = screen.getByRole("figure", { name: `${widget.type} chart` });

  return { view, figure };
}

function titles(figure: HTMLElement): string[] {
  return [...figure.querySelectorAll("title")].map(
    (title) => title.textContent ?? ""
  );
}

function legend(figure: HTMLElement): string[] {
  const list = within(figure).queryByRole("list", { name: "Chart legend" });

  return list
    ? within(list)
        .getAllByRole("listitem")
        .map((item) => item.textContent ?? "")
    : [];
}

describe("<ChartKind>", () => {
  it.each<[ChartWidget, MetricResult, string[], string]>([
    [
      { type: "area", metric: "m", x: "day", y: "n", series: "repo" },
      BY_DAY,
      ["api", "web"],
      ".recharts-area-area",
    ],
    [
      {
        type: "line",
        metric: "m",
        x: "day",
        y: "n",
        series: "repo",
        target: "goal",
      },
      BY_DAY,
      ["api", "web", "goal"],
      ".recharts-line-curve",
    ],
    [
      { type: "bar", metric: "m", x: "day", y: "n", series: "repo" },
      BY_DAY,
      ["api", "web"],
      ".recharts-bar-rectangle",
    ],
    [
      {
        type: "stacked",
        metric: "m",
        label: "day",
        value: "n",
        series: "repo",
      },
      BY_DAY,
      ["api", "web"],
      ".recharts-bar-rectangle",
    ],
    [
      { type: "composed", metric: "m", x: "day", y: "n", y2: "files" },
      BY_DAY,
      ["n", "files"],
      ".recharts-line-curve",
    ],
    [
      { type: "scatter", metric: "m", x: "files", y: "n", series: "repo" },
      BY_DAY,
      ["api", "web"],
      ".recharts-scatter-symbol",
    ],
    [
      { type: "bubble", metric: "m", x: "files", y: "n", size: "goal" },
      BY_DAY,
      ["n"],
      ".recharts-scatter-symbol",
    ],
    [
      { type: "waterfall", metric: "m", label: "repo", value: "n" },
      BY_REPO,
      ["Increase", "Decrease", "Total"],
      ".recharts-bar-rectangle",
    ],
    [
      { type: "pie", metric: "m", label: "repo", value: "n" },
      BY_REPO,
      ["api", "web"],
      ".recharts-pie-sector",
    ],
    [
      { type: "donut", metric: "m", label: "repo", value: "n" },
      BY_REPO,
      ["api", "web"],
      ".recharts-pie-sector",
    ],
    [
      { type: "radar", metric: "m", label: "repo", value: "n", target: "goal" },
      BY_REPO,
      ["n", "goal"],
      ".recharts-radar-polygon",
    ],
  ])("draws a %o with its legend", (widget, result, items, mark) => {
    const { figure } = drawn(widget, result);

    expect(legend(figure)).toEqual(items);
    expect(figure.querySelector(mark)).not.toBeNull();
  });

  it("ranks the largest rows first and writes each value beside its bar", () => {
    const { figure } = drawn(
      { type: "ranked", metric: "m", label: "repo", value: "n" },
      BY_REPO
    );

    expect(figure.querySelectorAll(".recharts-bar-rectangle")).toHaveLength(3);
    const labels = figure.querySelector(".recharts-label-list");
    expect(labels).not.toBeNull();
    expect(within(labels as HTMLElement).getByText("120")).toBeInTheDocument();
  });

  it("puts the total of a donut in its centre", () => {
    const { figure } = drawn(
      { type: "donut", metric: "m", label: "repo", value: "n" },
      BY_REPO
    );

    expect(within(figure).getByText("200")).toBeInTheDocument();
  });

  it("draws a radial ring as the share of its maximum", () => {
    const { figure } = drawn(
      { type: "radial", metric: "m", value: "n", max: "goal" },
      BY_REPO
    );

    expect(within(figure).getByText("80%")).toBeInTheDocument();
  });

  it("lists every funnel stage with its count, largest first", () => {
    const { figure } = drawn(
      { type: "funnel", metric: "m", label: "repo", value: "n" },
      BY_REPO
    );

    expect(
      within(figure)
        .getAllByRole("listitem")
        .map((stage) => stage.textContent)
    ).toEqual(["api120", "web80"]);
  });

  it("names each treemap tile in full even when its label is cut", () => {
    const { figure } = drawn(
      { type: "treemap", metric: "m", label: "repo", value: "n" },
      {
        columns: ["repo", "n"],
        rows: [
          ["a-very-long-repository-name-indeed", 90],
          ["web", 10],
        ],
      }
    );

    expect(
      titles(figure).some((title) =>
        title.startsWith("a-very-long-repository-name-indeed")
      )
    ).toBe(true);
  });

  it("draws one calendar cell per day", () => {
    const { figure } = drawn(
      { type: "heatmap", metric: "m", x: "day", value: "n" },
      BY_DAY
    );

    expect(figure.querySelectorAll("[data-day]")).toHaveLength(2);
    expect(figure.querySelector('[data-day="2026-09-02"]')).toHaveAttribute(
      "title",
      "2026-09-02: 70"
    );
  });

  it("leads a pulse with its latest reading and its change", () => {
    const { figure } = drawn(
      { type: "pulse", metric: "m", x: "day", y: "n" },
      {
        columns: ["day", "n"],
        rows: [
          ["2026-09-01", 100],
          ["2026-09-02", 125],
        ],
      }
    );

    expect(within(figure).getByText("125")).toBeInTheDocument();
    expect(within(figure).getByText("25%")).toBeInTheDocument();
  });

  it("shows no change for a pulse with a single reading", () => {
    const { figure } = drawn(
      { type: "pulse", metric: "m", x: "day", y: "n" },
      { columns: ["day", "n"], rows: [["2026-09-01", 7]] }
    );

    expect(within(figure).getByText("7")).toBeInTheDocument();
    expect(within(figure).queryByText(/%$/)).not.toBeInTheDocument();
  });

  it("cuts a long ranked label and keeps the full name in its tooltip title", () => {
    const { figure } = drawn(
      { type: "ranked", metric: "m", label: "repo", value: "n" },
      {
        columns: ["repo", "n"],
        rows: [["an-extremely-long-repository-name", 5]],
      }
    );

    expect(within(figure).getByText("an-extremely…")).toBeInTheDocument();
    expect(titles(figure)).toContain("an-extremely-long-repository-name");
  });
  it("gives a category axis room for a cut label and its ellipsis", () => {
    expect(CATEGORY_AXIS_WIDTH).toBeGreaterThanOrEqual((LABEL_CHARS + 1) * 8);
  });

  it("stacks bars once there are more than two series", () => {
    const { figure } = drawn(
      { type: "bar", metric: "m", x: "day", y: "n", series: "who" },
      {
        columns: ["day", "who", "n"],
        rows: [
          ["2026-09-01", "a", 1],
          ["2026-09-01", "b", 2],
          ["2026-09-01", "c", 3],
          ["2026-09-01", "d", 4],
        ],
      }
    );
    const lefts = [
      ...figure.querySelectorAll(".recharts-bar-rectangle path"),
    ].map((path) =>
      Number(/^M\s*([\d.]+)/.exec(path.getAttribute("d") ?? "")?.[1])
    );

    expect(lefts).toHaveLength(4);
    expect(new Set(lefts).size).toBe(1);
  });

  it("keeps the twelve largest treemap tiles and folds the rest into Other", () => {
    const rows = Array.from({ length: 20 }, (_, index) => [
      `env-${index}`,
      100 - index,
    ]);
    const { figure } = drawn(
      { type: "treemap", metric: "m", label: "env", value: "n" },
      { columns: ["env", "n"], rows }
    );
    const tiles = titles(figure).filter((title) => title.includes(":"));

    expect(tiles).toHaveLength(13);
    expect(tiles.some((title) => title.startsWith("Other:"))).toBe(true);
  });
});
