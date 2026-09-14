import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { CustomTable } from "./custom-table";

describe("<CustomTable>", () => {
  it("links a cell that is a URL, so a row can be followed", () => {
    render(
      <CustomTable
        result={{
          columns: ["pr", "url"],
          rows: [[42, "https://github.com/acme/app/pull/42"]],
        }}
      />
    );

    const link = screen.getByRole("link", {
      name: "https://github.com/acme/app/pull/42",
    });
    expect(link).toHaveAttribute("href", "https://github.com/acme/app/pull/42");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", expect.stringContaining("noreferrer"));
  });

  it("writes a percentage column as a percentage", () => {
    render(
      <CustomTable
        result={{
          columns: ["day", "pass_rate"],
          rows: [["2026-09-01", 83.91167192429022]],
          percents: ["pass_rate"],
        }}
      />
    );

    expect(screen.getByText("83.9%")).toBeInTheDocument();
  });

  it("leaves anything that is not a URL as text", () => {
    render(
      <CustomTable
        result={{
          columns: ["author", "note"],
          rows: [["Liam Nguyen", "see http not a url"]],
        }}
      />
    );

    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(screen.getByText("Liam Nguyen")).toBeInTheDocument();
  });

  it("says how much it is showing when there is more than it draws", () => {
    render(
      <CustomTable
        result={{
          columns: ["n"],
          rows: Array.from({ length: 250 }, (_, index) => [index]),
        }}
      />
    );

    expect(screen.getByText("First 200 of 250 rows")).toBeInTheDocument();
    expect(screen.getAllByRole("row")).toHaveLength(201);
  });

  it("pads a short row rather than misaligning the columns", () => {
    render(
      <CustomTable result={{ columns: ["a", "b"], rows: [["only"]] }} />
    );

    const cells = screen.getAllByRole("cell");
    expect(cells).toHaveLength(2);
    expect(cells[1]).toHaveTextContent("");
  });
});
