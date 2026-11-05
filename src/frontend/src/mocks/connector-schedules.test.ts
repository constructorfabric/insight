import { setupServer } from "msw/node";
import { afterAll, afterEach, beforeAll, beforeEach, expect, it } from "vitest";

import { createConnectorScheduleMocks } from "./connector-schedules";

let mocks = createConnectorScheduleMocks();
const server = setupServer();
const url = new URL(
  "/api/analytics/v1/connectors/example-tracker/main/schedule",
  window.location.origin
).href;
beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
beforeEach(() => {
  mocks = createConnectorScheduleMocks();
  server.use(...mocks.handlers);
});
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

it("mocks pending, applied and error requests with in-memory revisions", async () => {
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
  mocks.setStatus("example-tracker", "main", "error");
  const stalled = await (await fetch(url)).json();
  expect(stalled.status).toBe("error");
  expect(stalled.error).toEqual({
    attempted_revision: 1,
    attempted_at: expect.any(String),
    message: "Synthetic schedule apply failure.",
  });
  mocks.setStatus("example-tracker", "main", "applied");
  const applied = await (await fetch(url)).json();
  expect(applied.applied.crons).toEqual(["0 6 * * *"]);
  expect(applied.status).toBe("applied");
  expect(applied.applied.revision).toBe(1);
  expect(applied.applied.observed_at).toEqual(expect.any(String));
  const recorded = { crons: ["0 3 * * *"], suspended: false, timezone: "UTC" };
  const summary = mocks.summary("example-tracker", "main", recorded);
  expect(summary.schedule).toEqual(recorded);
  expect(summary.schedule_request).toEqual({
    crons: ["0 6 * * *"],
    revision: 1,
    status: "applied",
    updated_at: expect.any(String),
    updated_by: expect.any(String),
    applied: applied.applied,
    inherited: { crons: ["0 3 * * *"], origin: "default" },
    error: null,
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
