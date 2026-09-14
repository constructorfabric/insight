/**
 * Browser component tests for the geometry of `<SectionMetricIndex>` — the
 * "Also measured here" list, one metric per row: name, value, the pool's
 * middle, and the mark on the shared axis.
 *
 * What order the rows come in and which of them are dropped is covered by the
 * jsdom tests in `section-metric-index.test.tsx`. What needs a real browser is
 * the row: the readings hold a fixed width, so a name sharing their line has
 * whatever is left — which on a phone is nothing, and a name truncated to
 * nothing leaves a column of numbers belonging to no metric.
 *
 * See docs/testing/storybook-component-tests.md.
 */

import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect } from "storybook/test";

import type { MetricResult } from "@/api/metric-results-client";
import { SectionMetricIndex } from "@/components/widgets/metric-views/section-metric-index";
import { normalizeMetricResults } from "@/lib/metrics/collection";
import {
  CARD_PX_AT_390,
  CARD_PX_WIDE,
  expectHorizontallyContained,
} from "@/test/storybook/layout";

const ENTITY_ID = "019e27bc-dec0-7626-81a9-c5524662a6a9";

/** The row's name, long enough that a collapsed column is unmistakable. */
const LONG_NAME = "Merges without approval rate";

function metric(
  key: string,
  label: string,
  value: number,
  unit: string | null,
  median: number
): MetricResult {
  return {
    metric_key: key,
    label,
    unit,
    format: "integer",
    computation: "sum",
    direction: "higher_is_better",
    views: [
      { view: "period", values: [{ entity_id: ENTITY_ID, value }] },
      {
        view: "peer",
        values: [
          {
            entity_id: ENTITY_ID,
            target_value: value,
            p25: 1,
            median,
            p75: 100000,
          },
        ],
      },
    ],
  } as unknown as MetricResult;
}

const RESULTS = [
  metric("git.long", LONG_NAME, 325, null, 731),
  metric("git.short", "Merge rate", 12408, "lines", 9),
];

const meta: Meta<typeof SectionMetricIndex> = {
  title: "Widgets/MetricViews/SectionMetricIndex",
  component: SectionMetricIndex,
  args: {
    collection: { metrics: RESULTS.map((r) => ({ key: r.metric_key, views: [] })) },
    byKey: normalizeMetricResults(RESULTS),
    entityId: ENTITY_ID,
    shown: new Set<string>(),
  },
};
export default meta;

type Story = StoryObj<typeof SectionMetricIndex>;

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

/** Where a line fits both, the name and its readings share it. */
export const TestWideRowStaysOnOneLine: Story = {
  tags: ["test"],
  decorators: [
    (Story) => (
      <div style={{ width: CARD_PX_WIDE }}>
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas, canvasElement }) => {
    const name = canvas.getByText(LONG_NAME).getBoundingClientRect();
    const value = canvas.getByText("325").getBoundingClientRect();

    await expect(value.top, "the readings dropped to their own line").toBeLessThan(
      name.bottom
    );
    await expect(name.top).toBeLessThan(value.bottom);

    // The container query is what puts the readings back into fixed columns;
    // without it the rows still render, just unaligned, so nothing else here
    // would notice it failing to compile.
    const readings = canvasElement.querySelectorAll("dd");
    // The value and the median; the third child is the peer mark's own fixed
    // 112px svg, which no breakpoint touches.
    const columns = Array.from(readings).map((dd) =>
      Array.from(dd.children)
        .slice(0, 2)
        .map((span) => Math.round(span.getBoundingClientRect().width))
    );
    await expect(
      columns,
      `the value and median columns are not the fixed 128/112 (${JSON.stringify(columns)})`
    ).toEqual([
      [128, 112],
      [128, 112],
    ]);
  },
};

/**
 * Too narrow for both, the row stacks and the name keeps its whole width.
 *
 * The failure this pins reads as data with no labels: the readings held their
 * fixed columns, the name column collapsed to zero, and `truncate` cut the
 * name away entirely — leaving numbers belonging to no metric.
 */
export const TestNarrowRowKeepsTheMetricName: Story = {
  tags: ["test"],
  decorators: [
    (Story) => (
      <div style={{ width: CARD_PX_AT_390 }}>
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas, canvasElement }) => {
    // The column, not the text inside it: the name renders as an inline span,
    // which keeps its full width by overflowing a column collapsed to zero.
    const column = canvas.getByText(LONG_NAME).closest("dt")!;
    const value = canvas.getByText("325");
    const columnBox = column.getBoundingClientRect();

    const list = canvasElement.querySelector("dl");
    await expect(list, "the index renders its list").not.toBeNull();
    const row = column.parentElement!.getBoundingClientRect();

    // The readings having wrapped away, the name owns the row's whole width.
    await expect(
      columnBox.width,
      `the name column is ${columnBox.width}px of a ${row.width}px row`
    ).toBeGreaterThan(row.width / 2);
    await expect(
      value.getBoundingClientRect().top,
      "the readings share the name's line"
    ).toBeGreaterThanOrEqual(columnBox.bottom);

    // The readings are 368px of fixed columns at full width; hanging outside
    // the list drops the peer marks off the card.
    expectHorizontallyContained(
      list!,
      Array.from(list!.querySelectorAll("dd")),
      () => "a row's readings"
    );
  },
};
