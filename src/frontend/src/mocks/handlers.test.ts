import { setupServer } from "msw/node";
import { afterAll, beforeAll, expect, it } from "vitest";

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
