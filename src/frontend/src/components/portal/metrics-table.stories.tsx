import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn } from "storybook/test";

import { MetricsTable } from "@/components/portal/metrics-table";
import { normalizeMetricResults } from "@/lib/metrics/collection";

const PERSON = "019e27bc-dec0-7626-81a9-c5524662a6a9";

const BY_KEY = normalizeMetricResults(
  ["git.commits", "git.prs_merged"].map((metricKey, index) => ({
    metric_key: metricKey,
    label: index === 0 ? "Commits" : "PRs merged",
    unit: null,
    format: "integer" as const,
    computation: "sum" as const,
    direction: "higher_is_better" as const,
    views: [{ view: "period" as const, values: [{ entity_id: PERSON, value: 12 + index }] }],
  })),
);

const meta: Meta<typeof MetricsTable> = {
  title: "Portal/MetricsTable",
  component: MetricsTable,
  args: {
    rows: [
      { entityId: PERSON, displayName: "Alexandra Fernandez", depth: 0, hasReports: false, expanded: false },
    ],
    groups: [{ family: "git", name: "Development · Git", keys: ["git.commits", "git.prs_merged"] }],
    byKey: BY_KEY,
    helpByKey: new Map(),
    roles: new Map(),
    sort: null,
    onSort: fn(),
    onToggle: fn(),
    onScope: fn(),
  },
  decorators: [
    (Story) => (
      <div data-testid="frame" style={{ width: 1200 }}>
        <Story />
      </div>
    ),
  ],
};

export default meta;

type Story = StoryObj<typeof MetricsTable>;

export const TestAFewMetricsFillTheWidth: Story = {
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const frame = canvasElement.querySelector('[data-testid="frame"]')!;
    const table = canvasElement.querySelector("table")!;

    await expect(
      Math.round(table.getBoundingClientRect().width),
      "a table of two metrics leaves the rest of the page empty",
    ).toBeGreaterThanOrEqual(Math.round(frame.getBoundingClientRect().width) - 2);
  },
};
