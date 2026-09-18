import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

// Deterministic, DOM-inspectable stand-ins for the recharts wrappers (which
// render nothing under jsdom's zero-size ResponsiveContainer). Partial, not
// whole: the kit's own chart container imports pieces of recharts these tests
// never look at, and a bare object mock hid them.
vi.mock("recharts", async (importOriginal) => {
  const actual = await importOriginal<typeof import("recharts")>();

  const chart =
    (testId: string) =>
    ({
      data,
      children,
    }: {
      data?: Record<string, unknown>[];
      children: React.ReactNode;
    }) => (
      <div data-testid={testId} data-chart-data={JSON.stringify(data ?? null)}>
        {children}
      </div>
    );
  const series =
    (testId: string) =>
    ({ dataKey }: { dataKey: string }) => (
      <div data-testid={testId} data-key={dataKey} />
    );

  return {
    ...actual,
    ResponsiveContainer: ({ children }: { children: React.ReactNode }) => (
      <>{children}</>
    ),
    LineChart: chart("line-chart"),
    BarChart: chart("bar-chart"),
    AreaChart: chart("area-chart"),
    PieChart: chart("pie-chart"),
    CartesianGrid: () => null,
    Tooltip: () => null,
    Legend: () => null,
    XAxis: ({ dataKey }: { dataKey: string }) => (
      <div data-testid="x-axis" data-key={dataKey} />
    ),
    YAxis: () => null,
    Line: series("line"),
    Bar: series("bar"),
    Area: series("area"),
    Pie: ({
      dataKey,
      nameKey,
      children,
    }: {
      dataKey: string;
      nameKey: string;
      children: React.ReactNode;
    }) => (
      <div data-testid="pie" data-key={dataKey} data-name-key={nameKey}>
        {children}
      </div>
    ),
    Cell: () => null,
  };
});

import { CustomWidget } from "./custom-widget";

const result = {
  columns: ["day", "lines"],
  rows: [
    ["2026-09-01", 59],
    ["2026-09-02", 12],
  ],
};

describe("<CustomWidget>", () => {
  it("renders a table widget as rows", () => {
    render(
      <CustomWidget
        widget={{ type: "table", metric: "m", columns: ["day", "lines"] }}
        result={result}
      />,
    );

    expect(screen.getByRole("columnheader", { name: "day" })).toBeInTheDocument();
    expect(screen.getAllByRole("row")).toHaveLength(3);
  });

  it("renders a line widget as a chart", () => {
    render(
      <CustomWidget widget={{ type: "line", metric: "m", x: "day", y: "lines" }} result={result} />,
    );

    expect(screen.getByTestId("custom-line-chart")).toBeInTheDocument();
  });

  it("maps the widget's x and y fields to the chart axes and row values", () => {
    render(
      <CustomWidget widget={{ type: "line", metric: "m", x: "day", y: "lines" }} result={result} />,
    );

    expect(screen.getByTestId("x-axis")).toHaveAttribute("data-key", "day");
    expect(screen.getByTestId("line")).toHaveAttribute("data-key", "lines");

    const chartData = JSON.parse(
      screen.getByTestId("line-chart").getAttribute("data-chart-data")!,
    );
    expect(chartData).toEqual([
      { day: "2026-09-01", lines: 59 },
      { day: "2026-09-02", lines: 12 },
    ]);
  });

  it("shows an error in place of the content when the run failed", () => {
    render(
      <CustomWidget
        widget={{ type: "table", metric: "m", columns: ["day"] }}
        error={new Error("unknown table `evnts`")}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent("unknown table `evnts`");
  });

  it("says there is no data when the metric returned no rows", () => {
    render(
      <CustomWidget
        widget={{ type: "table", metric: "m", columns: ["day"] }}
        result={{ columns: ["day"], rows: [] }}
      />,
    );

    expect(screen.getByText(/no data/i)).toBeInTheDocument();
  });

  it("says so when the type is unknown", () => {
    render(<CustomWidget widget={{ type: "sankey", metric: "m" } as never} result={result} />);

    expect(screen.getByText(/unknown widget type/i)).toBeInTheDocument();
  });

  it("says there is no data when neither a result nor an error was passed", () => {
    render(<CustomWidget widget={{ type: "table", metric: "m", columns: ["day"] }} />);

    expect(screen.getByText(/no data/i)).toBeInTheDocument();
  });

  it("pads a short row with empty cells instead of misaligning columns", () => {
    render(
      <CustomWidget
        widget={{ type: "table", metric: "m", columns: ["day", "lines", "extra"] }}
        result={{ columns: ["day", "lines", "extra"], rows: [["2026-09-01", 59]] }}
      />,
    );

    const cells = screen.getAllByRole("cell");
    expect(cells).toHaveLength(3);
    expect(cells[0]).toHaveTextContent("2026-09-01");
    expect(cells[1]).toHaveTextContent("59");
    expect(cells[2]).toHaveTextContent("");
  });

  it("does not spill a long row past the declared columns", () => {
    render(
      <CustomWidget
        widget={{ type: "table", metric: "m", columns: ["day"] }}
        result={{ columns: ["day"], rows: [["2026-09-01", 59, "extra"]] }}
      />,
    );

    const cells = screen.getAllByRole("cell");
    expect(cells).toHaveLength(1);
    expect(cells[0]).toHaveTextContent("2026-09-01");
  });

  it.each([
    ["bar", "custom-bar-chart", "bar"],
    ["area", "custom-area-chart", "area"],
  ])("renders a %s widget from its x and y", (type, frame, series) => {
    render(
      <CustomWidget
        widget={{ type, metric: "m", x: "day", y: "lines" } as never}
        result={result}
      />
    );

    expect(screen.getByTestId(frame)).toBeInTheDocument();
    expect(screen.getByTestId("x-axis")).toHaveAttribute("data-key", "day");
    expect(screen.getByTestId(series)).toHaveAttribute("data-key", "lines");
  });

  it("renders a stat widget as the one number it is", () => {
    render(
      <CustomWidget
        widget={{ type: "stat", metric: "m", value: "lines", label: "Lines" }}
        result={{ columns: ["lines"], rows: [[24887]] }}
      />
    );

    expect(screen.getByTestId("custom-stat")).toHaveTextContent("24,887");
    expect(screen.getByTestId("custom-stat")).toHaveTextContent("Lines");
  });

  it("falls back to the column name when a stat carries no label", () => {
    render(
      <CustomWidget
        widget={{ type: "stat", metric: "m", value: "lines" }}
        result={{ columns: ["lines"], rows: [[7]] }}
      />
    );

    expect(screen.getByTestId("custom-stat")).toHaveTextContent("lines");
  });

  it("renders a pie widget as slices of its value", () => {
    render(
      <CustomWidget
        widget={{ type: "pie", metric: "m", label: "day", value: "lines" }}
        result={result}
      />
    );

    expect(screen.getByTestId("custom-pie-chart")).toBeInTheDocument();
    expect(screen.getByTestId("pie")).toHaveAttribute("data-key", "lines");
    expect(screen.getByTestId("pie")).toHaveAttribute("data-name-key", "day");
  });

  it.each([
    [{ type: "bar", metric: "lines_per_day", x: "day", y: "lines" }, "custom-bar-chart"],
    [{ type: "area", metric: "lines_per_day", x: "day", y: "lines" }, "custom-area-chart"],
    [{ type: "stat", metric: "lines_per_day", value: "lines" }, "custom-stat"],
    [
      { type: "pie", metric: "lines_per_day", label: "day", value: "lines" },
      "custom-pie-chart",
    ],
  ])("names the missing column rather than drawing %#", (widget, frame) => {
    render(
      <CustomWidget
        widget={widget as never}
        result={{ columns: ["day", "total_lines"], rows: [["2026-09-01", 132]] }}
      />
    );

    expect(screen.getByRole("alert")).toHaveTextContent("draws lines");
    expect(screen.queryByTestId(frame)).not.toBeInTheDocument();
  });

  it("says which column is missing instead of drawing an empty chart", () => {
    // Seen live: y named the metric's raw json field instead of its as_name,
    // so every point was undefined and the chart drew axes and no line.
    render(
      <CustomWidget
        widget={{ type: "line", metric: "lines_per_day", x: "day", y: "lines" }}
        result={{
          columns: ["day", "total_lines"],
          rows: [["2026-09-01", 132]],
        }}
      />
    );

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("draws lines");
    expect(alert).toHaveTextContent("lines_per_day does not return");
    expect(alert).toHaveTextContent("day, total_lines");
    expect(screen.queryByTestId("custom-line-chart")).not.toBeInTheDocument();
  });
});

describe("<CustomWidget> over a window that holds nothing", () => {
  it("says the window is empty rather than only that data is missing", () => {
    render(
      <CustomWidget
        widget={{ type: "line", metric: "opened", x: "bucket", y: "opened" }}
        result={{ columns: ["bucket", "opened"], rows: [] }}
        windowed
      />,
    );

    expect(screen.getByRole("status")).toHaveTextContent(/nothing in this window/i);
  });

  it("says only that there is no data when no window was asked for", () => {
    render(
      <CustomWidget
        widget={{ type: "stat", metric: "total", value: "total", label: "Total" }}
        result={{ columns: ["total"], rows: [] }}
      />,
    );

    expect(screen.getByRole("status")).toHaveTextContent(/no data/i);
  });
});
