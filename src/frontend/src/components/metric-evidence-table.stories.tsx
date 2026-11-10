import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, waitFor, within } from "storybook/test";

import { MetricEvidenceTable } from "@/components/metric-evidence-table";
import { expectNothingOverlaps } from "@/test/storybook/layout";

const meta: Meta<typeof MetricEvidenceTable> = {
  title: "Metrics/MetricEvidenceTable",
  component: MetricEvidenceTable,
  args: {
    metricKey: null,
    columns: [
      { key: "ref", label: "Ref", type: "string", sortable: true },
      { key: "title", label: "Title", type: "string", sortable: true },
    ],
    rows: [{ values: { ref: "15830", title: "Handle an empty period" } }],
    sort: null,
    onSortChange: fn(),
    fetchNextPage: fn(async () => undefined),
    hasNextPage: false,
    isFetchingNextPage: false,
    nextPageError: false,
    pageLimitReached: false,
  },
  decorators: [
    (Story) => (
      <div style={{ width: 720, height: 320 }}>
        <Story />
      </div>
    ),
  ],
};

export default meta;

type Story = StoryObj<typeof MetricEvidenceTable>;

function verticalCentre(element: Element): number {
  const box = element.getBoundingClientRect();
  return box.top + box.height / 2;
}

export const TestRowContentSharesOneLine: Story = {
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const expander = canvas.getByRole("button", { name: "Show full record" });
    const ref = canvas.getByText("15830");
    const title = canvas.getByText("Handle an empty period");

    await expect(
      Math.abs(verticalCentre(ref) - verticalCentre(expander)),
      "the ref sits off the expander's line",
    ).toBeLessThanOrEqual(1);
    await expect(
      Math.abs(verticalCentre(title) - verticalCentre(expander)),
      "the title sits off the expander's line",
    ).toBeLessThanOrEqual(1);
  },
};

export const TestNumbersAlignRight: Story = {
  tags: ["test"],
  args: {
    columns: [
      { key: "ref", label: "Ref", type: "string", sortable: true },
      { key: "lines", label: "Lines", type: "number", sortable: true },
    ],
    rows: [{ values: { ref: "15830", lines: 42 } }],
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const figure = canvas.getByText("42");
    const cell = figure.closest("td")!;
    const padding = Number.parseFloat(getComputedStyle(cell).paddingRight);

    await expect(
      Math.abs(cell.getBoundingClientRect().right - padding - figure.getBoundingClientRect().right),
      "a number sits off its cell's right edge",
    ).toBeLessThanOrEqual(1);
    await expect(
      Math.abs(verticalCentre(figure) - verticalCentre(cell)),
      "a number sits off its row's line",
    ).toBeLessThanOrEqual(1);
  },
};

export const TestFullRecordPushesTheNextRowDown: Story = {
  tags: ["test"],
  args: {
    columns: [
      { key: "merged", label: "Merged", type: "number", sortable: true },
      { key: "created", label: "Created", type: "number", sortable: true },
      { key: "date", label: "Date", type: "date", sortable: true },
    ],
    rows: [
      { values: { merged: 3, created: 4, date: "2026-01-12" } },
      { values: { merged: 1, created: 2, date: "2026-01-11" } },
    ],
    numbered: true,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const [first] = canvas.getAllByRole("button", { name: "Show full record" });
    await userEvent.click(first!);

    const record = await waitFor(() => {
      const list = canvasElement.querySelector("tbody dl");
      if (!list) throw new Error("the full record never opened");
      return list;
    });
    const next = canvasElement.querySelector('tbody tr[aria-rowindex="3"]');

    await expect(next, "the row after the opened one renders").toBeTruthy();
    await waitFor(() =>
      expectNothingOverlaps(record, [next!], () => "the next row"),
    );
  },
};
