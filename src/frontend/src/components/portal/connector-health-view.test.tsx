// @vitest-environment jsdom
/**
 * The connector-health pane reports recorded facts and nothing else.
 *
 * The cases below are the ones a reader could be misled by: an unmeasured
 * delivery must not read as zero, a sync nobody claimed must not read as manual,
 * and a page with no recorded history must not read as health.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type {
  ConnectorHealthResponse,
  ConnectorHealthRow,
  ConnectorRunsResponse,
} from "@/api/connector-health-client";

const health = vi.hoisted(() => ({
  value: {
    data: undefined as ConnectorHealthResponse | undefined,
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  },
}));

const runs = vi.hoisted(() => ({
  value: {
    data: undefined as ConnectorRunsResponse | undefined,
    isLoading: false,
    isError: false,
  },
}));

vi.mock("@/queries/connector-health", () => ({
  useConnectorHealth: () => health.value,
  useConnectorRuns: () => runs.value,
}));

import { ConnectorHealthView } from "./connector-health-view";

function row(over: Partial<ConnectorHealthRow> = {}): ConnectorHealthRow {
  return {
    connector: "example-tool",
    configured: true,
    last_run: {
      status: "ok",
      step: null,
      started_at: "2026-01-15T09:00:00Z",
      duration_ms: 90_000,
      transform_status: "ok",
    },
    last_sync: {
      trigger: "claimed",
      status: "ok",
      started_at: "2026-01-15T09:00:00Z",
      duration_ms: 60_000,
      records_moved: 400,
      rows_landed: 400,
    },
    storage: {
      observed_at: "2026-01-15T09:00:00Z",
      streams: 4,
      streams_with_data: 4,
      physical_rows: 1_000,
      bytes_on_disk: 1024,
    },
    streams: [],
    ...over,
  };
}

function respond(
  connectors: ConnectorHealthRow[],
  over: Partial<ConnectorHealthResponse> = {}
) {
  health.value.data = {
    as_of: new Date().toISOString(),
    swept_at: new Date(Date.now() - 5 * 60_000).toISOString(),
    history_available: true,
    connectors,
    ...over,
  };
}

beforeEach(() => {
  health.value.isLoading = false;
  health.value.isError = false;
  health.value.data = undefined;
  runs.value.data = { connector: "example-tool", runs: [] };
  runs.value.isLoading = false;
  runs.value.isError = false;
});

describe("connector health · what a row says", () => {
  it("names a delivery mismatch rather than calling the connector healthy", () => {
    respond([
      row({ last_sync: { ...row().last_sync!, records_moved: 12_400, rows_landed: 0 } }),
    ]);
    render(<ConnectorHealthView />);

    expect(screen.getByText("recorded, nothing landed")).toBeInTheDocument();
    expect(screen.getByText("12,400 / 0")).toBeInTheDocument();
  });

  it("shows an unmeasured delivery as unmeasured, never as zero", () => {
    respond([
      row({ last_sync: { ...row().last_sync!, records_moved: 5_000, rows_landed: null } }),
    ]);
    render(<ConnectorHealthView />);

    expect(screen.getByText("5,000 / not measured")).toBeInTheDocument();
    expect(screen.queryByText("recorded, nothing landed")).not.toBeInTheDocument();
  });

  it("says a sync ran without a transform when only the mover started it", () => {
    respond([
      row({
        last_run: null,
        last_sync: { ...row().last_sync!, trigger: "out_of_band" },
      }),
    ]);
    render(<ConnectorHealthView />);

    expect(screen.getAllByText("sync without transform")).not.toHaveLength(0);
    expect(
      screen.getByText("started outside the pipeline"),
    ).toBeInTheDocument();
  });

  it("does not present an unclaimed sync as a manual one", () => {
    respond([
      row({ last_sync: { ...row().last_sync!, trigger: "unclaimed" } }),
    ]);
    render(<ConnectorHealthView />);

    expect(screen.getByText("origin unknown")).toBeInTheDocument();
    expect(screen.queryByText(/manual/i)).not.toBeInTheDocument();
  });

  it("names the step a failed run stopped at", () => {
    respond([
      row({
        last_run: { ...row().last_run!, status: "failed", step: "resolve" },
      }),
    ]);
    render(<ConnectorHealthView />);

    expect(screen.getAllByText("run failed")).not.toHaveLength(0);
    expect(screen.getByText(/stopped at resolve/)).toBeInTheDocument();
  });

  it("separates a schema nobody configured from one that never ran", () => {
    respond([
      row({ connector: "ghost", configured: false, last_run: null, last_sync: null }),
      row({ connector: "waiting", configured: true, last_run: null, last_sync: null }),
    ]);
    render(<ConnectorHealthView />);

    expect(screen.getAllByText("not configured")).not.toHaveLength(0);
    expect(screen.getAllByText("never ran")).not.toHaveLength(0);
  });
});

describe("connector health · the page as a whole", () => {
  it("does not read as health when nothing has recorded a run", () => {
    respond([row({ last_run: null, last_sync: null })], {
      history_available: false,
    });
    render(<ConnectorHealthView />);

    expect(
      screen.getByText(/Nothing has recorded an ingestion run/),
    ).toBeInTheDocument();
  });

  it("states the swept time from the recorded marker, not from its own clock", () => {
    // `as_of` is the reader's own clock and would read as "just now" however
    // long ago the controller last ran — the one fabricated claim on the page.
    respond([row()], {
      as_of: new Date().toISOString(),
      swept_at: new Date(Date.now() - 3 * 3_600_000).toISOString(),
    });
    render(<ConnectorHealthView />);

    expect(screen.getByText(/last swept 3h ago/)).toBeInTheDocument();
  });

  it("says never swept when no tick has finished", () => {
    respond([row()], { swept_at: null });
    render(<ConnectorHealthView />);

    expect(screen.getByText(/never swept/)).toBeInTheDocument();
  });

  it("counts states in tiles that cannot disagree with the badges", () => {
    respond([
      row({ connector: "a", last_run: { ...row().last_run!, status: "failed" } }),
      row({ connector: "b", last_run: { ...row().last_run!, status: "failed" } }),
      row({ connector: "c" }),
    ]);
    render(<ConnectorHealthView />);

    const tile = screen.getByText("run failed", { selector: "div" });
    expect(tile.parentElement?.textContent).toMatch(/^2/);
    expect(screen.getAllByText("run failed")).toHaveLength(3); // one tile, two badges
  });

  it("offers a retry rather than an empty table when the read failed", async () => {
    health.value.isError = true;
    render(<ConnectorHealthView />);

    await userEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(health.value.refetch).toHaveBeenCalled();
  });
});

describe("connector health · expansion", () => {
  it("shows an empty stream as empty rather than as a zero to be read past", async () => {
    respond([
      row({
        streams: [
          { stream: "messages", physical_rows: 158_000, bytes_on_disk: 7_340_032 },
          { stream: "reactions", physical_rows: 0, bytes_on_disk: 0 },
        ],
      }),
    ]);
    render(<ConnectorHealthView />);

    await userEvent.click(screen.getByText("example-tool"));

    const detail = screen
      .getByRole("heading", { name: "Streams" })
      .closest("section")!;
    expect(within(detail).getByText("reactions")).toBeInTheDocument();
    expect(within(detail).getByText("empty")).toBeInTheDocument();
    // A populated stream shows what it holds AND what it costs.
    expect(within(detail).getByText(/158,000 · 7\.0 MiB/)).toBeInTheDocument();
  });

  it("labels stored rows as physical so nothing reads them as entities", async () => {
    respond([
      row({ streams: [{ stream: "items", physical_rows: 9, bytes_on_disk: 9 }] }),
    ]);
    render(<ConnectorHealthView />);

    await userEvent.click(screen.getByText("example-tool"));

    expect(screen.getByText(/does not count entities/)).toBeInTheDocument();
  });

  it("opens a row from the keyboard, not only with a mouse", async () => {
    // The expansion is the whole drill-down; a mouse-only control puts it out of
    // reach of a keyboard or screen-reader operator entirely.
    respond([row()]);
    render(<ConnectorHealthView />);

    const summary = screen.getByRole("button", { name: /example-tool/ });
    summary.focus();
    await userEvent.keyboard("{Enter}");

    expect(
      screen.getByRole("heading", { name: "Recent runs" }),
    ).toBeInTheDocument();
  });

  it("names who recorded an event and what it moved", async () => {
    // Without these the history reads "ok · 2h ago" twelve times over, and the
    // most likely investigation on this page dead-ends.
    runs.value.data = {
      connector: "example-tool",
      runs: [
        {
          event: "sync.completed",
          status: "ok",
          step: null,
          origin: "sweep",
          trigger: "out_of_band",
          started_at: "2026-01-15T09:00:00Z",
          duration_ms: 60_000,
          records_moved: 12_400,
          rows_landed: null,
        },
      ],
    };
    respond([row()]);
    render(<ConnectorHealthView />);

    await userEvent.click(screen.getByText("example-tool"));

    expect(screen.getByText(/recorded by sweep/)).toBeInTheDocument();
    expect(
      screen.getByText(/started outside the pipeline/),
    ).toBeInTheDocument();
    expect(screen.getByText(/12,400 \/ not measured/)).toBeInTheDocument();
  });

  it("says how many recorded events it is not showing", async () => {
    runs.value.data = {
      connector: "example-tool",
      runs: Array.from({ length: 30 }, (_unused, index) => ({
        event: "run.finished",
        status: "ok",
        step: null,
        origin: "pipeline",
        trigger: null,
        job_id: null,
        started_at: `2026-01-${String(index + 1).padStart(2, "0")}T09:00:00Z`,
        duration_ms: 1_000,
        records_moved: null,
        rows_landed: null,
      })),
    };
    respond([row()]);
    render(<ConnectorHealthView />);

    await userEvent.click(screen.getByText("example-tool"));

    // The response is capped, so its length is not the history. Saying "of 30
    // recorded events" claimed a total nobody counted.
    const note = screen.getByText(/Showing 12 of the 30 most recent/);
    expect(note).toBeInTheDocument();
    expect(note).toHaveTextContent(/older ones are not read here/);
  });

  it("collapses a row that is clicked again", async () => {
    respond([row()]);
    render(<ConnectorHealthView />);

    await userEvent.click(screen.getByText("example-tool"));
    expect(
      screen.getByRole("heading", { name: "Recent runs" }),
    ).toBeInTheDocument();

    await userEvent.click(screen.getByText("example-tool"));
    expect(
      screen.queryByRole("heading", { name: "Recent runs" }),
    ).not.toBeInTheDocument();
  });
});
