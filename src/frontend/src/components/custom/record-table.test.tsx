import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { DeclaredField } from "@/api/custom-client";

import { ARRIVED, RecordTable } from "./record-table";

const FIELDS: DeclaredField[] = [
  { name: "author", path: "who.email", type: "string" },
  { name: "lines", path: "lines", type: "int" },
  { name: "tags", path: "tags", type: "string" },
];

const RECORDS = [
  {
    id: "1",
    received_at: "2026-09-17 10:00:00",
    raw_data: { who: { email: "ada@example.com" }, lines: 7, tags: ["a", "b"] },
  },
];

function draw(
  shown = ["author", "lines"],
  onShow = vi.fn(),
  onOrder = vi.fn()
) {
  render(
    <RecordTable
      fields={FIELDS}
      records={RECORDS}
      shown={shown}
      ordering={{ by: ARRIVED, descending: true }}
      onShow={onShow}
      onOrder={onOrder}
    />
  );
  return { onShow, onOrder };
}

describe("<RecordTable>", () => {
  // The columns are the declaration's, not the keys a record happens to hold.
  it("draws a column per chosen declared field, read by its path", () => {
    draw();

    expect(
      screen.getByRole("columnheader", { name: "author" })
    ).toBeInTheDocument();
    expect(
      screen.getByRole("cell", { name: "ada@example.com" })
    ).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "7" })).toBeInTheDocument();
    expect(
      screen.queryByRole("columnheader", { name: "tags" })
    ).not.toBeInTheDocument();
  });

  it("says how many of the declared fields it is drawing", () => {
    draw();

    expect(
      screen.getByRole("button", { name: /Columns · 2 of 3/ })
    ).toBeInTheDocument();
  });

  it("offers every declared field, and reports the one toggled", async () => {
    const user = userEvent.setup();
    const { onShow } = draw();

    await user.click(screen.getByRole("button", { name: /Columns/ }));
    await user.click(screen.getByRole("checkbox", { name: /tags/ }));

    // Kept in declaration order, whatever order they were picked in.
    expect(onShow).toHaveBeenCalledWith(["author", "lines", "tags"]);
  });

  it("asks for a column's order, and turns it around when asked again", async () => {
    const user = userEvent.setup();
    const onOrder = vi.fn();
    render(
      <RecordTable
        fields={FIELDS}
        records={RECORDS}
        shown={["author"]}
        ordering={{ by: "author", descending: true }}
        onShow={vi.fn()}
        onOrder={onOrder}
      />
    );

    await user.click(screen.getByRole("button", { name: "Order by author" }));

    expect(onOrder).toHaveBeenCalledWith({ by: "author", descending: false });
  });

  // A cell holding a list or an object shows it compactly; the whole record
  // is a click away.
  it("shows a value that is not a scalar as the JSON it is", async () => {
    const user = userEvent.setup();
    draw(["tags"]);

    expect(screen.getByRole("cell", { name: '["a","b"]' })).toBeInTheDocument();

    await user.click(screen.getByRole("cell", { name: '["a","b"]' }));

    expect(screen.getByText(/"email": "ada@example.com"/)).toBeInTheDocument();
  });

  it("leaves a cell empty where the record has no value", () => {
    render(
      <RecordTable
        fields={FIELDS}
        records={[{ id: "1", received_at: "now", raw_data: {} }]}
        shown={["author"]}
        ordering={{ by: ARRIVED, descending: true }}
        onShow={vi.fn()}
        onOrder={vi.fn()}
      />
    );

    const cells = screen.getAllByRole("cell");
    expect(cells[1]).toHaveTextContent("");
  });
});
