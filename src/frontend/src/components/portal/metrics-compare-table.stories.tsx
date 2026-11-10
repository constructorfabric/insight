import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";

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
