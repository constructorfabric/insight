/**
 * Stories for the Data health pane's connector table. Fixture data only — the
 * connector names come from the repo's own connector catalogue and every count
 * and timestamp is invented.
 *
 * See docs/testing/storybook-component-tests.md.
 */

import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen } from "storybook/test";
import { http, HttpResponse } from "msw";

import { ManageView } from "@/components/portal/manage-view";

const CONNECTORS = [
  { connector: "github", namespace: "bronze_github", streams: 18, streams_with_data: 18, rows: 412_004, last_write: "2020-01-10T09:00:00Z" },
  { connector: "jira", namespace: "bronze_jira", streams: 14, streams_with_data: 14, rows: 88_120, last_write: "2020-01-10T04:00:00Z" },
  { connector: "confluence", namespace: "bronze_confluence", streams: 7, streams_with_data: 5, rows: 1_902, last_write: "2020-01-08T12:00:00Z" },
  { connector: "slack", namespace: "bronze_slack", streams: 4, streams_with_data: 0, rows: 0, last_write: null },
];

const handlers = [
  http.get("/api/analytics/v1/connector-health", () =>
    HttpResponse.json({ as_of: "2020-01-10T12:00:00Z", connectors: CONNECTORS }),
  ),
  http.get("/api/analytics/v1/metric-definitions", () =>
    HttpResponse.json({ metrics: [] }),
  ),
];

const meta = {
  title: "Portal/Data health",
  component: ManageView,
  parameters: { msw: { handlers } },
} satisfies Meta<typeof ManageView>;

export default meta;

type Story = StoryObj<typeof meta>;

export const ConnectorDelivery: Story = {
  args: { item: "data-health" },
  play: async () => {
    await expect(
      await screen.findByText("Connectors · 3 of 4 have delivered"),
    ).toBeInTheDocument();
    await expect(screen.getByText("never")).toBeInTheDocument();
    await expect(screen.getByText("2 days ago")).toBeInTheDocument();
  },
};
