/** See docs/testing/storybook-component-tests.md. */
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect } from "storybook/test";

import { AlertPage } from "@/components/alerts/alert-page";
import { NewAlertPage } from "@/components/alerts/alert-form";
import { AlertsList } from "@/components/alerts/alerts-list";
import { ALERT } from "@/test/alerts";
import {
  CARD_PX_AT_390,
  CARD_PX_WIDE,
  expectHorizontallyContained,
  expectNothingOverlaps,
} from "@/test/storybook/layout";

const BREACHED = {
  ...ALERT,
  name: "Open pull requests waiting on review across every repository",
  range: "P7D",
  state: {
    last_evaluated_at: "2026-10-01T09:00:00+00:00",
    last_outcome: "breach",
    last_value: 12,
    last_valid_breached: true,
    breached_since: "2026-10-01T09:00:00+00:00",
  },
};

const NOTIFICATIONS = [
  {
    id: "n2",
    rule_revision: 3,
    metric: "prs-open",
    column: "total",
    operator: ">",
    threshold: 10,
    value: 12,
    evaluated_at: "2026-10-01T09:00:00+00:00",
    destination: "ops",
    status: "sent",
    attempts: 1,
    provider_receipt: "12176902",
    created_at: "2026-10-01T09:00:00+00:00",
  },
  {
    id: "n1",
    rule_revision: 2,
    metric: "prs-open",
    column: "total",
    operator: ">",
    threshold: 10,
    value: 14,
    evaluated_at: "2026-09-30T15:30:00+00:00",
    destination: "ops",
    status: "failed",
    attempts: 5,
    last_error: "the provider rejected the message: answered 400 Bad Request",
    created_at: "2026-09-30T15:30:00+00:00",
  },
];

const handlers = [
  http.get("*/api/v3/v1/alerts", () =>
    HttpResponse.json({
      alerts: [
        { id: "a1", name: BREACHED.name, metric: "prs-open", enabled: true },
        {
          id: "a2",
          name: "Failed deploys",
          metric: "deploys-failed",
          enabled: false,
        },
      ],
      total: 2,
      limit: 50,
      offset: 0,
    })
  ),
  http.get("*/api/v3/v1/alerts/:id", () => HttpResponse.json(BREACHED)),
  http.get("*/api/v3/v1/alerts/:id/notifications", () =>
    HttpResponse.json({ notifications: NOTIFICATIONS, limit: 20, offset: 0 })
  ),
  http.get("*/api/v3/v1/alert-destinations", () =>
    HttpResponse.json({ destinations: [{ name: "ops", provider: "zulip" }] })
  ),
  http.get("*/api/v3/v1/metrics", () =>
    HttpResponse.json({ names: ["prs-open"], total: 1 })
  ),
];

function Screen({ which }: { which: "list" | "alert" | "new" }) {
  if (which === "list") return <AlertsList />;
  if (which === "alert") return <AlertPage id="a1" />;
  return <NewAlertPage />;
}

const meta: Meta<typeof Screen> = {
  title: "Alerts/Screens",
  component: Screen,
  parameters: { msw: { handlers } },
};
export default meta;

type Story = StoryObj<typeof Screen>;

function at(width: number): Story["decorators"] {
  return [
    (Story) => (
      <div style={{ width }}>
        <Story />
      </div>
    ),
  ];
}

export const List: Story = {
  args: { which: "list" },
  decorators: at(CARD_PX_WIDE),
};

export const Alert: Story = {
  args: { which: "alert" },
  decorators: at(CARD_PX_WIDE),
};

export const NewAlert: Story = {
  args: { which: "new" },
  decorators: at(CARD_PX_WIDE),
};

export const TestAlertHeaderWrapsOnAPhone: Story = {
  args: { which: "alert" },
  decorators: at(CARD_PX_AT_390),
  tags: ["test"],
  play: async ({ canvas, canvasElement }) => {
    const title = await canvas.findByRole("heading", { name: BREACHED.name });
    const actions = [
      canvas.getByRole("switch", { name: `Checks for ${BREACHED.name}` }),
      canvas.getByRole("button", { name: "Edit" }),
      canvas.getByRole("button", { name: "Delete" }),
    ];

    await expect(title).toBeVisible();
    expectNothingOverlaps(
      title,
      actions,
      (element) => `"${element.textContent}"`
    );
    expectHorizontallyContained(
      canvasElement,
      [title, ...actions, ...(await canvas.findAllByRole("table"))],
      (element) => element.tagName.toLowerCase()
    );
  },
};

export const TestFormFitsOnAPhone: Story = {
  args: { which: "new" },
  decorators: at(CARD_PX_AT_390),
  tags: ["test"],
  play: async ({ canvas, canvasElement }) => {
    const fields = [
      await canvas.findByLabelText("Name"),
      canvas.getByLabelText("Metric"),
      canvas.getByLabelText("Column"),
      canvas.getByLabelText("Window"),
      canvas.getByLabelText("The value is"),
      canvas.getByLabelText("Threshold"),
      canvas.getByLabelText("Check"),
      canvas.getByLabelText("Send to"),
      canvas.getByRole("button", { name: "Create alert" }),
    ];

    expectHorizontallyContained(
      canvasElement,
      fields,
      (element) => element.getAttribute("id") ?? element.tagName.toLowerCase()
    );
  },
};
