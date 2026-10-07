/** See docs/testing/storybook-component-tests.md. */
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect } from "storybook/test";

import type {
  Alert as AlertRule,
  AlertNotification,
  AlertPage as ListPage,
  AlertSummary,
  NotificationPage,
} from "@/api/alerts-types";
import { AlertPage } from "@/components/alerts/alert-page";
import { NewAlertPage } from "@/components/alerts/alert-form";
import { AlertsList } from "@/components/alerts/alerts-list";
import { ALERT, NOTIFICATION, SUMMARY } from "@/test/alerts";
import {
  CARD_PX_AT_390,
  CARD_PX_WIDE,
  expectHorizontallyContained,
  expectNothingOverlaps,
} from "@/test/storybook/layout";

const BREACHED: AlertRule = {
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

const SUMMARIES: AlertSummary[] = [
  { ...SUMMARY, name: BREACHED.name },
  { id: "a2", name: "Failed deploys", metric: "deploys-failed", enabled: false },
];

const LIST: ListPage = {
  alerts: SUMMARIES,
  total: SUMMARIES.length,
  limit: 50,
  offset: 0,
};

/** Newest first; the older one was owed under the rule's previous revision. */
const NOTIFICATIONS: AlertNotification[] = [
  { ...NOTIFICATION, id: "n2", provider_receipt: "12176902" },
  {
    ...NOTIFICATION,
    id: "n1",
    rule_revision: 2,
    column: "count",
    threshold: 8,
    value: 14,
    evaluated_at: "2026-09-30T15:30:00+00:00",
    status: "failed",
    attempts: 5,
    provider_receipt: null,
    last_error: "the provider rejected the message: answered 400 Bad Request",
    created_at: "2026-09-30T15:30:00+00:00",
  },
];

const HISTORY: NotificationPage = {
  notifications: NOTIFICATIONS,
  limit: 20,
  offset: 0,
};

const handlers = [
  http.get("*/api/v3/v1/alerts", () => HttpResponse.json(LIST)),
  http.get("*/api/v3/v1/alerts/:id", () => HttpResponse.json(BREACHED)),
  http.get("*/api/v3/v1/alerts/:id/notifications", () =>
    HttpResponse.json(HISTORY)
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

/** The card a screen is given, which the phone stories measure against. */
function at(width: number): Story["decorators"] {
  return [
    (Story) => (
      <div data-testid="card" style={{ width }}>
        <Story />
      </div>
    ),
  ];
}

function describeElement(element: Element): string {
  return (
    element.getAttribute("aria-label") ??
    element.getAttribute("id") ??
    element.textContent?.trim().slice(0, 40) ??
    element.tagName.toLowerCase()
  );
}

/**
 * A table wider than the card scrolls sideways inside it rather than running
 * off the card; its first column still reads without scrolling.
 */
function expectTablesReadWithin(card: Element, tables: readonly Element[]) {
  const scrollers = tables.map((table) => table.parentElement!);
  expectHorizontallyContained(card, scrollers, () => "a table's scroller");

  for (const table of tables) {
    expectHorizontallyContained(
      table.parentElement!,
      [table.querySelector("thead th")!],
      () => "a table's first column"
    );
  }
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
  play: async ({ canvas }) => {
    const card = canvas.getByTestId("card");
    const title = await canvas.findByRole("heading", { name: BREACHED.name });
    const actions = [
      canvas.getByRole("switch", { name: `Enable ${BREACHED.name}` }),
      canvas.getByRole("button", { name: "Edit" }),
      canvas.getByRole("button", { name: "Delete" }),
    ];
    const hint = canvas.getByText(/withdraws notifications not yet sent/);
    const tables = await canvas.findAllByRole("table");

    await expect(title).toBeVisible();
    expectNothingOverlaps(title, actions, describeElement);
    expectHorizontallyContained(
      card,
      [title, hint, ...actions],
      describeElement
    );
    expectTablesReadWithin(card, tables);
  },
};

export const TestListFitsOnAPhone: Story = {
  args: { which: "list" },
  decorators: at(CARD_PX_AT_390),
  tags: ["test"],
  play: async ({ canvas }) => {
    const card = canvas.getByTestId("card");
    const switches = await canvas.findAllByRole("switch");
    const longName = canvas.getByRole("link", { name: BREACHED.name });

    await expect(switches).toHaveLength(SUMMARIES.length);
    expectNothingOverlaps(longName, switches, describeElement);
    expectHorizontallyContained(
      card,
      [
        canvas.getByRole("heading", { name: "Alerts" }),
        canvas.getByRole("button", { name: "New alert" }),
        canvas.getByRole("searchbox", { name: "Search alerts" }),
        canvas.getByRole("table"),
        longName,
        ...switches,
      ],
      describeElement
    );
  },
};

export const TestFormFitsOnAPhone: Story = {
  args: { which: "new" },
  decorators: at(CARD_PX_AT_390),
  tags: ["test"],
  play: async ({ canvas }) => {
    const card = canvas.getByTestId("card");
    const fields = [
      await canvas.findByLabelText("Name"),
      canvas.getByLabelText("Metric"),
      canvas.getByLabelText("Column"),
      canvas.getByLabelText("Window"),
      canvas.getByLabelText("Condition"),
      canvas.getByLabelText("Threshold"),
      canvas.getByLabelText("Check every"),
      canvas.getByLabelText("Destination"),
      canvas.getByRole("button", { name: "Create alert" }),
    ];

    expectHorizontallyContained(card, fields, describeElement);
  },
};
