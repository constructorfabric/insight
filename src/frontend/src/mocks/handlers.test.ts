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

it("names each person's manager in the people mock", async () => {
  const response = await fetch(
    new URL("/api/identity/v1/people?q=bob", window.location.href)
  );
  const body = (await response.json()) as {
    items: Array<{ display_name: string; manager_name: string | null }>;
  };

  expect(body.items).toContainEqual(
    expect.objectContaining({ display_name: "Bob Park", manager_name: "Carol Chen" })
  );
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

const SOURCE_ID = "01900000-0000-7000-8000-00000000aa02";
const PERSON_ID = "e8a33e91-2658-58dc-8175-ebf473d8be5c";

function wireAccount(id: string) {
  return { source: "gitlab", source_id: SOURCE_ID, id };
}

async function post(path: string, body: unknown) {
  const response = await fetch(
    new URL(`/api/identity/v1/resolution/${path}`, window.location.href),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    }
  );
  return (await response.json()) as {
    items: Array<{ account_id: string; outcome: string }>;
  };
}

async function queuedIds() {
  const response = await fetch(
    new URL("/api/identity/v1/resolution/attention", window.location.href)
  );
  const body = (await response.json()) as {
    items: Array<{ account_id: string }>;
  };
  return body.items.map((item) => item.account_id);
}

it("decides the account a bind names and leaves the rest queued", async () => {
  const answer = await post("bind", {
    bindings: [{ account: wireAccount("pm.lee"), person_id: PERSON_ID }],
    comment: "",
  });

  expect(answer.items.map((item) => item.account_id)).toEqual(["pm.lee"]);
  const queued = await queuedIds();
  expect(queued).not.toContain("pm.lee");
  expect(queued).toContain("dev-42");
});

it("decides every account of a multi-binding bind", async () => {
  const answer = await post("bind", {
    bindings: [
      { account: wireAccount("shared-ci"), person_id: PERSON_ID },
      { account: wireAccount("build-agent"), person_id: PERSON_ID },
    ],
    comment: "",
  });

  expect(answer.items.map((item) => item.account_id)).toEqual([
    "shared-ci",
    "build-agent",
  ]);
  const queued = await queuedIds();
  expect(queued).not.toContain("shared-ci");
  expect(queued).not.toContain("build-agent");
});

it("decides the account an exclude names", async () => {
  const answer = await post("exclude", {
    account: {
      source: "github",
      source_id: "01900000-0000-7000-8000-00000000aa01",
      id: "ci-bot-7",
    },
    comment: "",
  });

  expect(answer.items.map((item) => item.account_id)).toEqual(["ci-bot-7"]);
  expect(await queuedIds()).not.toContain("ci-bot-7");
});
