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
    }
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
    }
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
    }
  );

  expect(response.status).toBe(400);
});

it("filters the canonical people mock before paging", async () => {
  const response = await fetch(
    new URL("/api/identity/v1/people?q=carol", window.location.href)
  );
  const body = (await response.json()) as {
    items: Array<{ display_name: string }>;
  };

  expect(body.items.map((person) => person.display_name)).toEqual([
    "Carol Chen",
  ]);
});

it("returns canonical person detail from the people mock", async () => {
  const response = await fetch(
    new URL(
      "/api/identity/v1/people/e8a33e91-2658-58dc-8175-ebf473d8be5c",
      window.location.href
    )
  );
  const body = (await response.json()) as {
    person_id: string;
    display_name: string;
  };

  expect(body).toMatchObject({
    person_id: "e8a33e91-2658-58dc-8175-ebf473d8be5c",
    display_name: "Carol Chen",
  });
});
