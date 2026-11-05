import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

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
import {
  useConnectorHealth,
  useConnectorSchedule,
  useWriteConnectorSchedule,
} from "./connector-health";

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
    applied: null,
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

it("keys schedules by instance and authorization scope and waits for an attributed instance", async () => {
  const { client, wrapper } = setup();
  const first = renderHook(() => useConnectorSchedule(instance, false), {
    wrapper,
  });
  await waitFor(() => expect(first.result.current.isSuccess).toBe(true));
  auth.scope = "second-example-scope";
  const second = renderHook(
    () => useConnectorSchedule({ ...instance, source_id: "second" }, false),
    { wrapper }
  );
  await waitFor(() => expect(second.result.current.isSuccess).toBe(true));
  renderHook(() => useConnectorSchedule({ connector: "unattributed" }, false), {
    wrapper,
  });
  expect(api.getConnectorSchedule).toHaveBeenCalledTimes(2);
  const keys = JSON.stringify(
    client
      .getQueryCache()
      .getAll()
      .map((query) => query.queryKey)
  );
  expect(keys).toContain("example-scope");
  expect(keys).toContain("second-example-scope");
  expect(keys).toContain("main");
  expect(keys).toContain("second");
  client.clear();
});

it.each(["put", "reset"])(
  "%s invalidates summary and row schedule after success",
  async (verb) => {
    const { client, wrapper } = setup();
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const otherKey = [
      "connector-health",
      "schedule",
      "example-tracker//main",
      "another-scope",
    ];
    const otherRead = { ...read, etag: '"8"' };
    client.setQueryData(otherKey, otherRead);
    vi.mocked(api.putConnectorSchedule).mockResolvedValue(read);
    vi.mocked(api.deleteConnectorSchedule).mockResolvedValue(read);
    const hook = renderHook(() => useWriteConnectorSchedule(instance), {
      wrapper,
    });
    await act(async () => {
      await hook.result.current.mutateAsync({
        etag: '"1"',
        crons: verb === "reset" ? null : ["0 3 * * *"],
      });
    });
    expect(
      verb === "reset" ? api.deleteConnectorSchedule : api.putConnectorSchedule
    ).toHaveBeenCalledWith(
      instance,
      '"1"',
      ...(verb === "reset" ? [] : [["0 3 * * *"]])
    );
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: ["connector-health", "summary"],
    });
    expect(client.getQueryData(otherKey)).toEqual(otherRead);
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: ["connector-health", "schedule", "example-tracker//main"],
    });
    client.clear();
  }
);

it.each(["pending", "error"] as const)(
  "polls summary and the open schedule every 30 seconds for %s, then stops the extra polling",
  async (status) => {
    const { client, wrapper } = setup();
    vi.useFakeTimers();
    const request = {
      crons: ["0 6 * * *"],
      revision: 2,
      status,
      updated_at: summary.as_of,
      updated_by: "example-admin",
    };
    vi.mocked(api.getConnectorHealth)
      .mockResolvedValueOnce({
        ...summary,
        connectors: [
          { ...instance, configured: true, schedule_request: request },
        ],
      })
      .mockResolvedValue(summary);
    vi.mocked(api.getConnectorSchedule)
      .mockResolvedValueOnce({
        ...read,
        schedule: { ...read.schedule, status },
      })
      .mockResolvedValue(read);
    const hook = renderHook(
      () => {
        const health = useConnectorHealth();
        return useConnectorSchedule(
          instance,
          health.data?.connectors.some(
            (row) =>
              row.schedule_request?.status === "pending" ||
              row.schedule_request?.status === "error"
          ) ?? false
        );
      },
      { wrapper }
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(api.getConnectorHealth).toHaveBeenCalledTimes(2);
    expect(api.getConnectorSchedule).toHaveBeenCalledTimes(2);
    expect(hook.result.current.data?.schedule.status).toBe("inherited");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(api.getConnectorHealth).toHaveBeenCalledTimes(2);
    expect(api.getConnectorSchedule).toHaveBeenCalledTimes(2);
    hook.unmount();
    client.clear();
  }
);
