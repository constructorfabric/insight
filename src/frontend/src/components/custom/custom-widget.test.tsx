import { cloneElement, isValidElement, type ReactNode } from "react";
import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

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

import { CustomApiError } from "@/api/custom-client";

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
      />
    );

    expect(
      screen.getByRole("columnheader", { name: "day" })
    ).toBeInTheDocument();
    expect(screen.getAllByRole("row")).toHaveLength(3);
  });

  it("renders a line widget as a chart", () => {
    render(
      <CustomWidget
        widget={{ type: "line", metric: "m", x: "day", y: "lines" }}
        result={result}
      />
    );

    expect(
      screen.getByRole("figure", { name: "line chart" })
    ).toBeInTheDocument();
  });

  it("reads the widget's x column along the axis and names its y series", () => {
    render(
      <CustomWidget
        widget={{ type: "line", metric: "m", x: "day", y: "lines" }}
        result={result}
      />
    );
    const figure = screen.getByRole("figure", { name: "line chart" });

    expect(within(figure).getByText("Sep 1")).toBeInTheDocument();
    expect(within(figure).getByRole("listitem")).toHaveTextContent("lines");
  });

  // A card is where a reader meets a refusal, so it shows what the service
  // said rather than the status it said it with.
  it("shows the service's refusal in place of the content", () => {
    render(
      <CustomWidget
        widget={{ type: "table", metric: "m", columns: ["day"] }}
        error={
          new CustomApiError(400, {
            detail: "a metric reads a dataset, so it may not name a `table`",
          })
        }
      />
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "a metric reads a dataset, so it may not name a `table`"
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent("400");
  });

  it("says plainly when the failure carries nothing readable", () => {
    render(
      <CustomWidget
        widget={{ type: "table", metric: "m", columns: ["day"] }}
        error={new TypeError("Failed to fetch")}
      />
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "The metric could not be run."
    );
  });

  it("says there is no data when the metric returned no rows", () => {
    render(
      <CustomWidget
        widget={{ type: "table", metric: "m", columns: ["day"] }}
        result={{ columns: ["day"], rows: [] }}
      />
    );

    expect(screen.getByText(/no data/i)).toBeInTheDocument();
  });

  it("says so when the type is unknown", () => {
    render(
      <CustomWidget
        widget={{ type: "sankey", metric: "m" } as never}
        result={result}
      />
    );

    expect(screen.getByText(/unknown widget type/i)).toBeInTheDocument();
  });

  // Switching boards leaves a card with no result for a moment. Saying "No
  // data" there reads as an answer, and then the answer changes.
  it("waits rather than saying there is no data while the run is in flight", () => {
    const { rerender } = render(
      <CustomWidget
        widget={{ type: "table", metric: "m", columns: ["day"] }}
        pending
      />
    );

    expect(screen.queryByText(/no data/i)).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toBeInTheDocument();

    rerender(
      <CustomWidget widget={{ type: "table", metric: "m", columns: ["day"] }} />
    );

    expect(screen.queryByText(/no data/i)).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("pads a short row with empty cells instead of misaligning columns", () => {
    render(
      <CustomWidget
        widget={{
          type: "table",
          metric: "m",
          columns: ["day", "lines", "extra"],
        }}
        result={{
          columns: ["day", "lines", "extra"],
          rows: [["2026-09-01", 59]],
        }}
      />
    );

    const cells = screen.getAllByRole("cell");
    expect(cells).toHaveLength(4);
    expect(cells[1]).toHaveTextContent("2026-09-01");
    expect(cells[2]).toHaveTextContent("59");
    expect(cells[3]).toHaveTextContent("");
  });

  it("does not spill a long row past the declared columns", () => {
    render(
      <CustomWidget
        widget={{ type: "table", metric: "m", columns: ["day"] }}
        result={{ columns: ["day"], rows: [["2026-09-01", 59, "extra"]] }}
      />
    );

    const cells = screen.getAllByRole("cell");
    expect(cells).toHaveLength(2);
    expect(cells[1]).toHaveTextContent("2026-09-01");
  });

  it.each([
    [{ type: "bar", metric: "m", x: "day", y: "lines" }],
    [{ type: "area", metric: "m", x: "day", y: "lines" }],
    [{ type: "donut", metric: "m", label: "day", value: "lines" }],
    [{ type: "composed", metric: "m", x: "day", y: "lines", y2: "lines" }],
    [{ type: "heatmap", metric: "m", x: "day", value: "lines" }],
    [{ type: "pulse", metric: "m", x: "day", y: "lines" }],
  ])("renders a %o through its own kind", (widget) => {
    render(<CustomWidget widget={widget as never} result={result} />);

    expect(
      screen.getByRole("figure", { name: `${widget.type} chart` })
    ).toBeInTheDocument();
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

    const figure = screen.getByRole("figure", { name: "pie chart" });

    expect(
      within(figure)
        .getAllByRole("listitem")
        .map((item) => item.textContent)
    ).toEqual(["2026-09-01", "2026-09-02"]);
  });

  it.each([
    [{ type: "bar", metric: "lines_per_day", x: "day", y: "lines" }],
    [{ type: "area", metric: "lines_per_day", x: "day", y: "lines" }],
    [{ type: "stat", metric: "lines_per_day", value: "lines" }],
    [{ type: "pie", metric: "lines_per_day", label: "day", value: "lines" }],
    [
      {
        type: "line",
        metric: "lines_per_day",
        x: "day",
        y: "total_lines",
        target: "lines",
      },
    ],
    [
      {
        type: "composed",
        metric: "lines_per_day",
        x: "day",
        y: "total_lines",
        y2: "lines",
      },
    ],
    [
      {
        type: "bubble",
        metric: "lines_per_day",
        x: "day",
        y: "total_lines",
        size: "lines",
      },
    ],
  ])("names the missing column rather than drawing %o", (widget) => {
    render(
      <CustomWidget
        widget={widget as never}
        result={{
          columns: ["day", "total_lines"],
          rows: [["2026-09-01", 132]],
        }}
      />
    );

    expect(screen.getByRole("alert")).toHaveTextContent("draws lines");
    expect(screen.queryByRole("figure")).not.toBeInTheDocument();
    expect(screen.queryByTestId("custom-stat")).not.toBeInTheDocument();
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
    expect(screen.queryByRole("figure")).not.toBeInTheDocument();
  });
});

describe("<CustomWidget> over a window that holds nothing", () => {
  it("says the window is empty rather than only that data is missing", () => {
    render(
      <CustomWidget
        widget={{ type: "line", metric: "opened", x: "bucket", y: "opened" }}
        result={{ columns: ["bucket", "opened"], rows: [] }}
        windowed
      />
    );

    expect(screen.getByRole("status")).toHaveTextContent(
      /nothing in this window/i
    );
  });

  it("says only that there is no data when no window was asked for", () => {
    render(
      <CustomWidget
        widget={{
          type: "stat",
          metric: "total",
          value: "total",
          label: "Total",
        }}
        result={{ columns: ["total"], rows: [] }}
      />
    );

    expect(screen.getByRole("status")).toHaveTextContent(/no data/i);
  });
});
