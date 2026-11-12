import { auth, instance, summary, read, setup } from "./fixtures";
import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import * as api from "@/api/connector-health-client";
import type {
  ConnectorHealthSummary,
  ScheduleRead,
} from "@/api/connector-health-client";
import {
  useConnectorHealth,
  useConnectorSchedule,
  useWriteConnectorSchedule,
  schedulePollInterval,
} from "../connector-health";

it.each(["put", "reset"])(
  "%s updates only the matching summary row in the current scope before refetch",
  async (verb) => {
    const { client, wrapper } = setup();
    const key = ["connector-health", "summary", auth.scope];
    const otherKey = ["connector-health", "summary", "other-scope"];
    const initial = {
      ...summary,
      connectors: [
        { ...instance, configured: true },
        { ...instance, source_id: "other", configured: true },
      ],
    };
    client.setQueryData(key, initial);
    client.setQueryData(otherKey, initial);
    const updated: ScheduleRead = {
      ...read,
      schedule: {
        ...read.schedule,
        status: verb === "put" ? "applied" : "inherited",
        requested: {
          crons: verb === "put" ? ["*/5 * * * *"] : [],
          updated_at: summary.as_of,
          updated_by: "example-admin",
        },
        live: {
          crons: ["*/5 * * * *"],
          suspended: false,
          timezone: "UTC",
          observed_at: summary.as_of,
        },
      },
    };
    vi.mocked(api.putConnectorSchedule).mockResolvedValue(updated);
    vi.mocked(api.deleteConnectorSchedule).mockResolvedValue(updated);
    let cached: ConnectorHealthSummary | undefined;
    vi.spyOn(client, "invalidateQueries").mockImplementation(async () => {
      cached = client.getQueryData(key);
    });
    const hook = renderHook(() => useWriteConnectorSchedule(instance), {
      wrapper,
    });
    await act(async () => {
      await hook.result.current.mutateAsync({
        etag: '"0"',
        crons: verb === "put" ? ["*/5 * * * *"] : null,
      });
    });
    expect(cached?.connectors[0].schedule_request).toEqual({
      revision: 1,
      crons: verb === "put" ? ["*/5 * * * *"] : [],
      last_observation: {
        status: updated.schedule.status,
        observed_at: summary.as_of,
      },
    });
    expect(cached?.connectors[0].schedule).toEqual(updated.schedule.live);
    expect(cached?.connectors[1]).toEqual(initial.connectors[1]);
    expect(client.getQueryData(otherKey)).toEqual(initial);
    expect(api.getConnectorHealth).not.toHaveBeenCalled();
    hook.unmount();
    client.clear();
  }
);

it("polls a waiting schedule every thirty seconds", async () => {
  vi.useFakeTimers();
  const { client, wrapper } = setup();
  vi.mocked(api.getConnectorSchedule).mockResolvedValue({
    ...read,
    schedule: {
      ...read.schedule,
      status: "pending",
      pending_reason: "different_schedule",
    },
  });
  const hook = renderHook(() => useConnectorSchedule(instance, false), {
    wrapper,
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1);
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(119_999);
  });
  expect(api.getConnectorSchedule).toHaveBeenCalledTimes(5);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(29_999);
  });
  expect(api.getConnectorSchedule).toHaveBeenCalledTimes(5);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1);
  });
  expect(api.getConnectorSchedule).toHaveBeenCalledTimes(6);
  hook.unmount();
  client.clear();
});

it.each(["no_cronworkflow", "awaiting_reconcile"] as const)(
  "polls %s every thirty seconds including near reconcile",
  async (pending_reason) => {
    vi.useFakeTimers();
    const { client, wrapper } = setup();
    vi.mocked(api.getConnectorSchedule).mockResolvedValue({
      ...read,
      schedule: {
        ...read.schedule,
        status: "pending",
        pending_reason,
        next_reconcile_at: new Date(Date.now() + 45_000).toISOString(),
      },
    });
    const hook = renderHook(() => useConnectorSchedule(instance, false), {
      wrapper,
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(api.getConnectorSchedule).toHaveBeenCalledTimes(2);
    hook.unmount();
    client.clear();
  }
);

it("uses the normal pending interval without a near reconcile estimate", () => {
  const now = Date.now();
  for (const next_reconcile_at of [
    null,
    new Date(now + 90_000).toISOString(),
    new Date(now - 1).toISOString(),
  ]) {
    expect(
      schedulePollInterval(
        {
          ...read,
          schedule: {
            ...read.schedule,
            status: "pending",
            pending_reason: "awaiting_reconcile",
            next_reconcile_at,
          },
        },
        false
      )
    ).toBe(30_000);
  }
  expect(schedulePollInterval(read, false)).toBe(false);
});

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

it.each(["pending"] as const)(
  "polls summary and the open schedule every 30 seconds for %s, then stops the extra polling",
  async (status) => {
    const { client, wrapper } = setup();
    vi.useFakeTimers();
    const request = {
      crons: ["0 6 * * *"],
      revision: 2,
      last_observation: { status, observed_at: summary.as_of },
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
              row.schedule_request?.last_observation?.status === "pending"
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
