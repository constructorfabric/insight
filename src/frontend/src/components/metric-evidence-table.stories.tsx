import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, within } from "storybook/test";

import { MetricEvidenceTable } from "@/components/metric-evidence-table";

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
