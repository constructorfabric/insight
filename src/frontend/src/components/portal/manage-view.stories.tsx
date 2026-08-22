import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, screen } from "storybook/test";
import { http, HttpResponse } from "msw";

import { ManageView } from "@/components/portal/manage-view";
import { ADMIN_ROLE_ID } from "@/queries/identity-me";
import { authStore } from "@/auth/auth-store";

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
  http.get("/api/identity/v1/me", () =>
    HttpResponse.json({
      person_id: "00000000-0000-0000-0000-0000000000bb",
      insight_tenant_id: "00000000-0000-4000-8000-00000000c0de",
      roles: [{ role_id: ADMIN_ROLE_ID, name: "admin" }],
      visibility_policy: "org_chart",
    }),
  ),
];

const meta = {
  title: "Portal/Data health",
  component: ManageView,
  parameters: { msw: { handlers } },
  beforeEach: () => {
    authStore.setAuthenticated({
      personId: "00000000-0000-0000-0000-0000000000bb",
      email: "viewer@example.com",
      tenantId: "00000000-0000-4000-8000-00000000c0de",
      roles: ["admin"],
      impersonatorEmail: null,
      csrfToken: "story",
      expiresAt: 4_000_000_000,
      refreshAt: 3_999_000_000,
    });
  },
} satisfies Meta<typeof ManageView>;

export default meta;

type Story = StoryObj<typeof meta>;

export const ConnectorDelivery: Story = {
  args: { item: "data-health" },
};

export const TestConnectorDeliveryReportsEachConnectorsState: Story = {
  args: { item: "data-health" },
  tags: ["test"],
  play: async () => {
    await expect(await screen.findByText("never")).toBeInTheDocument();
    await expect(screen.getByText("2 days ago")).toBeInTheDocument();
    await expect(screen.getByText("not delivering")).toBeInTheDocument();
    await expect(screen.getAllByText("partial")).toHaveLength(2);
  },
};
