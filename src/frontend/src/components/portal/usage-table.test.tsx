// @vitest-environment jsdom
/**
 * A cell's detail can be a whole feedback submission: what does not fit in the
 * window is unreachable unless the tooltip is bounded and scrolls.
 */
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { TruncatedCell, VirtualTable, type Column } from "./usage-table";

async function popupFor(trigger: string) {
  await userEvent.hover(screen.getByText(trigger));

  return waitFor(() => {
    const popup = document.querySelector('[data-slot="tooltip-content"]');
    if (!popup) throw new Error("the tooltip has not opened");
    return popup;
  });
}

describe("TruncatedCell", () => {
  it("bounds a long detail to the room the window has, and scrolls the rest", async () => {
    render(
      <TruncatedCell detail={"the whole report. ".repeat(300)}>
        the whole report.
      </TruncatedCell>,
    );

    const popup = await popupFor("the whole report.");

    expect(popup).toHaveClass(
      "max-h-[var(--available-height)]",
      "overflow-y-auto",
    );
  });

  it("keeps the bound when the caller styles the detail", async () => {
    render(
      <TruncatedCell detail="a long message" detailClassName="max-w-sm text-xs">
        a long message
      </TruncatedCell>,
    );

    const popup = await popupFor("a long message");

    expect(popup).toHaveClass("max-h-[var(--available-height)]", "max-w-sm");
  });
});

describe("VirtualTable", () => {
  interface Row {
    id: string;
  }

  const COLUMNS: Column<Row>[] = [
    { header: "Person", cell: (row) => row.id },
    { header: "Visits", sortKey: "visits", align: "right", cell: () => 1 },
    { header: "Last seen (UTC)", sortKey: "last_seen", cell: () => "1 Aug 09:00" },
  ];

  function renderTable(onSort = vi.fn()) {
    render(
      <VirtualTable
        label="Who opened it"
        rows={[]}
        rowKey={(row) => row.id}
        columns={COLUMNS}
        order={{ sort: "visits", direction: "desc" }}
        onSort={onSort}
      />,
    );
    return onSort;
  }

  it("announces the order the rows are in on the column that holds it", () => {
    renderTable();

    expect(screen.getByRole("columnheader", { name: /Visits/ })).toHaveAttribute(
      "aria-sort",
      "descending",
    );
    expect(screen.getByRole("columnheader", { name: /Last seen/ })).toHaveAttribute(
      "aria-sort",
      "none",
    );
  });

  it("hands a click on a sortable header to whoever owns the order", async () => {
    const onSort = renderTable();

    await userEvent.click(screen.getByRole("button", { name: /Last seen/ }));

    expect(onSort).toHaveBeenCalledWith("last_seen");
  });

  it("tells a chosen order apart from the default one", () => {
    const { rerender } = render(
      <VirtualTable
        label="Who opened it"
        rows={[]}
        rowKey={(row) => row.id}
        columns={COLUMNS}
        order={{ sort: "visits", direction: "desc" }}
        orderIsDefault
        onSort={vi.fn()}
      />,
    );
    const visits = () => screen.getByRole("columnheader", { name: /Visits/ });
    expect(visits()).toHaveAttribute("data-order", "default");

    rerender(
      <VirtualTable
        label="Who opened it"
        rows={[]}
        rowKey={(row) => row.id}
        columns={COLUMNS}
        order={{ sort: "visits", direction: "desc" }}
        onSort={vi.fn()}
      />,
    );
    expect(visits()).toHaveAttribute("data-order", "chosen");
  });

  it("leaves a column nobody can order by as a label", () => {
    renderTable();

    expect(screen.queryByRole("button", { name: "Person" })).toBeNull();
    expect(screen.getByRole("columnheader", { name: "Person" })).not.toHaveAttribute("aria-sort");
  });
});
