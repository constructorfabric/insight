/**
 * Browser component tests for the geometry of `<MembersGrid>` — the roster,
 * one row per member and one column per metric.
 *
 * Which rows it draws, how it sorts them and what each cell says is covered by
 * the jsdom tests in `members-grid.test.tsx`. What needs a real browser is the
 * width: the member column is fixed and the table scrolls sideways past it, so
 * on a phone a column sized for a desktop leaves no room for a single metric.
 *
 * See docs/testing/storybook-component-tests.md.
 */

import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect } from "storybook/test";

import type { MetricResult } from "@/api/metric-results-client";
import { MembersGrid } from "@/components/widgets/dashboard/members-grid";
import { normalizeMetricResults } from "@/lib/metrics/collection";
import {
  CARD_PX_AT_390,
  CARD_PX_WIDE,
  expectHorizontallyContained,
} from "@/test/storybook/layout";

/** Long enough that a member column crushed to nothing is unmistakable. */
const MEMBERS = [
  { entityId: "019e27bc-dec0-7626-81a9-c5524662a6a9", displayName: "Alexandra Fernandez" },
  { entityId: "019e27bc-dec0-7626-81a9-c5524662a6b0", displayName: "Bo Ng" },
];

function metric(key: string, shortLabel: string): MetricResult {
  return {
    metric_key: key,
    label: `${shortLabel} over the period`,
    short_label: shortLabel,
    unit: null,
    format: "integer",
    computation: "sum",
    direction: "higher_is_better",
    views: [
      {
        view: "period",
        values: MEMBERS.map((m, index) => ({
          entity_id: m.entityId,
          value: 100 + index,
        })),
      },
      {
        view: "peer",
        values: MEMBERS.map((m, index) => ({
          entity_id: m.entityId,
          target_value: 100 + index,
          p25: 50,
          median: 100,
          p75: 150,
          min: 0,
          max: 200,
          n: 12,
        })),
      },
    ],
  } as unknown as MetricResult;
}

/** Enough columns that the table overruns any container these stories use. */
const RESULTS = [
  metric("git.commits", "Commits"),
  metric("git.prs_merged", "PRs merged"),
  metric("git.code_lines", "Code lines"),
  metric("collab.messages", "Msgs"),
  metric("collab.meeting_hours", "Mtg hrs"),
  metric("ai.active_days", "AI days"),
];

const meta: Meta<typeof MembersGrid> = {
  title: "Widgets/Dashboard/MembersGrid",
  component: MembersGrid,
  args: {
    members: MEMBERS,
    metricKeys: RESULTS.map((r) => r.metric_key),
    byKey: normalizeMetricResults(RESULTS),
    caption: "Members grid",
  },
};
export default meta;

type Story = StoryObj<typeof MembersGrid>;

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

/** The column the names sit in is the one that holds the scroller's width. */
function memberColumnWidth(table: Element): number {
  const header = table.querySelector("thead th");
  return Math.round(header!.getBoundingClientRect().width);
}

/**
 * On a phone the first metric column reads in full beside the names.
 *
 * The failure this pins is invisible to a text query: every cell is in the DOM
 * and correct, and the member column simply holds a desktop width the viewport
 * does not have — so the roster shows names, part of one metric, and nothing
 * else, with the rest hidden behind a sideways scroll nobody looks for.
 */
export const TestNarrowRosterShowsAWholeMetricColumn: Story = {
  tags: ["test"],
  decorators: [
    (Story) => (
      <div style={{ width: CARD_PX_AT_390 }}>
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    const table = canvas.getByRole("table", { name: "Members grid" });
    const scroller = table.parentElement!;

    const firstMetric = canvas.getByRole("button", {
      name: "Commits over the period — sort by this column",
    });
    expectHorizontallyContained(
      scroller,
      [firstMetric.closest("th")!],
      () => "the first metric column"
    );

    // Reaching that by shrinking the names to nothing would be no better than
    // the bug: the column has to still hold a name.
    const name = canvas.getByRole("link", { name: "Alexandra Fernandez" });
    await expect(
      Math.round(name.getBoundingClientRect().width),
      "the member column collapsed the name away"
    ).toBeGreaterThan(80);
  },
};

/** Given the room, the names get the width they read best at. */
export const TestWideRosterKeepsTheComfortableNameColumn: Story = {
  tags: ["test"],
  decorators: [
    (Story) => (
      <div style={{ width: CARD_PX_WIDE }}>
        <Story />
      </div>
    ),
  ],
  play: async ({ canvas }) => {
    const table = canvas.getByRole("table", { name: "Members grid" });
    const width = memberColumnWidth(table);
    await expect(
      width,
      `the member column is ${width}px on a ${CARD_PX_WIDE}px card`
    ).toBe(256);
  },
};
