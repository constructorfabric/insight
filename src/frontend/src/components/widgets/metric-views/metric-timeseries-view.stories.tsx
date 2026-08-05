/**
 * Stories + browser component tests for `<MetricTimeseriesView>` — the card
 * that switches one metric collection between a chart and a table.
 *
 * The view fetches its own data, so every story drives it through the MSW
 * handler in `metric-timeseries.story-fixtures.ts` (see that file for the fact
 * cube). Covered states: chart, table, persisted presentation, metric select,
 * combined metrics, group-by + dimension filters, revalidation, loading,
 * error + retry, empty, and both export formats.
 *
 * Exports are asserted end to end: `captureDownloads()` intercepts the
 * object-URL + anchor click that `downloadBlob` performs, so the play function
 * reads the real CSV text and parses the real workbook instead of a mock call.
 *
 * See docs/testing/storybook-component-tests.md.
 */

import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen, userEvent, waitFor } from "storybook/test";

import { MetricTimeseriesView } from "@/components/widgets/metric-views/metric-timeseries-view";
import {
  COMMITS,
  ENTITY_ID,
  LINES_ADDED,
  metricResultsHandler,
  RANGE,
} from "@/components/widgets/metric-views/metric-timeseries.story-fixtures";

const TOP_REPOSITORIES = {
  default: "repository",
  limits: {
    repository: {
      count: 2,
      rankBy: COMMITS,
      includeRemainder: true,
    },
  },
};

const REPOSITORY_AND_SOURCE = { default: "repository", options: ["source"] };

interface CapturedDownload {
  filename: string;
  blob: Blob;
}

const downloads: CapturedDownload[] = [];

/** Failure budget for the retry story; reset in that story's `beforeEach`. */
const retryState = { failures: 1 };

/**
 * Capture what `downloadBlob` hands to the browser. A real anchor click on a
 * `download` link would leave the headless browser writing files; intercepting
 * it keeps the assertion on the produced bytes.
 */
function captureDownloads(): () => void {
  downloads.length = 0;
  const blobs = new Map<string, Blob>();
  const createObjectURL = URL.createObjectURL;
  const originalClick = HTMLAnchorElement.prototype.click;

  URL.createObjectURL = (object: Blob | MediaSource) => {
    const url = createObjectURL.call(URL, object);
    if (object instanceof Blob) blobs.set(url, object);
    return url;
  };
  HTMLAnchorElement.prototype.click = function (this: HTMLAnchorElement) {
    const blob = this.download ? blobs.get(this.href) : undefined;
    if (!blob) {
      originalClick.call(this);
      return;
    }
    downloads.push({ filename: this.download, blob });
  };

  return () => {
    URL.createObjectURL = createObjectURL;
    HTMLAnchorElement.prototype.click = originalClick;
  };
}

async function openExportMenu(): Promise<void> {
  await userEvent.click(screen.getByRole("button", { name: "Export" }));
}

const meta: Meta<typeof MetricTimeseriesView> = {
  title: "Widgets/MetricViews/MetricTimeseriesView",
  component: MetricTimeseriesView,
  args: {
    id: "git-output",
    entityId: ENTITY_ID,
    range: RANGE,
    metricKeys: [COMMITS, LINES_ADDED],
  },
  parameters: { msw: { handlers: [metricResultsHandler()] } },
  // The card is a dashboard grid cell with no intrinsic width; the chart's
  // ResponsiveContainer would measure 0 in an isolated centered story.
  decorators: [
    (Story) => (
      <div className="w-[900px]">
        <Story />
      </div>
    ),
  ],
};
export default meta;

type Story = StoryObj<typeof MetricTimeseriesView>;

/** Demo story for the Storybook UI (untagged — not a test). */
export const Chart: Story = {
  args: { groupBy: TOP_REPOSITORIES },
};

export const Table: Story = {
  args: { groupBy: TOP_REPOSITORIES, defaultPresentation: "table" },
};

export const GroupedAndFilterable: Story = {
  args: { groupBy: REPOSITORY_AND_SOURCE },
};

export const Loading: Story = {
  parameters: {
    msw: { handlers: [metricResultsHandler({ kind: "pending" })] },
  },
};

export const LoadFailed: Story = {
  parameters: { msw: { handlers: [metricResultsHandler({ kind: "error" })] } },
};

export const NoData: Story = {
  parameters: { msw: { handlers: [metricResultsHandler({ kind: "empty" })] } },
};

/** Grouped chart: caption, series legend (top-2 + remainder), plotted svg. */
export const TestChartPresentation: Story = {
  tags: ["test"],
  args: { groupBy: TOP_REPOSITORIES },
  play: async ({ canvas, canvasElement }) => {
    // The caption renders while the request is still in flight — a series
    // label is the signal that the collection resolved.
    await expect(await canvas.findByText("org/repo-a")).toBeInTheDocument();
    await expect(canvas.getByText("Weekly by repository")).toBeInTheDocument();
    await expect(canvas.getByText("org/repo-b")).toBeInTheDocument();
    await expect(canvas.getByText("Other")).toBeInTheDocument();
    // org/repo-c ranks third and folds into the remainder series.
    await expect(canvas.queryByText("org/repo-c")).not.toBeInTheDocument();
    await waitFor(() =>
      expect(canvasElement.querySelector("svg.recharts-surface")).toBeTruthy()
    );
  },
};

/** Toggling to the table swaps the body and persists the choice per card. */
export const TestPresentationToggle: Story = {
  tags: ["test"],
  args: { id: "toggle", groupBy: TOP_REPOSITORIES },
  play: async ({ canvas }) => {
    await canvas.findByText("org/repo-a");

    await userEvent.click(canvas.getByRole("button", { name: "Table view" }));
    await expect(canvas.getByText("Week")).toBeInTheDocument();
    await expect(canvas.getByText("Total")).toBeInTheDocument();
    await expect(
      canvas.getByText("Grand total").closest("tr")
    ).toHaveTextContent("Commits: 10");
    await expect(
      canvas.queryByText("Weekly by repository")
    ).not.toBeInTheDocument();
    await expect(
      window.localStorage.getItem("insight.timeseries.toggle.presentation")
    ).toBe("table");

    await userEvent.click(canvas.getByRole("button", { name: "Chart view" }));
    await expect(canvas.getByText("Weekly by repository")).toBeInTheDocument();
    await expect(
      window.localStorage.getItem("insight.timeseries.toggle.presentation")
    ).toBe("chart");
  },
};

/** A previously stored presentation wins over `defaultPresentation`. */
export const TestPersistedPresentation: Story = {
  tags: ["test"],
  args: { id: "persisted", defaultPresentation: "chart" },
  beforeEach: () => {
    window.localStorage.setItem(
      "insight.timeseries.persisted.presentation",
      "table"
    );
  },
  play: async ({ canvas }) => {
    await expect(await canvas.findByText("Week")).toBeInTheDocument();
    await expect(canvas.queryByText("Weekly")).not.toBeInTheDocument();
  },
};

/** Multi-metric chart: the header select drives the plotted metric. */
export const TestMetricSelect: Story = {
  tags: ["test"],
  args: { groupBy: TOP_REPOSITORIES },
  play: async ({ canvas }) => {
    const trigger = await canvas.findByLabelText("Metric");
    await expect(trigger).toHaveTextContent("Commits");

    await userEvent.click(trigger);
    await userEvent.click(
      await screen.findByRole("option", { name: "Lines added" })
    );

    await expect(canvas.getByLabelText("Metric")).toHaveTextContent(
      "Lines added"
    );
  },
};

/** Combined multi-metric chart: one title, no metric select. */
export const TestCombinedMetrics: Story = {
  tags: ["test"],
  args: { chart: { multiMetric: "combined" } },
  play: async ({ canvas }) => {
    await expect(
      await canvas.findByText("Commits & Lines added")
    ).toBeInTheDocument();
    await expect(canvas.queryByLabelText("Metric")).not.toBeInTheDocument();
    await expect(canvas.getByText("Weekly")).toBeInTheDocument();
  },
};

/** Group-by switch and dimension filters both re-shape the served series. */
export const TestGroupByAndFilters: Story = {
  tags: ["test"],
  args: { groupBy: REPOSITORY_AND_SOURCE },
  play: async ({ canvas }) => {
    await canvas.findByText("org/repo-a");

    await userEvent.click(canvas.getByRole("button", { name: "Filters" }));
    await userEvent.click(
      await screen.findByRole("checkbox", { name: "GitLab" })
    );
    await userEvent.keyboard("{Escape}");

    await waitFor(() =>
      expect(canvas.queryByText("org/repo-a")).not.toBeInTheDocument()
    );
    await expect(canvas.getByText("org/repo-c")).toBeInTheDocument();
    await expect(
      canvas.getByRole("button", { name: "Clear Source filter" })
    ).toHaveTextContent("Source: GitLab");

    await userEvent.click(
      canvas.getByRole("button", { name: "Clear Source filter" })
    );
    await waitFor(() =>
      expect(canvas.getByText("org/repo-a")).toBeInTheDocument()
    );

    await userEvent.click(canvas.getByRole("button", { name: "Source" }));
    // The caption follows local state; only a series label proves the
    // re-grouped collection arrived.
    await expect(await canvas.findByText("GitHub")).toBeInTheDocument();
    await expect(canvas.getByText("GitLab")).toBeInTheDocument();
    await expect(canvas.getByText("Weekly by source")).toBeInTheDocument();
  },
};

/** Revalidation keeps the previous data, marks the body busy, gates export. */
export const TestRevalidating: Story = {
  tags: ["test"],
  args: { groupBy: REPOSITORY_AND_SOURCE },
  parameters: {
    msw: { handlers: [metricResultsHandler({ kind: "data", delayMs: 800 })] },
  },
  play: async ({ canvas, canvasElement }) => {
    await canvas.findByText("org/repo-a", undefined, { timeout: 5000 });

    await userEvent.click(canvas.getByRole("button", { name: "Source" }));
    await waitFor(() =>
      expect(canvasElement.querySelector('[aria-busy="true"]')).toBeTruthy()
    );
    await expect(canvas.getByRole("button", { name: "Export" })).toBeDisabled();
    // Previous series stay on screen while the new group-by is in flight.
    await expect(canvas.getByText("org/repo-a")).toBeInTheDocument();

    await waitFor(
      () => expect(canvas.getByText("GitHub")).toBeInTheDocument(),
      { timeout: 5000 }
    );
    await expect(canvas.getByRole("button", { name: "Export" })).toBeEnabled();
  },
};

/** Initial load: spinner instead of a body, export gated. */
export const TestLoading: Story = {
  tags: ["test"],
  parameters: {
    msw: { handlers: [metricResultsHandler({ kind: "pending" })] },
  },
  play: async ({ canvas }) => {
    await expect(
      canvas.getByRole("status", { name: "Loading" })
    ).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: "Export" })).toBeDisabled();
    await expect(
      canvas.queryByText("No data in this period")
    ).not.toBeInTheDocument();
  },
};

/** Failed load: error card, export gated, Retry refetches into data. */
export const TestErrorAndRetry: Story = {
  tags: ["test"],
  args: { groupBy: TOP_REPOSITORIES },
  parameters: {
    msw: {
      handlers: [
        metricResultsHandler({ kind: "recovering", state: retryState }),
      ],
    },
  },
  beforeEach: () => {
    retryState.failures = 1;
  },
  play: async ({ canvas }) => {
    await expect(
      await canvas.findByText("Unable to load timeseries")
    ).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: "Export" })).toBeDisabled();

    await userEvent.click(canvas.getByRole("button", { name: "Retry" }));

    await expect(await canvas.findByText("org/repo-a")).toBeInTheDocument();
    await expect(canvas.getByText("Weekly by repository")).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: "Export" })).toBeEnabled();
  },
};

/** A response with no metrics renders the empty body and gates export. */
export const TestEmpty: Story = {
  tags: ["test"],
  parameters: { msw: { handlers: [metricResultsHandler({ kind: "empty" })] } },
  play: async ({ canvas }) => {
    await expect(
      await canvas.findByText("No data in this period")
    ).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: "Export" })).toBeDisabled();
  },
};

/** CSV export writes the grouped, per-metric grid with no footer rows. */
export const TestCsvExport: Story = {
  tags: ["test"],
  args: { groupBy: TOP_REPOSITORIES },
  beforeEach: () => captureDownloads(),
  play: async ({ canvas }) => {
    await canvas.findByText("org/repo-a");

    await openExportMenu();
    await userEvent.click(await screen.findByText("CSV (.csv)"));

    await waitFor(() => expect(downloads).toHaveLength(1));
    const download = downloads[0]!;
    await expect(download.filename).toBe(
      "git-output_2026-04-20_2026-05-04.csv"
    );
    const text = await download.blob.text();
    await expect(text).toContain(
      "Week,org/repo-a — Commits,org/repo-a — Lines added,org/repo-b — Commits,org/repo-b — Lines added,Other — Commits,Other — Lines added"
    );
    await expect(text).toContain("2026-04-20,3,30,2,18,1,11");
    // Missing points stay empty rather than zero-filled.
    await expect(text).toContain("2026-04-27,0,0,,,1,6");
    await expect(text).not.toContain("Grand total");
  },
};

/** Excel export writes a merged two-level header, totals, and grand total. */
export const TestXlsxExport: Story = {
  tags: ["test"],
  args: { groupBy: TOP_REPOSITORIES },
  beforeEach: () => captureDownloads(),
  play: async ({ canvas }) => {
    await canvas.findByText("org/repo-a");

    await openExportMenu();
    await userEvent.click(await screen.findByText("Excel (.xlsx)"));

    await waitFor(() => expect(downloads).toHaveLength(1), { timeout: 5000 });
    const download = downloads[0]!;
    await expect(download.filename).toBe(
      "git-output_2026-04-20_2026-05-04.xlsx"
    );

    const { Workbook } = await import("exceljs");
    const workbook = new Workbook();
    await workbook.xlsx.load(await download.blob.arrayBuffer());
    const sheet = workbook.getWorksheet("Timeseries");

    await expect(sheet?.getCell("A1").value).toBe("Week");
    await expect(sheet?.getCell("B1").value).toBe("org/repo-a");
    await expect(sheet?.getCell("B2").value).toBe("Commits");
    await expect(sheet?.getCell("C2").value).toBe("Lines added");
    await expect(sheet?.getCell("F1").value).toBe("Other");
    await expect(sheet?.getCell("A3").value).toBe("2026-04-20");
    await expect(sheet?.getCell("B6").value).toBe(5);
    await expect(sheet?.getCell("A6").value).toBe("Total");
    await expect(sheet?.getCell("A7").value).toBe("Grand total");
    await expect(sheet?.getCell("B7").value).toBe(
      "Commits: 10 · Lines added: 98"
    );
    await expect(sheet?.views[0]).toMatchObject({ xSplit: 1, ySplit: 2 });
  },
};
