import { setupServer } from "msw/node";
import { afterAll, beforeAll, expect, it } from "vitest";

import { FEEDBACK_MESSAGE_MAX } from "@/api/feedback-client";

import { handlers } from "./handlers";

const server = setupServer(...handlers);

beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterAll(() => server.close());

it("rejects unsupported metric result entity types", async () => {
  const response = await fetch(
    new URL("/api/analytics/v1/metric-results", window.location.href),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        entity: { type: "other" },
        period: { from: "2026-01-01", to: "2026-01-31" },
        metrics: [],
      }),
    },
  );

  expect(response.status).toBe(400);
});

it("takes a feedback message the service takes: the budget counts characters", async () => {
  const response = await fetch(
    new URL("/api/analytics/v1/feedback", window.location.href),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        message: "🙂".repeat(FEEDBACK_MESSAGE_MAX),
        path: "/portal/overview",
      }),
    },
  );

  expect(response.status).toBe(204);
});

it("refuses a feedback message past the budget", async () => {
  const response = await fetch(
    new URL("/api/analytics/v1/feedback", window.location.href),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        message: "x".repeat(FEEDBACK_MESSAGE_MAX + 1),
        path: "/portal/overview",
      }),
    },
  );

  expect(response.status).toBe(400);
});
