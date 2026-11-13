import { readFileSync } from "node:fs";
import { join } from "node:path";

import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, beforeEach, expect, it } from "vitest";

import type {
  ScheduleOrigin,
  SchedulePendingReason,
  ScheduleStatus,
} from "@/api/connector-health-client";
import { createConnectorScheduleMocks } from "./connector-schedules";

type OpenApiDocument = {
  components: {
    schemas: Record<
      "ScheduleStatus" | "PendingReason" | "ScheduleOrigin",
      { enum: string[] }
    > &
      Record<
        | "ScheduleResponse"
        | "LiveSchedule"
        | "InheritedSchedule"
        | "RequestedSchedule"
        | "HealthScheduleRequest"
        | "ScheduleObservation",
        { properties: Record<string, unknown> }
      > & { Problem: { required: string[] } };
  };
};

const openapi = JSON.parse(
  readFileSync(
    join(
      __dirname,
      "../../../../docs/components/backend/connectors/openapi.json"
    ),
    "utf8"
  )
) as OpenApiDocument;

let mocks = createConnectorScheduleMocks();
const server = setupServer();
const url = new URL(
  "/api/connectors/v1/connectors/example-tracker/main/schedule",
  window.location.origin
).href;
beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
beforeEach(() => {
  mocks = createConnectorScheduleMocks();
  server.use(...mocks.handlers);
});
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

it("mocks waiting and applied requests with in-memory revisions", async () => {
  const initial = await fetch(url);
  expect(initial.headers.get("ETag")).toBe('"0"');
  const saved = await fetch(url, {
    method: "PUT",
    headers: { "If-Match": '"0"', "Content-Type": "application/json" },
    body: JSON.stringify({ crons: ["0 6 * * *"] }),
  });
  expect(saved.status).toBe(200);
  expect(saved.headers.get("ETag")).toBe('"1"');
  expect((await saved.json()).status).toBe("pending");
  mocks.setStatus("example-tracker", "main", "pending");

  mocks.read("example-tracker", "main").pending_reason = "different_schedule";
  const stalled = await (await fetch(url)).json();
  expect(stalled.status).toBe("pending");
  expect(stalled).not.toHaveProperty("error");
  mocks.setStatus("example-tracker", "main", "applied");
  const applied = await (await fetch(url)).json();
  expect(applied.live.crons).toEqual(["0 6 * * *"]);
  expect(applied.status).toBe("applied");
  expect(applied.live).not.toHaveProperty("revision");
  expect(applied).not.toHaveProperty("applied");
  expect(applied.live.observed_at).toEqual(expect.any(String));
  const recorded = { crons: ["0 3 * * *"], suspended: false, timezone: "UTC" };
  const summary = mocks.summary("example-tracker", "main", recorded);
  expect(summary.schedule).toEqual(recorded);
  expect(summary.schedule_request).toEqual({
    crons: ["0 6 * * *"],
    revision: 1,
    read_error: null,
    last_observation: {
      status: "applied",
      observed_at: expect.any(String),
    },
  });
});

it("mocks invalid requests, concurrent edits and reset", async () => {
  const state = mocks.read("example-tracker", "main");
  const send = (method: string) =>
    fetch(url, {
      method,
      headers: {
        "If-Match": `"${state.revision}"`,
        "Content-Type": "application/json",
      },
      body:
        method === "PUT" ? JSON.stringify({ crons: ["0 8 * * *"] }) : undefined,
    });
  mocks.failNext("example-tracker", "main", 400);
  expect((await send("PUT")).status).toBe(400);
  mocks.failNext("example-tracker", "main", 412);
  expect((await send("PUT")).status).toBe(412);
  expect(state.requested?.crons).toEqual(["0 9 * * *"]);
  expect((await send("DELETE")).status).toBe(200);
  expect(state.requested?.crons).toEqual([]);
  expect(state.status).toBe("pending");
});

it("uses exactly the schedule enums published by the connectors service", () => {
  const statuses: Record<ScheduleStatus, true> = {
    applied: true,
    pending: true,
    inherited: true,
    paused: true,
  };
  const reasons: Record<SchedulePendingReason, true> = {
    no_cronworkflow: true,
    awaiting_reconcile: true,
    different_schedule: true,
  };
  const origins: Record<ScheduleOrigin, true> = {
    api: true,
    annotation: true,
    descriptor: true,
    default: true,
  };
  const schemas = openapi.components.schemas;
  expect(Object.keys(statuses).sort()).toEqual(
    [...schemas.ScheduleStatus.enum].sort()
  );
  expect(Object.keys(reasons).sort()).toEqual(
    [...schemas.PendingReason.enum].sort()
  );
  expect(Object.keys(origins).sort()).toEqual(
    [...schemas.ScheduleOrigin.enum].sort()
  );
});

it("returns contract fields and UUID attribution in schedule and health responses", async () => {
  const schemas = openapi.components.schemas;
  const fields = (value: object, schema: { properties: object }) => {
    expect(Object.keys(value).sort()).toEqual(
      Object.keys(schema.properties).sort()
    );
  };
  const initial = await (await fetch(url)).json();
  expect(initial.next_reconcile_at).toBeNull();
  expect(initial.reconcile_interval_secs).toBeNull();
  expect(initial).not.toHaveProperty("next_attempt_at");
  fields(initial, schemas.ScheduleResponse);
  fields(initial.live, schemas.LiveSchedule);
  fields(initial.inherited, schemas.InheritedSchedule);
  expect(schemas.ScheduleOrigin.enum).toContain(initial.inherited.origin);
  const saved = await fetch(url, {
    method: "PUT",
    headers: { "If-Match": '"0"', "Content-Type": "application/json" },
    body: JSON.stringify({ crons: ["0 6 * * *"] }),
  });
  const pending = await saved.json();
  expect(pending.next_reconcile_at).toEqual(expect.any(String));
  fields(pending.requested, schemas.RequestedSchedule);
  expect(pending.requested.updated_by).toMatch(
    /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i
  );
  expect(schemas.PendingReason.enum).toContain(pending.pending_reason);
  for (const status of ["inherited", "pending", "applied", "paused"] as const) {
    mocks.setStatus("example-tracker", "main", status);
    const response = await fetch(url);
    const state = await response.json();
    expect(state.next_reconcile_at).toEqual(
      status === "pending" ? expect.any(String) : null
    );
    fields(state, schemas.ScheduleResponse);
    expect(schemas.ScheduleStatus.enum).toContain(state.status);
    expect(schemas.ScheduleOrigin.enum).toContain(state.live.origin);
    const summary = mocks.summary("example-tracker", "main", null);
    fields(summary.schedule_request!, schemas.HealthScheduleRequest);
    fields(
      summary.schedule_request!.last_observation!,
      schemas.ScheduleObservation
    );
    expect(summary.schedule_request!.last_observation!.status).toBe(status);
  }
});

it.each(["PUT", "DELETE"])(
  "%s refusals use problem responses and stale revisions carry the current ETag",
  async (method) => {
    const headers = { "Content-Type": "application/json" };
    const body =
      method === "PUT" ? JSON.stringify({ crons: ["0 6 * * *"] }) : undefined;
    const missing = await fetch(url, { method, headers, body });
    expect(missing.status).toBe(428);
    const stale = await fetch(url, {
      method,
      body,
      headers: { ...headers, "If-Match": '"7"' },
    });
    expect(stale.status).toBe(412);
    expect(stale.headers.get("ETag")).toBe('"0"');
    expect(stale.headers.get("Content-Type")).toBe("application/problem+json");
    const problem = await stale.json();
    expect(problem.context.revision).toBe(0);
    expect(Object.keys(problem).sort()).toEqual(
      [...openapi.components.schemas.Problem.required].sort()
    );
  }
);

it.each(["PUT", "DELETE"])(
  "a refused %s preserves the exact schedule request",
  async (method) => {
    const before = structuredClone(mocks.read("example-tracker", "main"));
    mocks.failNext("example-tracker", "main", 503);
    const response = await fetch(url, {
      method,
      headers: { "If-Match": '"0"', "Content-Type": "application/json" },
      ...(method === "PUT"
        ? { body: JSON.stringify({ crons: ["0 6 * * *"] }) }
        : {}),
    });
    expect(response.status).toBe(503);
    expect((await response.json()).detail).toContain(
      "schedule was not changed"
    );
    expect(mocks.read("example-tracker", "main")).toEqual(before);
  }
);

it("a Reset without the deployment annotation preserves the request", async () => {
  const state = mocks.read("example-tracker", "main");
  state.inherited = null;
  const before = structuredClone(state);
  const response = await fetch(url, {
    method: "DELETE",
    headers: { "If-Match": '"0"' },
  });
  expect(response.status).toBe(409);
  expect((await response.json()).detail).toContain(
    "Reset requires the deployment schedule annotation"
  );
  expect(state).toEqual(before);
});

it("a paused job stays Paused after a saved change and in the health summary", async () => {
  const state = mocks.read("example-tracker", "main");
  state.live!.suspended = true;
  const response = await fetch(url, {
    method: "PUT",
    headers: { "If-Match": '"0"', "Content-Type": "application/json" },
    body: JSON.stringify({ crons: ["0 6 * * *"] }),
  });
  expect(response.status).toBe(200);
  expect((await response.json()).status).toBe("paused");
  expect(
    mocks.summary("example-tracker", "main", state.live ?? null)
      .schedule_request?.last_observation?.status
  ).toBe("paused");
});
