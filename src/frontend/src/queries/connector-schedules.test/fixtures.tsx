import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createElement, type ReactNode } from "react";
import { afterEach, beforeEach, vi } from "vitest";

import type {
  ConnectorHealthSummary,
  ScheduleRead,
} from "@/api/connector-health-client";

const auth = vi.hoisted(() => ({ scope: "example-scope" }));
vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: auth.scope }),
}));
vi.mock("@/auth/session-scope", () => ({
  sessionAuthorizationScope: (session: string) => session,
}));
vi.mock("@/api/connector-health-client", () => ({
  getConnectorHealth: vi.fn(),
  getConnectorSchedule: vi.fn(),
  putConnectorSchedule: vi.fn(),
  deleteConnectorSchedule: vi.fn(),
}));

import * as api from "@/api/connector-health-client";

const instance = { connector: "example-tracker", source_id: "main" };
const summary: ConnectorHealthSummary = {
  as_of: "2026-01-15T12:00:00Z",
  history_available: true,
  connectors: [],
};
const read: ScheduleRead = {
  etag: '"1"',
  schedule: {
    connector: "example-tracker",
    source_id: "main",
    revision: 1,
    requested: null,
    live: null,
    status: "inherited",
  },
};

function setup() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity },
      mutations: { retry: false },
    },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { client, wrapper };
}

beforeEach(() => {
  vi.clearAllMocks();
  auth.scope = "example-scope";
  vi.mocked(api.getConnectorHealth).mockResolvedValue(summary);
  vi.mocked(api.getConnectorSchedule).mockResolvedValue(read);
});
afterEach(() => vi.useRealTimers());

export { auth, instance, summary, read, setup };
