import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { MetricDrilldownResponse } from "@/api/metric-drilldown-client";
import { MetricEvidenceDialogProvider } from "@/components/metric-evidence-dialog-provider";
import { MetricValue } from "@/components/portal/metrics-table";
import type { NormalizedMetricResult } from "@/lib/metrics/collection";
import { buildMetricDefinitions } from "@/mocks/metric-definitions-factory";
import { metaFor } from "@/mocks/metric-results-factory";
import { AppTooltips } from "@/test/tooltip";

const mocks = vi.hoisted(() => ({
  queryMetricDrilldown: vi.fn(),
  listMetricDefinitions: vi.fn(),
}));

vi.mock("@/api/metric-drilldown-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/metric-drilldown-client")>()),
  queryMetricDrilldown: mocks.queryMetricDrilldown,
}));

vi.mock("@/api/metric-definitions-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/metric-definitions-client")>()),
  listMetricDefinitions: mocks.listMetricDefinitions,
}));

vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({
    session: {
      tenantId: "tenant",
      personId: "person",
      impersonatorEmail: null,
      roles: ["viewer"],
    },
  }),
}));

vi.mock("@/telemetry", () => ({ recordUsageEvent: vi.fn() }));

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        start: index * 44,
      })),
    getTotalSize: () => count * 44,
    measureElement: vi.fn(),
  }),
}));

const PERSON = "person-1";
const PERIOD = { from: "2026-07-01", to: "2026-07-31" };

interface TimeMetric {
  key: string;
  served: number[];
  figure: number;
  figureText: string;
  shown: string[];
}

const meetingHours: TimeMetric = {
  key: "collab.meeting_hours",
  served: [1.5, 0.25, 2.75, 0, 185 / 60, 40 / 60],
  figure: 8.25,
  figureText: "8 h 15 min",
  shown: ["1 h 30 min", "15 min", "2 h 45 min", "0 min", "3 h 5 min", "40 min"],
};

const prCycleTime: TimeMetric = {
  key: "git.pr_cycle_time_h",
  served: [2.5, 26.75, 49.03333333333333, 0.25, 130.5],
  figure: 26.75,
  figureText: "26 h 45 min",
  shown: ["2 h 30 min", "26 h 45 min", "49 h 2 min", "15 min", "131 h"],
};

const taskPickupTime: TimeMetric = {
  key: "tasks.pickup_time",
  served: [0.5, 2.25, 7.125],
  figure: 2.25,
  figureText: "2.3 d",
  shown: ["0.5 d", "2.3 d", "7.1 d"],
};

const TIME_METRICS = [meetingHours, prCycleTime, taskPickupTime];
const MEDIAN_METRICS = [prCycleTime, taskPickupTime];

function label(metric: TimeMetric): string {
  return metaFor(metric.key).label;
}

function result(metric: TimeMetric): NormalizedMetricResult {
  return {
    ...metaFor(metric.key),
    period: {
      view: "period",
      values: [{ entity_id: PERSON, value: metric.figure }],
    },
    drilldown: { granularity: ["event"] },
    selection: {
      metric_key: metric.key,
      entity: { type: "person", ids: [PERSON] },
      period: PERIOD,
      filters: [],
    },
  } as NormalizedMetricResult;
}

function page(metric: TimeMetric): MetricDrilldownResponse {
  return {
    selection: {
      metric_key: metric.key,
      entity: { type: "person", id: PERSON },
      period: PERIOD,
      filters: [],
      display_dimensions: [],
      sort: { key: "date", direction: "desc" },
    },
    columns: [
      { key: "date", label: "Date", type: "date", sortable: true },
      { key: "value", label: "Value", type: "number", sortable: true },
    ],
    rows: metric.served.map((value) => ({
      values: { date: "2026-07-06", value },
    })),
    next_cursor: null,
  };
}

function Providers({ children }: { children: ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return (
    <QueryClientProvider client={client}>
      <AppTooltips>
        <MetricEvidenceDialogProvider>{children}</MetricEvidenceDialogProvider>
      </AppTooltips>
    </QueryClientProvider>
  );
}

async function drill(metric: TimeMetric) {
  mocks.queryMetricDrilldown.mockResolvedValue(page(metric));
  const user = userEvent.setup();
  render(
    <MetricValue
      result={result(metric)}
      entityId={PERSON}
      personName="Ada Lovelace"
    />,
    { wrapper: Providers }
  );
  const cell = screen.getByRole("button", {
    name: (name) => name.includes(label(metric)),
  });
  const cellText = cell.textContent ?? "";
  await user.click(cell);
  const dialog = await screen.findByRole("dialog");
  await within(dialog).findByRole("table");
  return { cellText, dialog };
}

function shownValues(dialog: HTMLElement, metric: TimeMetric): string[] {
  const [header, ...body] = within(dialog).getAllByRole("row");
  const column = within(header!)
    .getAllByRole("columnheader")
    .findIndex((cell) => cell.textContent === label(metric));
  return body.map(
    (row) => within(row).getAllByRole("cell")[column]!.textContent ?? ""
  );
}

function minutesOf(text: string): number {
  const match = /^(?:(\d+) h)?\s*(?:(\d+) min)?$/.exec(text);
  if (!match || (match[1] == null && match[2] == null)) {
    throw new Error(`"${text}" is not hours and minutes`);
  }
  return Number(match[1] ?? 0) * 60 + Number(match[2] ?? 0);
}

describe("MetricValue drill-down of a time metric", () => {
  beforeEach(() => {
    mocks.queryMetricDrilldown.mockReset();
    mocks.listMetricDefinitions
      .mockReset()
      .mockResolvedValue({ metrics: buildMetricDefinitions() });
  });

  it.each(TIME_METRICS)(
    "shows every $key row the way the metric reads",
    async (metric) => {
      const { dialog } = await drill(metric);

      expect(shownValues(dialog, metric)).toEqual(metric.shown);
    }
  );

  it.each(TIME_METRICS)(
    "names the $key figure in the heading",
    async (metric) => {
      const { dialog } = await drill(metric);

      expect(within(dialog).getByRole("heading")).toHaveTextContent(
        metric.figureText
      );
    }
  );

  it("adds the meeting rows, as shown, up to the cell that opened them", async () => {
    const { cellText, dialog } = await drill(meetingHours);

    const rowMinutes = shownValues(dialog, meetingHours).map(minutesOf);

    expect(cellText).toBe("8 h 15 min");
    expect(rowMinutes.reduce((sum, minutes) => sum + minutes, 0)).toBe(
      minutesOf(cellText)
    );
  });

  it.each(MEDIAN_METRICS)(
    "reads the median $key row as the figure",
    async (metric) => {
      const { dialog } = await drill(metric);

      const ordered = [...metric.served].sort((a, b) => a - b);
      const median = ordered[(ordered.length - 1) / 2]!;

      expect(shownValues(dialog, metric)[metric.served.indexOf(median)]).toBe(
        metric.figureText
      );
    }
  );
});
