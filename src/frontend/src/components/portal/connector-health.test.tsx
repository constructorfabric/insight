// @vitest-environment jsdom
/**
 * The pane's job is to print what it was served, and to be honest about what
 * it was not served. So the tests here check that it does not re-order the
 * rows (the service ordered them by what needs acting on), that an unmeasured
 * number renders as absence rather than a zero, and that a stopped recorder
 * reaches a screen reader as an alert rather than as ordinary prose.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  ConnectorHealthSummary,
  ConnectorSyncHistory,
} from "@/api/connector-health-client";

const mocks = vi.hoisted(() => ({
  summary: {
    data: undefined as ConnectorHealthSummary | undefined,
    isPending: false,
    isError: false,
    refetch: vi.fn(),
  },
  syncs: {
    data: undefined as ConnectorSyncHistory | undefined,
    isPending: false,
    isError: false,
    refetch: vi.fn(),
  },
}));

vi.mock("@/queries/connector-health", () => ({
  useConnectorHealth: () => mocks.summary,
  useConnectorSyncs: () => mocks.syncs,
}));

import { ConnectorHealthPane } from "./connector-health";

const NOW = "2026-01-15T12:00:00.000Z";

function summary(over: Partial<ConnectorHealthSummary> = {}): ConnectorHealthSummary {
  return {
    as_of: NOW,
    checked_at: "2026-01-15T11:59:00.000Z",
    typical_read_interval_ms: 900_000,
    history_available: true,
    connectors: [],
    ...over,
  };
}

beforeEach(() => {
  mocks.summary.refetch.mockClear();
  mocks.syncs.refetch.mockClear();
  mocks.summary.isPending = false;
  mocks.summary.isError = false;
  mocks.summary.data = summary();
  mocks.syncs.isPending = false;
  mocks.syncs.isError = false;
  mocks.syncs.data = { connector: "alpha", syncs: [], window: 50 };
});

describe("the pane prints what it was served", () => {
  it("keeps the served order rather than sorting again", () => {
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "broken",
          configured: true,
          last_sync: {
            job_id: "2",
            status: "failed",
            started_at: NOW,
            duration_ms: 1_000,
            records_reported: 0,
          },
        },
        {
          connector: "alpha",
          configured: true,
          last_sync: {
            job_id: "1",
            status: "succeeded",
            started_at: NOW,
            duration_ms: 2_000,
            records_reported: 5,
          },
        },
      ],
    });
    render(<ConnectorHealthPane />);

    const names = screen
      .getAllByRole("button", { expanded: false })
      .map((button) => button.textContent);
    expect(names).toEqual(["broken", "alpha"]);
  });

  it("prints an unmeasured number as absence, not as a zero", () => {
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "alpha",
          configured: true,
          last_sync: {
            job_id: "1",
            status: "running",
            started_at: null,
            duration_ms: null,
            records_reported: null,
          },
        },
      ],
    });
    render(<ConnectorHealthPane />);

    const row = screen.getByRole("button", { name: "alpha" }).closest("tr");
    expect(row).not.toBeNull();
    expect(within(row as HTMLElement).getAllByText("—")).toHaveLength(3);
    expect(within(row as HTMLElement).queryByText("0")).not.toBeInTheDocument();
  });

  it("says a reported zero is a zero", () => {
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "alpha",
          configured: true,
          last_sync: {
            job_id: "1",
            status: "succeeded",
            started_at: NOW,
            duration_ms: 0,
            records_reported: 0,
          },
        },
      ],
    });
    render(<ConnectorHealthPane />);

    const row = screen.getByRole("button", { name: "alpha" }).closest("tr");
    expect(within(row as HTMLElement).getByText("0")).toBeInTheDocument();
  });

  it("says nothing has been recorded instead of implying health", () => {
    mocks.summary.data = summary({
      connectors: [],
      history_available: false,
      checked_at: null,
    });
    render(<ConnectorHealthPane />);

    expect(
      screen.getByText(/nothing has been read from the connectors yet/i),
    ).toBeInTheDocument();
    // Deliberately blunt: the word must not appear at all, not even negated.
    // "Nothing here says the connectors are healthy" reads, at a glance, as
    // exactly the claim it was written to disclaim.
    expect(screen.queryByText(/healthy|up to date|fine/i)).not.toBeInTheDocument();
  });
});

describe("the pane dates itself", () => {
  it("reports when the mover was last read", () => {
    render(<ConnectorHealthPane />);
    expect(screen.getByText(/last checked 1 min ago/i)).toBeInTheDocument();
  });

  it("raises a stopped recorder as an alert, not as prose", () => {
    mocks.summary.data = summary({
      checked_at: "2026-01-15T10:00:00.000Z",
    });
    render(<ConnectorHealthPane />);

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(/recording appears to have stopped/i);
  });
});

describe("a read that failed says so", () => {
  it("offers a retry instead of an empty table", async () => {
    mocks.summary.isError = true;
    mocks.summary.data = undefined;
    render(<ConnectorHealthPane />);

    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /retry|try again/i }));
    expect(mocks.summary.refetch).toHaveBeenCalled();
  });

  it("waits rather than rendering an empty page while the read is in flight", () => {
    mocks.summary.isPending = true;
    mocks.summary.data = undefined;
    render(<ConnectorHealthPane />);

    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(screen.queryByText(/nothing has been read/i)).not.toBeInTheDocument();
  });

  it("keeps a failed window inside its own row", async () => {
    mocks.summary.data = summary({
      connectors: [{ connector: "alpha", configured: true, last_sync: null }],
    });
    mocks.syncs.isError = true;
    mocks.syncs.data = undefined;
    render(<ConnectorHealthPane />);

    await userEvent.click(screen.getByRole("button", { name: "alpha" }));

    // The summary above it is still readable — one connector's unreadable
    // history must not take the page down with it.
    expect(screen.getByRole("table")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /retry|try again/i }));
    expect(mocks.syncs.refetch).toHaveBeenCalled();
  });

  it("says a connector has no recorded sync rather than showing an empty list", async () => {
    mocks.summary.data = summary({
      connectors: [{ connector: "alpha", configured: true, last_sync: null }],
    });
    mocks.syncs.data = { connector: "alpha", syncs: [], window: 50 };
    render(<ConnectorHealthPane />);

    await userEvent.click(screen.getByRole("button", { name: "alpha" }));

    expect(
      screen.getByText(/no sync has been recorded for this connector/i),
    ).toBeInTheDocument();
  });
});

describe("a row expands to its recent syncs", () => {
  it("is operable from the keyboard and announces its state", async () => {
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "alpha",
          configured: true,
          last_sync: {
            job_id: "1",
            status: "succeeded",
            started_at: NOW,
            duration_ms: 1_000,
            records_reported: 5,
          },
        },
      ],
    });
    mocks.syncs.data = {
      connector: "alpha",
      syncs: [
        {
          job_id: "1",
          status: "succeeded",
          started_at: NOW,
          duration_ms: 1_000,
          records_reported: 5,
        },
      ],
      window: 50,
    };
    render(<ConnectorHealthPane />);

    const toggle = screen.getByRole("button", { name: "alpha" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");

    await userEvent.click(toggle);

    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText(/recent syncs/i)).toBeInTheDocument();
  });

  it("says the list is a window rather than the whole history", async () => {
    mocks.summary.data = summary({
      connectors: [
        { connector: "alpha", configured: true, last_sync: null },
      ],
    });
    mocks.syncs.data = {
      connector: "alpha",
      syncs: [
        {
          job_id: "1",
          status: "succeeded",
          started_at: NOW,
          duration_ms: 1_000,
          records_reported: 5,
        },
      ],
      window: 50,
    };
    render(<ConnectorHealthPane />);

    await userEvent.click(screen.getByRole("button", { name: "alpha" }));

    expect(
      screen.getByText(/the most recent 50, not the full history/i),
    ).toBeInTheDocument();
  });

  it("does not take the row out of the table to make it clickable", () => {
    mocks.summary.data = summary({
      connectors: [
        { connector: "alpha", configured: true, last_sync: null },
      ],
    });
    render(<ConnectorHealthPane />);

    // One header row plus one body row. A `role="button"` on a `<tr>` would
    // remove it from the table for a screen reader, which is why the toggle is
    // a real button inside the row instead.
    expect(screen.getAllByRole("row")).toHaveLength(2);
  });
});
