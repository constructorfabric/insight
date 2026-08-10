import { http, HttpResponse } from "msw";

import type { MetricResultsRequest } from "@/api/metric-results-client";
import type {
  CustomMetric,
  CustomMetricGraph,
} from "@/api/metrics-client";
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
  ...customMetricHandlers(),
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

// ── Custom metrics (`/v1/metrics`) ────────────────────────────
// A tiny in-memory store so the metrics console's CRUD + export/import
// round-trip in mock, Storybook, and `VITE_ENABLE_MOCKS=true` dev runs.
// Synthetic data only.

const METRICS_BASE = "/api/analytics/v1/metrics";

const customMetricStore = new Map<string, CustomMetric>();

const SAMPLE_OBSERVATION_SQL =
  "SELECT tenant_id, source_key, entity_type, entity_id, metric_date, " +
  "measure_key, observed_at, value, subject_key, dimensions FROM example_source";

function seedCustomMetric(graph: CustomMetricGraph): void {
  customMetricStore.set(graph.metric_key, { ...graph, origin: "custom" });
}

(function seedCustomMetrics() {
  seedCustomMetric({
    metric_key: "example.accepted_lines",
    label: "Accepted lines",
    short_label: "Lines",
    description: "Synthetic sample custom metric over the contract.",
    explanation: null,
    entity_type: "person",
    unit: "lines",
    format: "integer",
    direction: "higher_is_better",
    computation: "sum",
    scale: null,
    peer_cohort_key: null,
    transform: null,
    source_key: "example_source",
    observation_sql: SAMPLE_OBSERVATION_SQL,
    measures: ["accepted_lines"],
    dimensions: ["repo", "language"],
    inputs: [{ role: "value", measure_key: "accepted_lines" }],
  });
})();

function toSummary(metric: CustomMetric) {
  return {
    metric_key: metric.metric_key,
    label: metric.label,
    computation: metric.computation,
    entity_type: metric.entity_type,
  };
}

function stripOrigin(metric: CustomMetric): CustomMetricGraph {
  const { origin: _origin, ...graph } = metric;
  return graph;
}

/** A graph is well-formed enough to persist: identity, source, SQL, at least
 *  one measure, and the input wiring its computation requires. Mirrors the
 *  backend's create/update validation so FE tests exercise real behavior. */
function isValidGraph(graph: CustomMetricGraph | null): graph is CustomMetricGraph {
  if (
    !graph?.metric_key ||
    !graph.label ||
    !graph.source_key ||
    !graph.observation_sql ||
    !Array.isArray(graph.measures) ||
    graph.measures.length === 0 ||
    !Array.isArray(graph.inputs) ||
    graph.inputs.length === 0
  ) {
    return false;
  }
  if (graph.computation === "ratio") {
    const roles = new Set(graph.inputs.map((input) => input.role));
    return (
      roles.has("numerator") &&
      roles.has("denominator") &&
      typeof graph.scale === "number"
    );
  }
  return graph.inputs.some((input) => input.role === "value");
}

/** True when `source_key` already belongs to a DIFFERENT stored metric. The
 *  backend rejects such an update/create with 409. */
function sourceKeyTakenByOther(sourceKey: string, metricKey: string): boolean {
  for (const metric of customMetricStore.values()) {
    if (metric.metric_key !== metricKey && metric.source_key === sourceKey) {
      return true;
    }
  }
  return false;
}

function customMetricHandlers() {
  return [
    http.get(METRICS_BASE, () =>
      HttpResponse.json({
        items: [...customMetricStore.values()].map(toSummary),
      }),
    ),
    http.post(METRICS_BASE, async ({ request }) => {
      const body = (await request
        .json()
        .catch(() => null)) as CustomMetricGraph | null;
      if (!isValidGraph(body)) {
        return HttpResponse.json({ error: "invalid_argument" }, { status: 400 });
      }
      // A duplicate key is a conflict, not an overwrite — leave the store
      // untouched and mirror the backend's 409.
      if (customMetricStore.has(body.metric_key)) {
        return HttpResponse.json({ error: "already_exists" }, { status: 409 });
      }
      if (sourceKeyTakenByOther(body.source_key, body.metric_key)) {
        return HttpResponse.json({ error: "source_key_conflict" }, { status: 409 });
      }
      const created: CustomMetric = { ...body, origin: "custom" };
      customMetricStore.set(created.metric_key, created);
      return HttpResponse.json(created, { status: 201 });
    }),
    // Static sub-paths must precede the `:metricKey` param route so they are
    // not captured as a metric key.
    http.get(`${METRICS_BASE}/export`, () =>
      HttpResponse.json({
        metrics: [...customMetricStore.values()].map(stripOrigin),
      }),
    ),
    http.post(`${METRICS_BASE}/import`, async ({ request }) => {
      const body = (await request.json().catch(() => null)) as {
        metrics?: CustomMetricGraph[];
      } | null;
      // Import is all-or-nothing: validate the whole batch first and mutate
      // nothing if any member is malformed. Only after the batch is known good
      // do we apply it, skipping keys that already exist.
      if (!Array.isArray(body?.metrics) || !body.metrics.every(isValidGraph)) {
        return HttpResponse.json({ error: "invalid_argument" }, { status: 400 });
      }
      const skipped: string[] = [];
      let imported = 0;
      for (const graph of body.metrics) {
        if (customMetricStore.has(graph.metric_key)) {
          skipped.push(graph.metric_key);
          continue;
        }
        customMetricStore.set(graph.metric_key, { ...graph, origin: "custom" });
        imported += 1;
      }
      return HttpResponse.json({ imported, skipped });
    }),
    http.get(`${METRICS_BASE}/:metricKey`, ({ params }) => {
      const found = customMetricStore.get(String(params.metricKey));
      return found
        ? HttpResponse.json(found)
        : HttpResponse.json({ error: "not_found" }, { status: 404 });
    }),
    http.put(`${METRICS_BASE}/:metricKey`, async ({ params, request }) => {
      const key = String(params.metricKey);
      if (!customMetricStore.has(key)) {
        return HttpResponse.json({ error: "not_found" }, { status: 404 });
      }
      const body = (await request
        .json()
        .catch(() => null)) as CustomMetricGraph | null;
      const candidate = body ? { ...body, metric_key: key } : null;
      // Reject an incomplete/invalid graph instead of persisting it.
      if (!isValidGraph(candidate)) {
        return HttpResponse.json({ error: "invalid_argument" }, { status: 400 });
      }
      // A source_key already claimed by a different metric is a 409, matching
      // the backend's new collision check.
      if (sourceKeyTakenByOther(candidate.source_key, key)) {
        return HttpResponse.json({ error: "source_key_conflict" }, { status: 409 });
      }
      const updated: CustomMetric = { ...candidate, origin: "custom" };
      customMetricStore.set(key, updated);
      return HttpResponse.json(updated);
    }),
    http.delete(`${METRICS_BASE}/:metricKey`, ({ params }) => {
      customMetricStore.delete(String(params.metricKey));
      return new HttpResponse(null, { status: 204 });
    }),
  ];
}
