/**
 * Browser component tests for the header of `<MetricActivity>` — the metric's
 * name on one side, its total and the two comparisons on the other.
 *
 * The rest of the section (the event list, the day strip) is covered by the
 * jsdom tests in `metric-activity.test.tsx`; what needs a real browser is the
 * header's geometry, which no text query can see. The fixture declares no
 * granularity on purpose: that leaves the detail query disabled, so the
 * section renders its header and nothing that fetches.
 *
 * See docs/testing/storybook-component-tests.md.
 */

import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect } from "storybook/test";

import type { MetricResult } from "@/api/metric-results-client";
import { MetricActivity } from "@/components/widgets/metric-views/metric-activity";
import { normalizeMetricResults } from "@/lib/metrics/collection";
import { CARD_PX_AT_390, CARD_PX_WIDE } from "@/test/storybook/layout";

const ENTITY_ID = "019e27bc-dec0-7626-81a9-c5524662a6a9";
const METRIC_KEY = "git.prs_merged";

/**
 * A long name against a long pair of comparisons — the shape that has no room
 * for both on one line, and the only shape this test is about.
 */
function result(value: number, withPeer: boolean): MetricResult {
  return {
    metric_key: METRIC_KEY,
    label: "Pull requests merged",
    description: "Authored pull requests merged",
    unit: "PRs",
    format: "integer",
    computation: "sum",
    direction: "higher_is_better",
    views: [
      { view: "period", values: [{ entity_id: ENTITY_ID, value }] },
      ...(withPeer
        ? [
            {
              view: "peer",
              values: [
                {
                  entity_id: ENTITY_ID,
                  target_value: value,
                  p25: 1,
                  median: 3,
                  p75: 7,
                },
              ],
            },
          ]
        : []),
    ],
    selection: {
      metric_key: METRIC_KEY,
      entity: { type: "person", ids: [ENTITY_ID] },
      period: { from: "2026-08-24", to: "2026-08-30" },
      filters: [],
    },
  } as unknown as MetricResult;
}

function normalized(value: number, withPeer: boolean) {
  return normalizeMetricResults([result(value, withPeer)]).get(METRIC_KEY)!;
}

const meta: Meta<typeof MetricActivity> = {
  title: "Widgets/MetricViews/MetricActivity",
  component: MetricActivity,
  args: {
    metric: normalized(11, true),
    previous: normalized(14, false),
    entityId: ENTITY_ID,
    periodNoun: "week",
  },
};
export default meta;

type Story = StoryObj<typeof MetricActivity>;

/** Demo story for the Storybook UI (untagged — not a test). */
export const Default: Story = {
  decorators: [
    (Story) => (
      <div style={{ width: CARD_PX_WIDE }}>
        <Story />
      </div>
    ),
  ],
};

/**
 * Where a line fits both, the name and the figures share it.
 *
 * The narrow case below only proves something if the wide case is still one
 * line — a header that always stacks would pass it and be a different bug.
 */
export const TestWideHeaderStaysOnOneLine: Story = {
  tags: ["test"],
  decorators: [
    (Story) => (
      <div style={{ width: CARD_PX_WIDE }}>
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    const name = canvas.getByText("Pull requests merged");
    const total = canvas.getByText("11 PRs");

    // They share a line — asserted as overlapping vertical bands rather than
    // equal tops, which an inline box and a block box never have.
    const nameBox = name.getBoundingClientRect();
    const totalBox = total.getBoundingClientRect();

    await expect(totalBox.top).toBeLessThan(nameBox.bottom);
    await expect(nameBox.top).toBeLessThan(totalBox.bottom);
  },
};

/**
 * Too narrow for both, the header stacks and the name keeps the whole line.
 *
 * Unfixed, the name column collapsed and the name wrapped into a stack of
 * single words beside the figures — unreadable, while every `getByText` still
 * passed. Only geometry can see it.
 */
export const TestNarrowHeaderStacksInsteadOfOverlapping: Story = {
  tags: ["test"],
  decorators: [
    (Story) => (
      <div style={{ width: CARD_PX_AT_390 }}>
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    // The column, not the text in it: the name renders as an inline span, which
    // keeps its own width by overflowing a column collapsed to nothing.
    const name = canvas.getByText("Pull requests merged");
    const column = name.closest("div")!;
    const columnBox = column.getBoundingClientRect();
    const rowBox = column.parentElement!.getBoundingClientRect();
    const totalBox = canvas.getByText("11 PRs").getBoundingClientRect();

    await expect(
      totalBox.top,
      `the figures share the name's line (name ends ${columnBox.bottom}, figures start ${totalBox.top})`
    ).toBeGreaterThanOrEqual(columnBox.bottom);

    // Stacking is only worth the line it costs if the name got that line: a
    // column narrow enough to break the name into single words satisfies the
    // assertion above just as well.
    await expect(
      columnBox.width,
      `the name column is ${columnBox.width}px of a ${rowBox.width}px row`
    ).toBeGreaterThan(rowBox.width / 2);
  },
};
