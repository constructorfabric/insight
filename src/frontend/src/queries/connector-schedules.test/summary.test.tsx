import { instance, summary, read, setup } from "./fixtures";
import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import * as api from "@/api/connector-health-client";
import type { ScheduleRead } from "@/api/connector-health-client";
import {
  useConnectorHealth,
  useConnectorSchedule,
  useWriteConnectorSchedule,
} from "../connector-health";

it.each([
  ["applied", "revision"],
  ["inherited", "revision"],
  ["applied", "observation"],
  ["inherited", "observation"],
] as const)(
  "refreshes an open quiet pane for another admin's %s %s",
  async (status, change) => {
    vi.useFakeTimers();
    const { client, wrapper } = setup();
    const initial: ScheduleRead = {
      ...read,
      schedule: {
        ...read.schedule,
        status: "applied",
        live: {
          crons: ["0 6 * * *"],
          timezone: "UTC",
          origin: "api",
          suspended: false,
          observed_at: summary.as_of,
        },
      },
    };
    const next: ScheduleRead = {
      etag: change === "revision" ? '"2"' : '"1"',
      schedule: {
        ...initial.schedule,
        revision: change === "revision" ? 2 : 1,
        status,
        requested:
          status === "inherited"
            ? null
            : {
                crons: ["0 8 * * *"],
                updated_at: summary.as_of,
                updated_by: "example-admin",
              },
        live: {
          ...initial.schedule.live!,
          crons: [status === "inherited" ? "0 4 * * *" : "0 8 * * *"],
          observed_at: "2026-01-15T12:01:00Z",
        },
      },
    };
    const row = {
      ...instance,
      configured: true,
      schedule_request: {
        crons: ["0 6 * * *"],
        revision: 1,
        last_observation: {
          status: "applied" as const,
          observed_at: summary.as_of,
        },
      },
    };
    vi.mocked(api.getConnectorSchedule)
      .mockResolvedValueOnce(initial)
      .mockResolvedValue(next);
    vi.mocked(api.getConnectorHealth)
      .mockResolvedValueOnce({ ...summary, connectors: [row] })
      .mockResolvedValue({
        ...summary,
        connectors: [
          {
            ...row,
            schedule_request: {
              crons: next.schedule.requested?.crons ?? [],
              revision: next.schedule.revision,
              last_observation: {
                status,
                observed_at: next.schedule.live!.observed_at,
              },
            },
          },
        ],
      });
    const hook = renderHook(
      () => {
        const health = useConnectorHealth();
        return useConnectorSchedule(
          health.data?.connectors[0] ?? instance,
          false
        );
      },
      { wrapper }
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(api.getConnectorSchedule).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(api.getConnectorSchedule).toHaveBeenCalledTimes(2);
    expect(hook.result.current.data).toEqual(next);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(api.getConnectorSchedule).toHaveBeenCalledTimes(2);
    hook.unmount();
    client.clear();
  }
);

it("keeps a confirmed reset's Next sync through summary refetch until a newer live read", async () => {
  const { client, wrapper } = setup();
  const old = {
    ...summary,
    checked_at: "2026-01-15T11:00:00Z",
    as_of: "2026-01-15T11:00:00Z",
    connectors: [
      {
        ...instance,
        configured: true,
        schedule_request: {
          revision: 1,
          crons: ["0 6 * * *"],
          last_observation: {
            status: "applied" as const,
            observed_at: "2026-01-15T11:00:00Z",
          },
        },
        schedule: { crons: ["0 6 * * *"], suspended: false, timezone: "UTC" },
      },
    ],
  };
  vi.mocked(api.getConnectorHealth).mockResolvedValue(old);
  const reset: ScheduleRead = {
    ...read,
    schedule: {
      ...read.schedule,
      revision: 2,
      status: "inherited",
      requested: null,
      live: {
        crons: ["0 4 * * *"],
        suspended: false,
        timezone: "UTC",
        origin: "descriptor",
        observed_at: summary.as_of,
      },
    },
  };
  vi.mocked(api.deleteConnectorSchedule).mockResolvedValue(reset);
  const hook = renderHook(
    () => ({
      summary: useConnectorHealth(),
      write: useWriteConnectorSchedule(instance),
    }),
    { wrapper }
  );
  await waitFor(() => expect(hook.result.current.summary.isSuccess).toBe(true));
  await act(async () => {
    await hook.result.current.write.mutateAsync({ etag: '"1"', crons: null });
  });
  await waitFor(() =>
    expect(
      hook.result.current.summary.data?.connectors[0].schedule?.crons
    ).toEqual(["0 4 * * *"])
  );
  expect(
    hook.result.current.summary.data?.connectors[0].schedule_request?.crons
  ).toEqual([]);
  vi.mocked(api.getConnectorHealth).mockResolvedValue({
    ...old,
    as_of: "2026-01-15T12:05:00Z",
    connectors: [
      {
        ...old.connectors[0],
        schedule_request: {
          ...old.connectors[0].schedule_request,
          last_observation: {
            status: "pending",
            observed_at: "2026-01-15T12:05:00Z",
          },
        },
        schedule: { ...old.connectors[0].schedule, crons: ["0 8 * * *"] },
      },
    ],
  });
  await act(async () => {
    await hook.result.current.summary.refetch();
  });
  await waitFor(() =>
    expect(
      hook.result.current.summary.data?.connectors[0].schedule?.crons
    ).toEqual(["0 8 * * *"])
  );
  hook.unmount();
  client.clear();
});

it("shows a newer live manual change even while the warehouse stamp predates an applied cache", async () => {
  const { client, wrapper } = setup();
  const cached: ScheduleRead = {
    ...read,
    schedule: {
      ...read.schedule,
      status: "applied",
      live: {
        crons: ["0 6 * * *"],
        timezone: "UTC",
        suspended: false,
        observed_at: "2026-01-15T12:00:00Z",
      },
    },
  };
  client.setQueryData(
    ["connector-health", "schedule", "example-tracker//main", "example-scope"],
    cached
  );
  vi.mocked(api.getConnectorHealth).mockResolvedValue({
    ...summary,
    as_of: "2026-01-15T12:05:00Z",
    checked_at: "2026-01-15T11:00:00Z",
    connectors: [
      {
        ...instance,
        configured: true,
        schedule: { crons: ["0 9 * * *"], timezone: "UTC", suspended: false },
        schedule_request: {
          revision: 1,
          crons: ["0 6 * * *"],
          last_observation: {
            status: "pending",
            observed_at: "2026-01-15T12:05:00Z",
          },
        },
      },
    ],
  });
  const hook = renderHook(() => useConnectorHealth(), { wrapper });
  await waitFor(() => expect(hook.result.current.isSuccess).toBe(true));
  expect(hook.result.current.data!.connectors[0].schedule!.crons).toEqual([
    "0 9 * * *",
  ]);
  expect(
    hook.result.current.data!.connectors[0].schedule_request!.last_observation!
      .status
  ).toBe("pending");
  hook.unmount();
  client.clear();
});
