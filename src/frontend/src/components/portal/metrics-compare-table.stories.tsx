import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";

import type { MetricDirection, MetricFormat } from "@/api/metric-results-client";
import { EvidenceDialogContext } from "@/components/metric-evidence-context";
import { MetricsCompareTable } from "@/components/portal/metrics-compare-table";
import { normalizeMetricResults } from "@/lib/metrics/collection";

const PEOPLE = [
  { entityId: "019e27bc-dec0-7626-81a9-c5524662a6a9", displayName: "Alexandra Fernandez" },
  { entityId: "019e27bc-dec0-7626-81a9-c5524662a6b0", displayName: "Bo Ng" },
];

const BY_KEY = normalizeMetricResults([
  {
    metric_key: "git.commits",
    label: "Commits",
    unit: "commits",
    format: "integer",
    computation: "sum",
    direction: "higher_is_better",
    drilldown: { granularity: ["event"] },
    selection: {
      metric_key: "git.commits",
      entity: { type: "person", ids: PEOPLE.map((person) => person.entityId) },
      period: { from: "2026-10-01", to: "2026-10-31" },
      filters: [],
    },
    views: [
      {
        view: "period",
        values: [
          { entity_id: PEOPLE[0]!.entityId, value: 210 },
          { entity_id: PEOPLE[1]!.entityId, value: 822 },
        ],
      },
    ],
  },
]);

const meta: Meta<typeof MetricsCompareTable> = {
  title: "Portal/MetricsCompareTable",
  component: MetricsCompareTable,
  args: {
    people: PEOPLE,
    keys: ["git.commits"],
    byKey: BY_KEY,
    helpByKey: new Map(),
    roles: new Map([[PEOPLE[0]!.entityId, "Software Engineer"]]),
  },
  decorators: [
    (Story) => (
      <EvidenceDialogContext.Provider
        value={{ openEvidence: () => {}, openEvidenceTargets: () => {}, openEvidencePeople: () => {} }}
      >
        <div style={{ width: 900 }}>
          <Story />
        </div>
      </EvidenceDialogContext.Provider>
    ),
  ],
};

export default meta;

type Story = StoryObj<typeof MetricsCompareTable>;

const STRESS_PEOPLE = [
  { entityId: "person-01", displayName: "Alexandra Fernandez-Montgomery" },
  { entityId: "person-02", displayName: "Bo Ng" },
  { entityId: "person-03", displayName: "Priyanka Venkateswaran-Subramanian" },
  { entityId: "person-04", displayName: "Jean-Baptiste de la Fontaine-Moreau" },
  { entityId: "person-05", displayName: "Kim Li" },
  { entityId: "person-06", displayName: "Anastasia Konstantinovna Sokolova" },
  { entityId: "person-07", displayName: "Diego Alejandro Hernández-Gutiérrez" },
  { entityId: "person-08", displayName: "Samir al-Rashid" },
  { entityId: "person-09", displayName: "Morgan Quinn-Okonkwo" },
  { entityId: "person-10", displayName: "Christopher Williamson-Blake III" },
  { entityId: "person-11", displayName: "Yuki Tanaka-Nakamura" },
  { entityId: "person-12", displayName: "Helena Björk-Lindström" },
];

const STRESS_ROLES = new Map([
  [STRESS_PEOPLE[0]!.entityId, "Senior Staff Software Engineer, Developer Experience"],
  [STRESS_PEOPLE[2]!.entityId, "Engineering Manager, Platform"],
  [STRESS_PEOPLE[6]!.entityId, "Principal Engineer"],
]);

function stressMetric(
  metricKey: string,
  label: string,
  format: MetricFormat,
  unit: string | null,
  direction: MetricDirection,
  values: readonly number[],
) {
  return {
    metric_key: metricKey,
    label,
    unit,
    format,
    computation: "sum" as const,
    direction,
    drilldown: { granularity: ["event" as const] },
    selection: {
      metric_key: metricKey,
      entity: { type: "person" as const, ids: STRESS_PEOPLE.map((person) => person.entityId) },
      period: { from: "2026-10-01", to: "2026-10-31" },
      filters: [],
    },
    views: [
      {
        view: "period" as const,
        values: STRESS_PEOPLE.map((person, index) => ({
          entity_id: person.entityId,
          value: values[index]!,
        })),
      },
    ],
  };
}

const STRESS_BY_KEY = normalizeMetricResults([
  stressMetric("git.lines_changed", "Lines changed", "integer", "lines", "higher_is_better", [
    18_432_109, 12, 4_250_881, 980_441.6, 1, 7_777_777, 120_004, 56, 2_500_000.4, 64_000_321, 890, 3_333_333,
  ]),
  stressMetric("git.review_cost", "Review cost", "currency", "USD", "lower_is_better", [
    1_250_000.49, 89.2, 12_345_678.9, 420.15, 0.4, 8_888_888.88, 15_000.5, 2.5, 999_999.99, 3_141_592.65, 75, 640_000,
  ]),
  stressMetric("git.cycle_time_h", "Pull request cycle time", "decimal", "h", "lower_is_better", [
    1.25, 26.75, 148.333333333, 0.083333333, 1234.567890123, 0.5, 99.999, 8.5, 2_500.125, 0.0166667, 48.016666, 1_500_000.75,
  ]),
  stressMetric("git.merge_rate", "Merge rate", "percent", null, "higher_is_better", [
    99.6, 0.4, 12.3456789, 87.654321, 100, 3.1415926535, 50, 1.999, 73.25, 0.01, 64.8, 8.888888,
  ]),
  stressMetric(
    "git.review_turnaround",
    "Pull request review turnaround excluding weekends and holidays",
    "decimal",
    null,
    "lower_is_better",
    [0.123456789, 12.987654321, 1_234_567.891234567, 8.25, 0.000123456, 456_789.123456, 3.5, 19.75, 88_888.888888, 2.2, 7.125, 10_000.5],
  ),
  stressMetric("collab.messages", "Messages sent", "integer", "messages", "neutral", [
    4, 18, 2_200, 640, 12_345, 7, 890_123, 55, 1_001, 42, 3_300_000, 19,
  ]),
]);

const STRESS_KEYS = [...STRESS_BY_KEY.keys()];

export const Stress: Story = {
  tags: ["test"],
  args: {
    people: STRESS_PEOPLE,
    keys: STRESS_KEYS,
    byKey: STRESS_BY_KEY,
    helpByKey: new Map(),
    roles: STRESS_ROLES,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const table = canvasElement.querySelector("table")!;
    const scroller = table.parentElement!;
    const header = table.querySelector("thead tr")!;

    await expect(
      scroller.scrollWidth,
      "twelve people still have to scroll instead of crushing the columns",
    ).toBeGreaterThan(scroller.clientWidth + 8);
    await expect(header.getBoundingClientRect().height, "names wrap into a tall header").toBeLessThan(96);

    for (const cell of table.querySelectorAll("th, td")) {
      const spilled = textSpillingOutOf(cell);
      await expect(spilled, "text leaves its cell").toBeNull();
    }

    for (const slot of table.querySelectorAll("td span.inline-block")) {
      await expect(slot.scrollWidth, slot.textContent ?? "").toBeLessThanOrEqual(slot.clientWidth + 1);
    }

    const lines = canvas.getByRole("button", {
      name: "Alexandra Fernandez-Montgomery — Lines changed: 18,432,109 lines",
    });
    await expect(lines).toHaveTextContent("18.4M");
    await expect(lines).toHaveAttribute("title", "18,432,109 lines");

    const cost = canvas.getByRole("button", {
      name: "Alexandra Fernandez-Montgomery — Review cost: $1,250,000",
    });
    const gap = cost.parentElement?.querySelector("span[title]");
    await expect(gap?.textContent).toBe("+312.5M%");
    await expect(gap?.getAttribute("title")).toMatch(/%$/);

    const hours = canvas.getByRole("button", {
      name: "Helena Björk-Lindström — Pull request cycle time: 1,500,001 h",
    });
    await expect(hours).toHaveTextContent("1.5M h");
    await expect(hours).toHaveAttribute("title", "1,500,001 h");

    for (const link of canvas.getAllByRole("link")) {
      await expect(link.getBoundingClientRect().height, link.textContent ?? "").toBeLessThan(28);
    }

    scroller.scrollLeft = 280;
    const metric = canvas.getByRole("columnheader", { name: "Metric" });
    await expect(
      Math.abs(metric.getBoundingClientRect().left - scroller.getBoundingClientRect().left),
    ).toBeLessThan(2);
  },
};

function textSpillingOutOf(cell: Element): string | null {
  const box = cell.getBoundingClientRect();
  const walker = document.createTreeWalker(cell, NodeFilter.SHOW_TEXT);
  let node = walker.nextNode();
  while (node) {
    const raw = node.textContent ?? "";
    const parent = node.parentElement;
    if (raw.trim() && !parent?.closest(".sr-only")) {
      const range = document.createRange();
      range.selectNodeContents(node);
      const rect = range.getBoundingClientRect();
      const spills = rect.width > 0 && (rect.left < box.left - 1 || rect.right > box.right + 1);
      if (spills && !clippedInside(parent, cell, box)) return raw.trim();
    }
    node = walker.nextNode();
  }
  return null;
}

function clippedInside(start: HTMLElement | null, cell: Element, box: DOMRect): boolean {
  let el = start;
  while (el && el !== cell) {
    const overflow = getComputedStyle(el).overflowX;
    if (overflow === "hidden" || overflow === "clip") {
      const rect = el.getBoundingClientRect();
      return rect.left >= box.left - 1 && rect.right <= box.right + 1;
    }
    el = el.parentElement;
  }
  return false;
}

export const TestEachNameLinesUpWithItsValues: Story = {
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    for (const [name, value] of [
      ["Alexandra Fernandez", "210 commits"],
      ["Bo Ng", "822 commits"],
    ] as const) {
      const nameEdge = canvas.getByRole("link", { name }).getBoundingClientRect().right;
      const text = document.createRange();
      text.selectNodeContents(canvas.getByRole("button", { name: new RegExp(`${value}$`) }));
      const valueEdge = text.getBoundingClientRect().right;

      await expect(
        Math.abs(nameEdge - valueEdge),
        `${name} does not end where its values end`,
      ).toBeLessThanOrEqual(1);
    }
  },
};
