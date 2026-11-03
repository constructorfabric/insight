// @vitest-environment jsdom
/**
 * The pane's job is to print what it was served, and to be honest about what
 * it was not served. So the tests here check that it does not re-order the
 * rows (the service ordered them by what needs acting on), that an unmeasured
 * number renders as absence rather than a zero, and that a stopped recorder
 * reaches a screen reader as an alert rather than as ordinary prose.
 */
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  ConnectorHealthSummary,
  ConnectorSyncHistory,
} from "@/api/connector-health-client";

/**
 * A connector's row. The whole row is the disclosure control — the same rule
 * every console listing follows — so there is no button inside it to aim at.
 */
function rowFor(connector: string): HTMLElement {
  const row = screen.getByRole("cell", { name: connector }).closest("tr");
  if (row === null) throw new Error(`no row for ${connector}`);
  return row;
}

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
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  mocks.summary.refetch.mockClear();
  mocks.syncs.refetch.mockClear();
  mocks.summary.isPending = false;
  mocks.summary.isError = false;
  mocks.summary.data = summary();
  mocks.syncs.isPending = false;
  mocks.syncs.isError = false;
  mocks.syncs.data = { connector: "alpha", syncs: [], window: 50 };
});

afterEach(() => {
  vi.useRealTimers();
});

describe("the schedule forecast follows the viewer's clock", () => {
  beforeEach(() => {
    vi.useRealTimers();
    vi.useFakeTimers({ toFake: ["Date", "setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
    vi.setSystemTime(NOW);
    mocks.summary.data = summary({
      connectors: [{
        connector: "example-tracker",
        configured: true,
        schedule: { crons: ["0 * * * *"], timezone: "UTC", suspended: false },
        last_sync: null,
      }],
    });
  });

  function forecast() {
    render(<ConnectorHealthPane />);
    return within(rowFor("example-tracker")).getByRole("button");
  }

  it("uses the clock at opening rather than the response time or render time", async () => {
    const trigger = forecast();
    vi.setSystemTime(new Date("2026-01-15T14:00:00.000Z"));

    await act(async () => { trigger.focus(); });

    expect(trigger).toHaveFocus();
    expect(screen.getByRole("tooltip")).toHaveTextContent("Next run 2026-01-15 15:00 UTC (in 1h 0m)");
  });

  it("moves to the following start on the next 30-second tick without a refetch", async () => {
    const openingTime = "2026-01-15T12:59:45.000Z";
    vi.setSystemTime(new Date(openingTime));
    mocks.summary.data!.as_of = openingTime;
    const trigger = forecast();

    await act(async () => { trigger.focus(); });
    expect(screen.getByRole("tooltip")).toHaveTextContent("Next run 2026-01-15 13:00 UTC (in 15.0 s)");

    await act(async () => { vi.advanceTimersByTime(29_999); });
    expect(screen.getByRole("tooltip")).toHaveTextContent("Next run 2026-01-15 13:00 UTC (in 15.0 s)");

    await act(async () => { vi.advanceTimersByTime(1); });
    expect(screen.getByRole("tooltip")).toHaveTextContent("Next run 2026-01-15 14:00 UTC (in 59m 45s)");
    expect(mocks.summary.refetch).not.toHaveBeenCalled();
  });

  it("captures the new clock when reopened after time passes while closed", async () => {
    const trigger = forecast();

    await act(async () => { trigger.focus(); });
    expect(screen.getByRole("tooltip")).toHaveTextContent("Next run 2026-01-15 13:00 UTC (in 1h 0m)");

    await act(async () => { trigger.blur(); });
    await act(async () => { vi.advanceTimersByTime(2 * 60 * 60 * 1_000); });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();

    await act(async () => { trigger.focus(); });
    expect(screen.getByRole("tooltip")).toHaveTextContent("Next run 2026-01-15 15:00 UTC (in 1h 0m)");
  });
});

describe("the pane prints what it was served", () => {
  it("shows suspension and exposes its forecast on keyboard focus", async () => {
    mocks.summary.data = summary({
      connectors: [{
        connector: "example-tracker",
        configured: true,
        schedule: { crons: ["0 22 * * *"], timezone: "UTC", origin: "annotation", suspended: true },
        last_sync: null,
      }],
    });
    render(<ConnectorHealthPane />);

    const row = rowFor("example-tracker");
    expect(within(row).getByText("UTC · Secret annotation · suspended")).toHaveClass("text-muted-foreground");
    const trigger = within(row).getByRole("button");
    const user = userEvent.setup();
    await user.tab();
    await user.tab();
    expect(trigger).toHaveFocus();
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Suspended: no run is scheduled.");
  });

  it("keeps each sibling's cron and muted schedule line on its own row", () => {
    const schedules = [
      { crons: ["0 5 * * *"], timezone: "UTC", origin: "annotation", suspended: true },
      { crons: ["0 17 * * *"], timezone: "Europe/Berlin", origin: "descriptor", suspended: false },
    ] as const;
    mocks.summary.data = summary({
      connectors: schedules.map((schedule, index) => ({
        connector: "example-tracker",
        tenant_id: "example-tenant",
        source_id: `source-${index}`,
        configured: true,
        schedule: { ...schedule, crons: [...schedule.crons] },
        last_sync: null,
      })),
    });
    render(<ConnectorHealthPane />);

    const rows = screen.getAllByRole("row", { expanded: false });
    expect(rows).toHaveLength(2);
    for (const [index, detail] of ["UTC · Secret annotation · suspended", "Europe/Berlin · connector default"].entries()) {
      const cell = within(rows[index]).getAllByRole("cell")[2];
      expect(within(cell).getByText(schedules[index].crons[0])).toHaveClass("font-mono");
      expect(within(cell).getByText(detail)).toHaveClass("text-muted-foreground");
      expect(within(cell).queryByText(schedules[1 - index].crons[0])).not.toBeInTheDocument();
    }
  });

  it.each(["click", "Enter", "Space"])("the schedule button leaves expansion unchanged on %s", async (gesture) => {
    mocks.summary.data = summary({
      connectors: [{
        connector: "example-tracker",
        configured: true,
        schedule: { crons: ["0 22 * * *"], timezone: "UTC", suspended: false },
        last_sync: null,
      }],
    });
    render(<ConnectorHealthPane />);

    const row = rowFor("example-tracker");
    const trigger = within(row).getByRole("button");
    const user = userEvent.setup();
    for (const expanded of [false, true]) {
      if (expanded) await user.click(within(row).getByRole("cell", { name: "example-tracker" }));
      trigger.focus();
      if (gesture === "click") {
        await user.click(trigger);
      } else {
        await user.keyboard(gesture === "Enter" ? "{Enter}" : " ");
      }
      expect(row).toHaveAttribute("aria-expanded", String(expanded));
    }
  });

  it.each(["hover", "focus"] as const)("exposes the schedule forecast on %s", async (gesture) => {
    mocks.summary.data = summary({
      connectors: [{
        connector: "example-tracker",
        configured: true,
        schedule: { crons: ["0 22 * * *"], timezone: "Europe/Sofia", suspended: false },
        last_sync: null,
      }],
    });
    render(<ConnectorHealthPane />);

    const cell = within(rowFor("example-tracker")).getAllByRole("cell")[2];
    const trigger = within(cell).getByRole("button");
    const user = userEvent.setup();
    if (gesture === "hover") {
      await user.hover(trigger);
    } else {
      await user.tab();
      await user.tab();
      expect(trigger).toHaveFocus();
    }

    const tooltip = await screen.findByRole("tooltip");
    expect(tooltip).toHaveTextContent("Next run 2026-01-15 22:00 Europe/Sofia (in 8h 0m)");
    expect(trigger).toHaveAttribute("aria-describedby", tooltip.id);
    expect(rowFor("example-tracker")).toHaveAttribute("aria-expanded", "false");
  });
  it("shows the recorded schedule after the instance, with each cron on its own line", () => {
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "example-tracker",
          configured: true,
          schedule: {
            suspended: false,
            crons: ["0 22 * * *", "0 10 * * *"],
            timezone: "Europe/Sofia",
            origin: "annotation",
          },
          last_sync: null,
        },
      ],
    });
    render(<ConnectorHealthPane />);

    const headings = screen
      .getAllByRole("columnheader")
      .map((cell) => cell.textContent);
    expect(headings).toEqual([
      "Connector",
      "Instance",
      "Schedule",
      "State",
      "Last sync started",
      "Duration",
      "Records reported",
    ]);
    const cell = within(rowFor("example-tracker")).getAllByRole("cell")[2];
    const first = within(cell).getByText("0 22 * * *");
    const second = within(cell).getByText("0 10 * * *");
    expect(first).toHaveClass("block");
    expect(second).toHaveClass("block");
    expect(first).toHaveClass("font-mono");
    expect(second).toHaveClass("font-mono");
    expect(within(cell).getByText("Europe/Sofia · Secret annotation")).toHaveClass(
      "text-muted-foreground",
    );
  });

  it.each([
    [null, "descriptor", "controller time · connector default"],
    [null, "default", "controller time · built-in default"],
    ["UTC", null, "UTC"],
    [null, null, "controller time"],
  ] as const)(
    "prints recorded timezone %s and origin %s",
    (timezone, origin, detail) => {
      mocks.summary.data = summary({
        connectors: [
          {
            connector: "example-tracker",
            configured: true,
            schedule: { crons: ["0 22 * * *"], timezone, origin, suspended: false },
            last_sync: null,
          },
        ],
      });
      render(<ConnectorHealthPane />);

      const cell = within(rowFor("example-tracker")).getAllByRole("cell")[2];
      expect(within(cell).getByText(detail)).toBeInTheDocument();
      expect(cell.textContent).not.toMatch(/ · $/);
    },
  );

  it("prints an unrecorded schedule as unknown and spans all seven columns when expanded", async () => {
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "example-tracker",
          configured: true,
          schedule: null,
          last_sync: null,
        },
      ],
    });
    render(<ConnectorHealthPane />);

    const cell = within(rowFor("example-tracker")).getAllByRole("cell")[2];
    expect(cell).toHaveTextContent(/^—$/);
    expect(cell.querySelector('[data-slot="tooltip-trigger"]')).toBeNull();
    expect(within(cell).queryByRole("button")).not.toBeInTheDocument();
    await userEvent.click(rowFor("example-tracker"));
    expect(
      screen.getByText(/no sync has been recorded/i).closest("td"),
    ).toHaveAttribute("colspan", "7");
  });

  it("keeps the served order rather than sorting again", () => {
    // Deliberately an order no local rule reproduces: not alphabetical, not
    // severity-first, not by job id. A two-row fixture whose served order also
    // happens to be the severity order proves nothing.
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "zulu",
          configured: true,
          last_sync: { job_id: "1", status: "succeeded", started_at: NOW },
        },
        {
          connector: "alpha",
          configured: true,
          last_sync: { job_id: "9", status: "failed", started_at: NOW },
        },
        {
          connector: "mike",
          configured: true,
          last_sync: { job_id: "5", status: "running", started_at: NOW },
        },
      ],
    });
    render(<ConnectorHealthPane />);

    const names = screen
      .getAllByRole("row", { expanded: false })
      .map((row) => row.querySelector("td")?.textContent);
    expect(names).toEqual(["zulu", "alpha", "mike"]);
  });

  it("keeps the same row expanded when the served order changes", async () => {
    // The page polls, so a row's position is not its identity. Keying expansion
    // on the position opens a different connector the moment the order moves.
    const alpha = {
      connector: "alpha",
      configured: true,
      last_sync: { job_id: "1", status: "succeeded" as const, started_at: NOW },
    };
    const bravo = {
      connector: "bravo",
      configured: true,
      last_sync: { job_id: "2", status: "failed" as const, started_at: NOW },
    };
    mocks.summary.data = summary({ connectors: [alpha, bravo] });
    const { rerender } = render(<ConnectorHealthPane />);

    await userEvent.click(rowFor("alpha"));
    expect(rowFor("alpha")).toHaveAttribute("aria-expanded", "true");

    mocks.summary.data = summary({ connectors: [bravo, alpha] });
    rerender(<ConnectorHealthPane />);

    expect(rowFor("alpha")).toHaveAttribute("aria-expanded", "true");
    expect(rowFor("bravo")).toHaveAttribute("aria-expanded", "false");
  });

  it("gives two installations of one connector a row each", () => {
    // They share a name, so the identity cell is the only thing that tells the
    // rows apart — and a page that collapsed them would report one
    // installation's failure as the other's health.
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "claude-team",
          tenant_id: "acme",
          source_id: "claude-team-main",
          configured: true,
          last_sync: null,
        },
        {
          connector: "claude-team",
          tenant_id: "acme",
          source_id: "claude-team-second",
          configured: true,
          last_sync: null,
        },
      ],
    });
    render(<ConnectorHealthPane />);

    expect(screen.getAllByRole("cell", { name: "claude-team" })).toHaveLength(2);
    expect(
      screen.getByRole("cell", { name: "acme / claude-team-main" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("cell", { name: "acme / claude-team-second" }),
    ).toBeInTheDocument();
  });

  it("opens one installation without opening its sibling", async () => {
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "claude-team",
          tenant_id: "acme",
          source_id: "claude-team-main",
          configured: true,
          last_sync: null,
        },
        {
          connector: "claude-team",
          tenant_id: "acme",
          source_id: "claude-team-second",
          configured: true,
          last_sync: null,
        },
      ],
    });
    render(<ConnectorHealthPane />);

    const rows = screen.getAllByRole("row", { expanded: false });
    await userEvent.click(rows[1]);

    const expanded = screen.getAllByRole("row", { expanded: true });
    expect(expanded).toHaveLength(1);
    expect(
      within(expanded[0]).getByRole("cell", { name: "acme / claude-team-second" }),
    ).toBeInTheDocument();
  });

  it("points two rows at two disclosures when an identity carries a separator", () => {
    // The panel id is the row's identity with the separator swapped for one an
    // id may carry. Values the ledger recorded are not parsed on the way in, so
    // two identities can differ only in where that separator falls — and both
    // rows would then name one panel through `aria-controls`.
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "alpha",
          tenant_id: "acme_one",
          source_id: "main",
          configured: true,
          last_sync: null,
        },
        {
          connector: "alpha",
          tenant_id: "acme",
          source_id: "one_main",
          configured: true,
          last_sync: null,
        },
      ],
    });
    render(<ConnectorHealthPane />);

    const controls = screen
      .getAllByRole("row", { expanded: false })
      .map((row) => row.getAttribute("aria-controls"));

    expect(controls[0]).not.toBe(controls[1]);
  });

  it("prints an unmeasured number as absence, not as a zero", () => {
    // Named and scheduled, so the three absences below are the three numbers this
    // test is about rather than the identity or schedule cell joining them.
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "alpha",
          tenant_id: "acme",
          source_id: "main",
          configured: true,
          schedule: {
            suspended: false,
            crons: ["0 * * * *"],
            timezone: "UTC",
            origin: "default",
          },
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

    const row = rowFor("alpha");
    expect(within(row).getAllByText("—")).toHaveLength(3);
    expect(within(row).queryByText("0")).not.toBeInTheDocument();
  });

  it("says how long a sync in flight has been going, not that it took no time", () => {
    // The mover reports an unfinished job's duration as a running total, and
    // ledger rows recorded before that stopped being stored still carry its
    // zero. A row printing that zero hides the one state this page exists to
    // surface: a sync that started and never came back.
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "alpha",
          configured: true,
          last_sync: {
            job_id: "1",
            status: "running",
            started_at: "2026-01-15T10:46:00.000Z",
            duration_ms: 0,
            records_reported: null,
          },
        },
      ],
    });
    render(<ConnectorHealthPane />);

    const row = rowFor("alpha");
    expect(within(row).getByText("1h 14m so far")).toBeInTheDocument();
    expect(within(row).queryByText("0 ms")).not.toBeInTheDocument();
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

    expect(within(rowFor("alpha")).getByText("0")).toBeInTheDocument();
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

  it("does not say nothing has been read when something has", () => {
    // An empty table and a never-read ledger are different states. Keying the
    // copy on the row count makes the body contradict the header one line up.
    mocks.summary.data = summary({ connectors: [], history_available: true });
    render(<ConnectorHealthPane />);

    expect(screen.getByText(/last checked/i)).toBeInTheDocument();
    expect(
      screen.queryByText(/nothing has been read/i),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/reported no connector/i)).toBeInTheDocument();
  });
});

describe("the pane dates itself", () => {
  it("reports when the mover was last read", () => {
    render(<ConnectorHealthPane />);
    expect(screen.getByText(/last checked 1 min ago/i)).toBeInTheDocument();
  });

  it("announces a stopped recorder together with its consequence", () => {
    mocks.summary.data = summary({ checked_at: "2026-01-15T04:00:00.000Z" });
    render(<ConnectorHealthPane />);

    // One permanently mounted region, so the announcement does not depend on a
    // role appearing on a node that already existed — and it holds the
    // consequence as well as the headline, which is the half a reader needs.
    const region = screen.getByRole("status");
    expect(region).toHaveTextContent(/recording appears to have stopped/i);
    expect(region).toHaveTextContent(/may no longer be current/i);
  });

  it("says it cannot date the page when the two clocks disagree", () => {
    mocks.summary.data = summary({ checked_at: "2026-01-16T00:00:00.000Z" });
    render(<ConnectorHealthPane />);

    expect(screen.getByRole("status")).toHaveTextContent(/cannot tell when/i);
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

    await userEvent.click(rowFor("alpha"));

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

    await userEvent.click(rowFor("alpha"));

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

    expect(rowFor("alpha")).toHaveAttribute("aria-expanded", "false");

    await userEvent.click(rowFor("alpha"));

    expect(rowFor("alpha")).toHaveAttribute("aria-expanded", "true");
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

    await userEvent.click(rowFor("alpha"));

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

    // One header row plus one body row: the gesture is on the `<tr>` and the
    // role is not overridden, so the row stays in its table for a screen
    // reader and still announces itself as a disclosure.
    expect(screen.getAllByRole("row")).toHaveLength(2);
    expect(rowFor("alpha")).toHaveAttribute("aria-expanded");
  });

  it("opens the connector from anywhere in the row, not just its name", async () => {
    // The complaint this answers: only the name was clickable, so the rest of
    // a wide row looked interactive and did nothing.
    mocks.summary.data = summary({
      connectors: [
        {
          connector: "alpha",
          configured: true,
          last_sync: { job_id: "1", status: "succeeded", started_at: NOW },
        },
      ],
    });
    render(<ConnectorHealthPane />);

    await userEvent.click(screen.getByText("sync ok"));

    expect(rowFor("alpha")).toHaveAttribute("aria-expanded", "true");
  });

  it("opens the connector from the keyboard", async () => {
    mocks.summary.data = summary({
      connectors: [
        { connector: "alpha", configured: true, last_sync: null },
      ],
    });
    render(<ConnectorHealthPane />);

    rowFor("alpha").focus();
    await userEvent.keyboard("{Enter}");

    expect(rowFor("alpha")).toHaveAttribute("aria-expanded", "true");

    await userEvent.keyboard(" ");

    expect(rowFor("alpha")).toHaveAttribute("aria-expanded", "false");
  });

  it("does not open the connector when a selection was dragged across it", async () => {
    // Copying a connector name out of the table must not open it. The rule is
    // shared with every other console listing, which is the point.
    mocks.summary.data = summary({
      connectors: [
        { connector: "alpha", configured: true, last_sync: null },
      ],
    });
    render(<ConnectorHealthPane />);

    const selection = window.getSelection();
    selection?.selectAllChildren(screen.getByRole("cell", { name: "alpha" }));

    // Dispatched rather than driven through userEvent: a real pointer press
    // collapses the selection before the handler sees it, so the one state
    // this guard exists for cannot be reached that way.
    fireEvent.click(rowFor("alpha"));

    expect(rowFor("alpha")).toHaveAttribute("aria-expanded", "false");
  });
});
