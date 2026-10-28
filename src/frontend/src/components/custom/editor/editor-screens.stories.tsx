/** See docs/testing/storybook-component-tests.md. */
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, userEvent } from "storybook/test";

import type { EditableKind } from "@/api/custom-client";
import type { Dataset, DefinitionResponse } from "@/api/custom-types";
import { EditorPage } from "@/components/custom/editor/editor-page";
import {
  CARD_PX_AT_390,
  CARD_PX_WIDE,
  expectHorizontallyContained,
} from "@/test/storybook/layout";

const PULL_REQUESTS: Dataset = {
  name: "pull-requests",
  declaration: {
    title: "Pull requests",
    fields: [
      { name: "team", path: "team", type: "string" },
      { name: "id", path: "id", type: "string" },
      { name: "opened_at", path: "opened_at", type: "datetime" },
    ],
  },
};

const METRIC: DefinitionResponse<Record<string, unknown>> = {
  id: "m1",
  name: "Open pull requests by team",
  body: {
    dataset: "pull-requests",
    fields: [
      { field: "team", type: "string", as_name: "team" },
      { field: "id", type: "string", agg: "count", as_name: "open" },
    ],
    group_by: ["team"],
    limit: 50,
  },
};

const WIDGET: DefinitionResponse<Record<string, unknown>> = {
  id: "w1",
  name: "Open pull requests",
  body: { type: "stat", metric: "m1", value: "open" },
};

const DASHBOARD: DefinitionResponse<Record<string, unknown>> = {
  id: "d1",
  name: "Delivery",
  body: {
    title: "Delivery",
    items: [{ heading: "Review flow" }, { widget: "w1" }],
  },
  access: "owner",
};

const STORED = { metrics: METRIC, widgets: WIDGET, dashboards: DASHBOARD };

const REFUSAL = {
  detail: "Request validation failed",
  context: {
    field_violations: [
      { field: "limit", description: "must be positive", reason: "INVALID" },
      {
        field: "fields[1].agg",
        description: "unknown aggregate",
        reason: "INVALID",
      },
    ],
  },
};

const API = "*/api/v3/v1";

/**
 * Keyed so a story replaces one group and keeps the rest: the addon merges
 * story handlers over the meta's by key.
 */
const handlers = {
  datasets: [
    http.get(`${API}/datasets`, () =>
      HttpResponse.json({ names: [PULL_REQUESTS.name], total: 1 })
    ),
    http.get(`${API}/datasets/:name`, () => HttpResponse.json(PULL_REQUESTS)),
    http.put(`${API}/datasets/:name`, () => HttpResponse.json(PULL_REQUESTS)),
  ],
  definitions: (["metrics", "widgets", "dashboards"] as const).flatMap(
    (kind) => [
      http.get(`${API}/${kind}`, () =>
        HttpResponse.json({ items: [], total: 0, limit: 50, offset: 0 })
      ),
      http.get(`${API}/${kind}/:id`, () => HttpResponse.json(STORED[kind])),
      http.post(`${API}/${kind}`, () => HttpResponse.json({ id: "new" })),
      http.put(
        `${API}/${kind}/:id`,
        () => new HttpResponse(null, { status: 204 })
      ),
    ]
  ),
};

function Screen({ kind, id }: { kind: EditableKind; id?: string }) {
  return <EditorPage kind={kind} id={id} />;
}

const meta: Meta<typeof Screen> = {
  title: "Custom/Editor Screens",
  component: Screen,
  parameters: { msw: { handlers } },
};
export default meta;

type Story = StoryObj<typeof Screen>;

/** The card a screen is given, which the phone story measures against. */
function at(width: number): Story["decorators"] {
  return [
    (Story) => (
      <div data-testid="card" style={{ width }}>
        <Story />
      </div>
    ),
  ];
}

const wide = at(CARD_PX_WIDE);

export const NewDataset: Story = {
  args: { kind: "datasets" },
  decorators: wide,
};

export const NewMetric: Story = { args: { kind: "metrics" }, decorators: wide };

export const NewWidget: Story = { args: { kind: "widgets" }, decorators: wide };

export const NewDashboard: Story = {
  args: { kind: "dashboards" },
  decorators: wide,
};

/** A dataset keeps its name; removing it takes its records too. */
export const EditDataset: Story = {
  args: { kind: "datasets", id: PULL_REQUESTS.name },
  decorators: wide,
};

export const EditMetric: Story = {
  args: { kind: "metrics", id: METRIC.id },
  decorators: wide,
};

export const EditWidget: Story = {
  args: { kind: "widgets", id: WIDGET.id },
  decorators: wide,
};

export const EditDashboard: Story = {
  args: { kind: "dashboards", id: DASHBOARD.id },
  decorators: wide,
};

export const NotFound: Story = {
  args: { kind: "metrics", id: "missing" },
  decorators: wide,
  parameters: {
    msw: {
      handlers: {
        definitions: [
          http.get(`${API}/metrics/:id`, () =>
            HttpResponse.json(
              { detail: "No metric is stored under that id." },
              { status: 404 }
            )
          ),
        ],
      },
    },
  },
};

/** Save is pressed for you; each complaint is listed under the document. */
export const TestRefusedSaveListsEachComplaint: Story = {
  args: { kind: "metrics", id: METRIC.id },
  decorators: wide,
  tags: ["test"],
  parameters: {
    msw: {
      handlers: {
        definitions: [
          http.get(`${API}/metrics/:id`, () => HttpResponse.json(METRIC)),
          http.put(`${API}/metrics/:id`, () =>
            HttpResponse.json(REFUSAL, { status: 400 })
          ),
        ],
      },
    },
  },
  play: async ({ canvas }) => {
    await canvas.findByLabelText(/as text/i);
    await userEvent.click(canvas.getByRole("button", { name: "Save" }));

    const alerts = await canvas.findAllByRole("alert");
    await expect(alerts.map((alert) => alert.textContent)).toEqual([
      "limit: must be positive",
      "fields[1].agg: unknown aggregate",
    ]);
  },
};

export const TestEditorFitsOnAPhone: Story = {
  args: { kind: "metrics", id: METRIC.id },
  decorators: at(CARD_PX_AT_390),
  tags: ["test"],
  play: async ({ canvas }) => {
    const card = canvas.getByTestId("card");
    const controls = [
      await canvas.findByLabelText(/as text/i),
      canvas.getByRole("textbox", { name: "Name" }),
      canvas.getByRole("button", { name: "Save" }),
    ];

    expectHorizontallyContained(card, controls, (element) =>
      element.tagName.toLowerCase()
    );
  },
};
