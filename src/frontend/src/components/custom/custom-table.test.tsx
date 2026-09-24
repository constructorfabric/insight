import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
    expect(cells).toHaveLength(3);
    expect(cells[2]).toHaveTextContent("");
  });

  it("numbers the rows it draws", () => {
    render(
      <CustomTable
        result={{
          columns: ["author"],
          rows: [["Liam Nguyen"], ["Ada Okafor"], ["Mia Castillo"]],
        }}
      />
    );

    const numbered = screen
      .getAllByRole("row")
      .slice(1)
      .map((row) => row.querySelector("td")?.textContent);
    expect(numbered).toEqual(["1", "2", "3"]);
  });

  it("orders a column up, then down, then back as the metric returned it", async () => {
    const user = userEvent.setup();
    render(
      <CustomTable
        result={{ columns: ["runs"], rows: [[9], [2], [10]] }}
      />
    );

    const drawn = () =>
      screen
        .getAllByRole("row")
        .slice(1)
        .map((row) => row.querySelectorAll("td")[1]?.textContent);

    await user.click(screen.getByRole("button", { name: "Order by runs" }));
    expect(drawn()).toEqual(["2", "9", "10"]);

    await user.click(
      screen.getByRole("button", { name: "Order by runs, now ascending" })
    );
    expect(drawn()).toEqual(["10", "9", "2"]);

    await user.click(
      screen.getByRole("button", { name: "Order by runs, now descending" })
    );
    expect(drawn()).toEqual(["9", "2", "10"]);
  });

  // The cap is a drawing limit, so ordering has to happen over everything or
  // it only shuffles the first page.
  it("orders the whole result before it caps what it draws", async () => {
    const user = userEvent.setup();
    render(
      <CustomTable
        result={{
          columns: ["n"],
          rows: Array.from({ length: 250 }, (_, index) => [250 - index]),
        }}
      />
    );

    await user.click(screen.getByRole("button", { name: "Order by n" }));

    const first = screen.getAllByRole("row")[1];
    expect(first?.querySelectorAll("td")[1]?.textContent).toBe("1");
  });

  // The arrow appears in a slot that is there either way: a header that grew
  // when it was ordered by shifted every column beside it.
  it("keeps the same room for the arrow whether or not a column is ordered by", async () => {
    const user = userEvent.setup();
    render(
      <CustomTable result={{ columns: ["tool", "runs"], rows: [["a", 1]] }} />
    );

    const slots = () =>
      screen
        .getAllByRole("columnheader")
        .map((head) => head.querySelectorAll("button > span").length);
    expect(slots()).toEqual([0, 1, 1]);

    await user.click(screen.getByRole("button", { name: "Order by runs" }));

    expect(slots()).toEqual([0, 1, 1]);
  });
});
