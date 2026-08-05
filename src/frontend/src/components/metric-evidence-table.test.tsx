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
    expect(screen.getAllByRole("columnheader")).toHaveLength(3);
    expect(screen.getAllByRole("row")[1]).toHaveAttribute("aria-rowindex", "2");
    expect(screen.getAllByRole("cell")).toHaveLength(6);
    expect(screen.getByText("1.2")).toBeInTheDocument();
    expect(screen.getByText("Yes")).toBeInTheDocument();
    expect(screen.getByText("No")).toBeInTheDocument();
    expect(screen.getAllByText("—")).toHaveLength(2);
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
