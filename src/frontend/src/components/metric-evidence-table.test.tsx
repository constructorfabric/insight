import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MetricEvidenceTable } from "@/components/metric-evidence-table";

const mocks = vi.hoisted(() => ({
  toastError: vi.fn(),
}));

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        start: index * 44,
      })),
    getTotalSize: () => count * 44,
    measureElement: () => undefined,
  }),
}));

vi.mock("sonner", () => ({
  toast: { error: mocks.toastError },
}));

const columns = [
  { key: "ref", label: "Ref", type: "string" as const },
  { key: "value", label: "Value", type: "number" as const },
  { key: "active", label: "Active", type: "string" as const },
];

const rows = [
  { values: { ref: "abc123", value: 1.234, active: true } },
  { values: { ref: null, value: null, active: false } },
];

function renderTable(
  overrides: Partial<React.ComponentProps<typeof MetricEvidenceTable>> = {}
) {
  const props = {
    rows,
    columns,
    sort: null,
    onSortChange: vi.fn(),
    fetchNextPage: vi.fn().mockResolvedValue(undefined),
    hasNextPage: false,
    isFetchingNextPage: false,
    nextPageError: false,
    pageLimitReached: false,
    ...overrides,
  };
  return { ...render(<MetricEvidenceTable {...props} />), props };
}

describe("MetricEvidenceTable", () => {
  beforeEach(() => {
    mocks.toastError.mockReset();
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
    });
  });

  it("preserves table semantics while virtualizing rows", () => {
    renderTable();

    const table = screen.getByRole("table");
    expect(table).toHaveAttribute("aria-rowcount", "2");
    expect(screen.getAllByRole("rowgroup")).toHaveLength(2);
    // Three data columns plus the leading expander column.
    expect(screen.getAllByRole("columnheader")).toHaveLength(4);
    expect(screen.getAllByRole("row")[1]).toHaveAttribute("aria-rowindex", "2");
    expect(screen.getAllByRole("cell")).toHaveLength(8);
    expect(screen.getByText("1.2")).toBeInTheDocument();
    expect(screen.getByText("Yes")).toBeInTheDocument();
    expect(screen.getByText("No")).toBeInTheDocument();
    expect(screen.getAllByText("—")).toHaveLength(2);
  });

  it("announces which column is sorted and which way", () => {
    renderTable({ sort: { key: "value", direction: "desc" } });

    expect(screen.getByRole("columnheader", { name: "Value" })).toHaveAttribute(
      "aria-sort",
      "descending"
    );
    expect(screen.getByRole("columnheader", { name: "Ref" })).toHaveAttribute(
      "aria-sort",
      "none"
    );
  });

  it("asks for a sort when a header is clicked", async () => {
    const user = userEvent.setup();
    const onSortChange = vi.fn();
    renderTable({ onSortChange });

    await user.click(screen.getByRole("button", { name: "Value" }));
    expect(onSortChange).toHaveBeenCalledWith("value");
  });

  describe("the full record", () => {
    const message = "Add the parser\n\nIt handles nested groups.";
    const longRows = [
      { values: { ref: "abc123", value: 1, active: message } },
      { values: { ref: "def456", value: 2, active: "Short" } },
    ];

    it("shows only the first line in the cell", () => {
      renderTable({ rows: longRows });
      expect(screen.getByText("Add the parser")).toBeInTheDocument();
      expect(
        screen.queryByText(/It handles nested groups/)
      ).not.toBeInTheDocument();
    });

    it("reveals the rest, newlines and all, when the row is expanded", async () => {
      const user = userEvent.setup();
      renderTable({ rows: longRows });
      const [toggle] = screen.getAllByRole("button", {
        name: "Show full record",
      });

      await user.click(toggle!);
      expect(toggle).toHaveAttribute("aria-expanded", "true");
      const detail = screen.getByText(/It handles nested groups/);
      expect(detail).toHaveTextContent("Add the parser");
      expect(detail).toHaveClass("whitespace-pre-wrap");
    });

    it("opens from a click on the row, where the cut text actually is", async () => {
      const user = userEvent.setup();
      renderTable({ rows: longRows });

      await user.click(screen.getByText("Add the parser"));
      expect(screen.getByText(/It handles nested groups/)).toBeInTheDocument();
    });

    it("leaves the row alone when the copy button is pressed", async () => {
      const user = userEvent.setup();
      renderTable({ rows: longRows });

      await user.click(screen.getByRole("button", { name: "Copy abc123" }));
      expect(
        screen.queryByText(/It handles nested groups/)
      ).not.toBeInTheDocument();
    });

    it("shows only the visible line in the hover text, not the whole field", () => {
      renderTable({ rows: longRows });
      expect(screen.getByText("Add the parser").closest("td")).toHaveAttribute(
        "title",
        "Add the parser"
      );
    });

    it("closes again on a second press", async () => {
      const user = userEvent.setup();
      renderTable({ rows: longRows });
      const [toggle] = screen.getAllByRole("button", {
        name: "Show full record",
      });

      await user.click(toggle!);
      await user.click(
        screen.getByRole("button", { name: "Hide full record" })
      );
      expect(
        screen.queryByText(/It handles nested groups/)
      ).not.toBeInTheDocument();
    });

    it("keeps a record open across a re-sort rather than following its position", async () => {
      const user = userEvent.setup();
      const { rerender, props } = renderTable({ rows: longRows });

      await user.click(
        screen.getAllByRole("button", { name: "Show full record" })[0]!
      );
      rerender(
        <MetricEvidenceTable {...props} rows={[...longRows].reverse()} />
      );

      expect(screen.getByText(/It handles nested groups/)).toBeInTheDocument();
      expect(
        screen.getAllByRole("button", { name: "Hide full record" })
      ).toHaveLength(1);
    });
  });

  it("copies references and reports clipboard failures", async () => {
    const user = userEvent.setup();
    const writeText = vi
      .spyOn(navigator.clipboard, "writeText")
      .mockResolvedValue(undefined);
    const { rerender, props } = renderTable();

    await user.click(screen.getByRole("button", { name: "Copy abc123" }));
    expect(writeText).toHaveBeenCalledWith("abc123");
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();

    writeText.mockRejectedValue(new Error("denied"));
    rerender(<MetricEvidenceTable {...props} />);
    await user.click(screen.getByRole("button", { name: "Copied" }));
    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith("Unable to copy ref")
    );
  });

  it("loads the next page near the end and renders progress states", async () => {
    const fetchNextPage = vi.fn().mockResolvedValue(undefined);
    const { rerender, props } = renderTable({
      fetchNextPage,
      hasNextPage: true,
    });
    await waitFor(() => expect(fetchNextPage).toHaveBeenCalledTimes(1));

    rerender(
      <MetricEvidenceTable {...props} hasNextPage={false} isFetchingNextPage />
    );
    expect(screen.getByRole("status", { name: "Loading" })).toBeInTheDocument();

    rerender(
      <MetricEvidenceTable {...props} hasNextPage={false} nextPageError />
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Unable to load more rows"
    );

    rerender(
      <MetricEvidenceTable {...props} hasNextPage={false} pageLimitReached />
    );
    expect(
      screen.getByText(/Showing the first 5,000 rows/)
    ).toBeInTheDocument();
  });
});
