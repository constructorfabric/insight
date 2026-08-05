import { http, HttpResponse } from "msw";

import type { MetricResultsRequest } from "@/api/metric-results-client";
import { isPersonId } from "@/lib/metrics/entity";

import { buildMetricResultsResponse } from "./metric-results-factory";
import { buildIdentityTree, PEOPLE, PEOPLE_BY_EMAIL } from "./registry";

const defaultPerson = PEOPLE[0];

// Stable synthetic session for mock/Storybook runs. The old in-code
// MOCKS_ENABLED viewer path is gone; an authenticated viewer now comes from
// the same `/auth/me` probe the real app uses, so the boot `loadSession()`
// call resolves to `authenticated` against these handlers.
const MOCK_SESSION = {
  // `user` is the person id the SPA keys on (the gateway JWT `sub`).
  user: defaultPerson?.person_id ?? "00000000-0000-0000-0000-0000000000bb",
  email: defaultPerson?.email ?? "bob.park@example.com",
  tenant_id: "00000000-0000-0000-0000-000000000001",
  roles: ["user"],
  // Required by loadSession's fail-closed guard (a live session always has one).
  csrf_token: "mock-csrf-token",
};

// Session timing for the refresh driver: mirror the backend defaults
// (ttl 600 s, refresh ~90 s before expiry) relative to "now".
function mockSessionTiming(): { expires_at: number; refresh_at: number } {
  const now = Math.floor(Date.now() / 1000);
  return { expires_at: now + 600, refresh_at: now + 510 };
}

export const handlers = [
  http.get("/auth/me", () =>
    HttpResponse.json({ ...MOCK_SESSION, ...mockSessionTiming() }),
  ),
  http.post("/auth/refresh", () => HttpResponse.json(mockSessionTiming())),
  http.post("/auth/logout", () => HttpResponse.json({ rp_logout_url: null })),
  http.post("/api/analytics/v1/metric-results", async ({ request }) => {
    const body = (await request
      .json()
      .catch(() => null)) as MetricResultsRequest | null;
    if (
      !body ||
      !Array.isArray(body.entity?.ids) ||
      !Array.isArray(body.metrics)
    ) {
      return HttpResponse.json({ error: "invalid_argument" }, { status: 400 });
    }
    // Mirror the real endpoint since the identity cutover: entity ids are
    // person UUIDs and an email is a 400. Without this the mock would happily
    // answer a stale email fixture and hide the very regression it exists to
    // catch.
    if (!body.entity.ids.every((id) => typeof id === "string" && isPersonId(id))) {
      return HttpResponse.json(
        { error: "invalid_argument", field: "entity.ids" },
        { status: 400 },
      );
    }
    return HttpResponse.json(buildMetricResultsResponse(body));
  }),
  http.post(
    "/api/identity/v1/profiles",
    async ({ request }) => {
      const body = (await request.json().catch(() => null)) as
        | { value_type?: string; value?: string }
        | null;
      const value = (body?.value ?? "").trim();
      // The service resolves `person_id` (the SPA's key) and `email` (legacy
      // URL migration only); anything else is a client error.
      if (body?.value_type !== "email" && body?.value_type !== "person_id") {
        return HttpResponse.json(
          { type: "urn:insight:error:invalid_argument" },
          { status: 400 },
        );
      }
      // A malformed person_id is a 400, not a 404 — matching the service, where
      // "does not parse" and "resolves to nobody" are different answers.
      if (body.value_type === "person_id" && !isPersonId(value)) {
        return HttpResponse.json(
          { type: "urn:insight:error:invalid_argument" },
          { status: 400 },
        );
      }
      const personId =
        body.value_type === "email"
          ? PEOPLE_BY_EMAIL[value.toLowerCase()]?.person_id
          : value.toLowerCase();
      if (!personId) {
        return HttpResponse.json(
          { type: "urn:insight:error:person_not_found" },
          { status: 404 },
        );
      }
      const tree = buildIdentityTree(personId);
      if (!tree) {
        return HttpResponse.json(
          { type: "urn:insight:error:person_not_found" },
          { status: 404 },
        );
      }
      return HttpResponse.json(tree);
    },
  ),
  ...savedQueryHandlers(),
];

// ── Saved queries (`/v1/queries`) ────────────────────────────
// A tiny in-memory store so the console's CRUD + run round-trip in mock,
// Storybook, and `VITE_ENABLE_MOCKS=true` dev runs. Synthetic data only.

interface MockSavedQuery {
  id: string;
  insight_tenant_id: string;
  name: string;
  description: string | null;
  sql: string;
  created_at: string;
  updated_at: string;
}

const QUERIES_BASE = "/api/analytics/v1/queries";

const savedQueryStore = new Map<string, MockSavedQuery>();

(function seedSavedQueries() {
  const now = "2026-07-01T00:00:00Z";
  const seed: MockSavedQuery = {
    id: "11111111-1111-1111-1111-111111111111",
    insight_tenant_id: MOCK_SESSION.tenant_id,
    name: "Commits by tool",
    description: "Synthetic sample over the contract.",
    sql: "SELECT tool, commits FROM example ORDER BY commits DESC",
    created_at: now,
    updated_at: now,
  };
  savedQueryStore.set(seed.id, seed);
})();

function savedQueryHandlers() {
  return [
    http.get(QUERIES_BASE, () =>
      HttpResponse.json({
        items: [...savedQueryStore.values()].map((q) => ({
          id: q.id,
          name: q.name,
          description: q.description,
        })),
      }),
    ),
    http.post(QUERIES_BASE, async ({ request }) => {
      const body = (await request.json().catch(() => null)) as {
        name?: string;
        description?: string | null;
        sql?: string;
      } | null;
      if (!body?.name || !body?.sql) {
        return HttpResponse.json({ error: "invalid_argument" }, { status: 400 });
      }
      const now = new Date().toISOString();
      const created: MockSavedQuery = {
        id: crypto.randomUUID(),
        insight_tenant_id: MOCK_SESSION.tenant_id,
        name: body.name,
        description: body.description ?? null,
        sql: body.sql,
        created_at: now,
        updated_at: now,
      };
      savedQueryStore.set(created.id, created);
      return HttpResponse.json(created, { status: 201 });
    }),
    http.get(`${QUERIES_BASE}/:id`, ({ params }) => {
      const found = savedQueryStore.get(String(params.id));
      return found
        ? HttpResponse.json(found)
        : HttpResponse.json({ error: "not_found" }, { status: 404 });
    }),
    http.put(`${QUERIES_BASE}/:id`, async ({ params, request }) => {
      const existing = savedQueryStore.get(String(params.id));
      if (!existing) {
        return HttpResponse.json({ error: "not_found" }, { status: 404 });
      }
      const body = (await request.json().catch(() => ({}))) as {
        name?: string;
        description?: string | null;
        sql?: string;
      };
      const updated: MockSavedQuery = {
        ...existing,
        name: body.name ?? existing.name,
        description:
          body.description === undefined
            ? existing.description
            : body.description,
        sql: body.sql ?? existing.sql,
        updated_at: new Date().toISOString(),
      };
      savedQueryStore.set(updated.id, updated);
      return HttpResponse.json(updated);
    }),
    http.delete(`${QUERIES_BASE}/:id`, ({ params }) => {
      savedQueryStore.delete(String(params.id));
      return new HttpResponse(null, { status: 204 });
    }),
    http.post(`${QUERIES_BASE}/:id/run`, ({ params }) => {
      if (!savedQueryStore.has(String(params.id))) {
        return HttpResponse.json({ error: "not_found" }, { status: 404 });
      }
      return HttpResponse.json({
        rows: [
          { tool: "github", commits: 128 },
          { tool: "gitlab", commits: 74 },
          { tool: "bitbucket_cloud", commits: 39 },
        ],
      });
    }),
  ];
}
