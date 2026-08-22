import { http, HttpResponse } from "msw";

import { FEEDBACK_MESSAGE_MAX } from "@/api/feedback-client";
import type { MetricResultsRequest } from "@/api/metric-results-client";
import type {
  CustomMetric,
  CustomMetricGraph,
} from "@/api/metrics-client";
import { isPersonId } from "@/lib/metrics/entity";

import { buildMetricDefinitions } from "./metric-definitions-factory";
import { buildMetricResultsResponse } from "./metric-results-factory";
import { buildIdentityTree, PEOPLE, PEOPLE_BY_EMAIL } from "./registry";

const defaultPerson = PEOPLE[0];

/**
 * A mock page holds far fewer rows than a real one. The synthetic roster is
 * smaller than the console's page size, so honouring `?limit=` would put every
 * row on page one and leave "show more" unreachable in mock mode — the affordance
 * would have no dev or Storybook path at all.
 */
const MOCK_PAGE_SIZE = 8;

/**
 * One page of a listing, cursor and all — the mock pages the way the service
 * does so the console's "show more" is exercised in mock mode too. The cursor
 * carries the query it was issued for, and a mismatched one restarts, which is
 * the behaviour the real cursor enforces by refusing.
 */
function pageOf<T>(items: T[], params: URLSearchParams, query: string) {
  const limit = Math.min(Number(params.get("limit") ?? 20), MOCK_PAGE_SIZE);
  const cursor = params.get("cursor");
  const decoded = cursor ? JSON.parse(atob(cursor)) : null;
  const offset = decoded?.q === query ? Number(decoded.at) : 0;

  const slice = items.slice(offset, offset + limit);
  const next = offset + slice.length;
  const more = next < items.length;

  return {
    items: slice,
    next_cursor: more ? btoa(JSON.stringify({ q: query, at: next })) : null,
  };
}

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


// Assembled last: the sections above declare module-level stores/consts the
// handler factories close over, and the factories are CALLED right here —
// an earlier array literal hits the temporal dead zone (seen live as
// `Cannot access 'QUERIES_BASE' before initialization`).
/** Whoever the person mode has open holds the two accounts it lists for them. */
const HELD_BY = "2517cd48-4961-52b3-a401-b0e5a03858a4";

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
      !body.entity ||
      !Array.isArray(body.metrics)
    ) {
      return HttpResponse.json({ error: "invalid_argument" }, { status: 400 });
    }

    const entityType: unknown = (body.entity as { type?: unknown }).type;
    if (entityType !== "person" && entityType !== "tenant") {
      return HttpResponse.json(
        { error: "invalid_argument", field: "entity.type" },
        { status: 400 },
      );
    }

    // Mirror the real endpoint since the identity cutover: entity ids are
    // person UUIDs and an email is a 400. Without this the mock would happily
    // answer a stale email fixture and hide the very regression it exists to
    // catch.
    if (
      body.entity.type === "person" &&
      (!Array.isArray(body.entity.ids) ||
        !body.entity.ids.every((id) => typeof id === "string" && isPersonId(id)))
    ) {
      return HttpResponse.json(
        { error: "invalid_argument", field: "entity.ids" },
        { status: 400 },
      );
    }
    if (
      body.entity.type === "tenant" &&
      body.metrics.some((metric) =>
        metric.views.some((view) => view.view === "peer"),
      )
    ) {
      return HttpResponse.json(
        { error: "invalid_argument", field: "metrics.views" },
        { status: 400 },
      );
    }
    return HttpResponse.json(buildMetricResultsResponse(body));
  }),
  // The demo viewer is an identity admin, so mock mode exercises the admin
  // surfaces (Manage → Identities). The role id is the backend's seeded
  // `roles_repo::ADMIN_ROLE_ID` migration constant.
  http.get("/api/identity/v1/me", () =>
    HttpResponse.json({
      person_id: defaultPerson?.person_id ?? "00000000-0000-0000-0000-0000000000bb",
      insight_tenant_id: "00000000-0000-4000-8000-00000000c0de",
      roles: [
        {
          role_id: "a4d11000-0000-4000-8000-000000000001",
          name: "admin",
        },
      ],
      // Mock mode demos the reporting-line product; the flat roster below is
      // served anyway, so switching this to "flat" exercises that shell.
      visibility_policy: "org_chart",
    }),
  ),
  // Minimal, honest empty catalog: without this handler the request falls
  // through to the network, and in a proxy-configured dev run the resulting
  // 401 bounces the whole mock session to the real IdP.
  http.get("/api/analytics/v1/metric-definitions", () =>
    HttpResponse.json({ metrics: buildMetricDefinitions() }),
  ),
  // One account's binding + decision trail. dev-42 carries a small history so
  // the panel has something to show; any other account answers 200 with no
  // binding and no history — the real endpoint has no 404: an account nobody
  // ever observed or decided reads as an empty journal, and THAT is what a
  // stale shared link lands on. Timestamps are zone-less UTC on the wire
  // (.NET parity) — a `Z` here would train the panel on a shape production
  // never sends.
  http.get(
    "/api/identity/v1/resolution/accounts/:source/:sourceId/:accountId",
    ({ params }) => {
      // The roster mint: bound, by the batch, with nothing but its own
      // creation on the trail — the state an operator is asked to confirm.
      if (params.accountId === "874") {
        const minted = {
          person_id: "01900000-0000-7000-8000-0000000000d0",
          display_name: "Ravi Menon",
          job_title: "Facilities Lead",
        };
        return HttpResponse.json({
          source: params.source,
          source_id: params.sourceId,
          account_id: params.accountId,
          person_id: minted.person_id,
          history: [
            {
              person_id: minted.person_id,
              // No `provisional` here: the server builds trail cards from the
              // journal alone and never marks them, so claiming it would have
              // the console verified against a shape it will not receive.
              person: minted,
              author_person_id: "00000000-0000-0000-0000-000000000000",
              by_operator: false,
              reason: "roster-mint",
              recorded_at: "2026-08-14T06:30:00.000000",
            },
          ],
          operations: [],
        });
      }
      // The two accounts the person listing above claims for whoever is open:
      // reporting them as unbound here would have the console demonstrate a
      // state the service cannot produce — an account in a person's own list
      // that the binding read says nobody holds.
      if (params.accountId === "gh-main" || params.accountId === "gl-alt") {
        return HttpResponse.json({
          source: params.source,
          source_id: params.sourceId,
          account_id: params.accountId,
          person_id: HELD_BY,
          history: [
            {
              person_id: HELD_BY,
              author_person_id: "00000000-0000-0000-0000-000000000000",
              by_operator: params.accountId === "gl-alt",
              reason: "seed",
              recorded_at: "2026-08-14T06:30:00.000000",
            },
          ],
          operations: [],
        });
      }
      if (params.accountId !== "dev-42") {
        return HttpResponse.json({
          source: params.source,
          source_id: params.sourceId,
          account_id: params.accountId,
          person_id: null,
          history: [],
        });
      }
      const [bob, carol] = PEOPLE;
      return HttpResponse.json({
        source: params.source,
        source_id: params.sourceId,
        account_id: params.accountId,
        person_id: bob?.person_id,
        history: [
          {
            person_id: bob?.person_id,
            person: bob
              ? {
                  person_id: bob.person_id,
                  email: bob.email,
                  display_name: bob.name,
                  job_title: bob.role,
                }
              : null,
            author_person_id: carol?.person_id,
            author: carol
              ? {
                  person_id: carol.person_id,
                  email: carol.email,
                  display_name: carol.name,
                  job_title: carol.role,
                }
              : null,
            by_operator: true,
            reason: "operator-bind",
            recorded_at: "2026-08-01T10:15:00.000000",
          },
          {
            person_id: carol?.person_id,
            author_person_id: "00000000-0000-0000-0000-000000000000",
            by_operator: false,
            // The resolver's own rows carry an EMPTY reason, never null —
            // mirroring the real column, which a nullish fallback misses.
            reason: "",
            recorded_at: "2026-07-15T08:00:00.000000",
          },
          {
            person_id: carol?.person_id,
            author_person_id: "00000000-0000-0000-0000-000000000000",
            by_operator: false,
            reason: "login-bootstrap",
            recorded_at: "2026-07-01T06:30:00.000000",
          },
        ],
        // The call behind the operator's row above: who ran it, how far it
        // reached, and the one thing no other record holds — why.
        operations: [
          {
            operation_id: "01900000-0000-7000-8000-0000000000f1",
            verb: "operator-bind",
            author_person_id: carol?.person_id,
            author: carol
              ? {
                  person_id: carol.person_id,
                  email: carol.email,
                  display_name: carol.name,
                  job_title: carol.role,
                }
              : null,
            comment: "Checked with HR — same person, the chat handle is theirs.",
            accounts_touched: 3,
            outcome: "applied",
            recorded_at: "2026-08-01T10:15:00.000000",
          },
        ],
      });
    },
  ),
  // The person listing: a blank query is the whole roster, terms narrow it,
  // and both are paged the way the service pages them.
  http.get("/api/identity/v1/persons", ({ request }) => {
    const params = new URL(request.url).searchParams;
    const q = params.get("q")?.trim() ?? "";
    const terms = q ? q.toLowerCase().split(/\s+/) : [];
    // A term that parses as an id names a person, mirroring the service: it is
    // the only way to reach someone the journal holds no values for.
    const items = PEOPLE.filter((p) =>
      terms.every((term) =>
        isPersonId(term)
          ? p.person_id.toLowerCase() === term
          : [p.name, p.email, p.role].some((v) => v.toLowerCase().includes(term)),
      ),
    )
      .map((p) => ({
        person_id: p.person_id,
        email: p.email,
        username: null,
        display_name: p.name,
        job_title: p.role,
        status: "active",
      }))
      .sort((left, right) =>
        (left.display_name ?? "").localeCompare(right.display_name ?? ""),
      );
    return HttpResponse.json(pageOf(items, params, q));
  }),
  // Who the caller may see. Mock mode has one tenant and one roster, so this
  // answers the same people the operator listing does — the difference on a real
  // stand is the visible-set filter, which a mock cannot have an opinion about.
  http.get("/api/identity/v1/visible-persons", ({ request }) => {
    const params = new URL(request.url).searchParams;
    const q = params.get("q")?.trim().toLowerCase() ?? "";
    const items = PEOPLE.filter(
      (p) =>
        !q ||
        [p.name, p.email, p.role].some((v) => v.toLowerCase().includes(q)),
    )
      .map((p) => ({
        person_id: p.person_id,
        email: p.email,
        username: null,
        display_name: p.name,
        job_title: p.role,
        status: "active",
      }))
      .sort((left, right) =>
        (left.display_name ?? "").localeCompare(right.display_name ?? ""),
      );
    return HttpResponse.json(pageOf(items, params, q));
  }),
  // The account listing: the same roster seen as accounts; blank lists them all.
  http.get("/api/identity/v1/resolution/accounts", ({ request }) => {
    const params = new URL(request.url).searchParams;
    const q = params.get("q")?.trim() ?? "";
    const needle = q.toLowerCase();
    const items = PEOPLE.filter(
      (p) => !needle || [p.name, p.email].some((v) => v.toLowerCase().includes(needle)),
    ).map((p, index) => ({
      source: index % 2 === 0 ? "github" : "gitlab",
      source_id: "01900000-0000-7000-8000-00000000aa01",
      account_id: `acct-${index + 1}`,
      email: p.email,
      username: null,
      display_name: p.name,
      person: {
        person_id: p.person_id,
        email: p.email,
        display_name: p.name,
        job_title: p.role,
      },
      bound_by_operator: index % 3 === 0,
    }));
    // The service orders by the label each row shows; the mock mirrors it so a
    // mock run does not demonstrate an order the real listing never produces.
    items.sort((left, right) =>
      (left.email ?? left.account_id).localeCompare(right.email ?? right.account_id),
    );
    return HttpResponse.json(pageOf(items, params, q));
  }),
  // A merge preview's substance: two synthetic accounts for anyone.
  http.get(
    "/api/identity/v1/resolution/persons/:personId/accounts",
    ({ params }) =>
      HttpResponse.json({
        person_id: params.personId,
        accounts: [
          {
            source: "github",
            source_id: "01900000-0000-7000-8000-00000000aa01",
            account_id: "gh-main",
            email: "main@example.com",
            username: "gh-main",
            bound_by_operator: false,
          },
          {
            source: "gitlab",
            source_id: "01900000-0000-7000-8000-00000000aa02",
            account_id: "gl-alt",
            email: null,
            username: "gl-alt",
            bound_by_operator: true,
          },
        ],
      }),
  ),
  // The four correction verbs: happy-path outcomes, no state kept — the queue
  // mock is static, so the demo shows the flow rather than a simulation.
  ...["bind", "merge", "detach", "exclude"].map((verb) =>
    http.post(`/api/identity/v1/resolution/${verb}`, () =>
      HttpResponse.json({
        applied: 1,
        already_decided: 0,
        items: [
          {
            source: "github",
            source_id: "01900000-0000-7000-8000-00000000aa01",
            account_id: "dev-42",
            outcome: "applied",
          },
        ],
        ...(verb === "detach"
          ? { new_person_id: "01900000-0000-7000-8000-00000000dead" }
          : {}),
      }),
    ),
  ),
  // The review queue, exercising all three kinds. Candidates reuse the seeded
  // roster so names/emails stay consistent with every other mock surface.
  http.get("/api/identity/v1/resolution/attention", () => {
    const [bob, carol, alice] = PEOPLE;
    const card = (p: (typeof PEOPLE)[number], extra?: object) => ({
      person_id: p.person_id,
      email: p.email,
      username: null,
      display_name: p.name,
      job_title: p.role ?? null,
      status: "active",
      ...extra,
    });
    return HttpResponse.json({
      items: [
        {
          kind: "contested",
          source: "github",
          source_id: "01900000-0000-7000-8000-00000000aa01",
          account_id: "dev-42",
          email: "dev42@example.com",
          username: "dev42",
          // Contested means unbound: nobody holds it, which is why two people
          // can claim it.
          bound_to: null,
          candidates: [card(bob), card(carol)],
        },
        {
          kind: "binding_conflict",
          source: "gitlab",
          source_id: "01900000-0000-7000-8000-00000000aa02",
          account_id: "a.kim",
          email: alice?.email ?? "alice.kim@example.com",
          username: null,
          bound_to: alice?.person_id,
          candidates: [card(alice)],
        },
        {
          kind: "no_evidence",
          source: "github",
          source_id: "01900000-0000-7000-8000-00000000aa01",
          account_id: "ci-bot-7",
          email: null,
          username: "ci-bot-7",
          candidates: [],
        },
        {
          // Minted during a sign-in so its owner could get in: bound, and
          // still nobody's decision. It may duplicate a person the roster
          // already knows, which only an operator can settle.
          kind: "provisioned_at_login",
          source: "github",
          source_id: "01900000-0000-7000-8000-00000000aa01",
          account_id: "new-joiner",
          email: null,
          username: "new-joiner",
          bound_to: carol?.person_id,
          candidates: [card(carol, { provisional: true })],
        },
        {
          // Added because the roster lists the account, not because anything
          // matched: no address, so the person may already be on the roster
          // under a different account. Bound, and still nobody's decision.
          kind: "minted_from_roster",
          source: "hr",
          source_id: "01900000-0000-7000-8000-00000000aa03",
          account_id: "874",
          email: null,
          username: null,
          display_name: "Ravi Menon",
          job_title: "Facilities Lead",
          department: "Operations",
          status: "Active",
          manager_email: "carol.chen@example.com",
          bound_to: "01900000-0000-7000-8000-0000000000d0",
          candidates: [
            {
              person_id: "01900000-0000-7000-8000-0000000000d0",
              email: null,
              username: null,
              display_name: "Ravi Menon",
              job_title: "Facilities Lead",
              status: "active",
              // Minted for this very account, so nothing else is known about
              // them and they may be someone the roster already lists.
              provisional: true,
            },
          ],
        },
        {
          // Neither address nor handle — nothing automation can match on. The
          // source still describes the human, which is what the fold reads for
          // the operator and what makes this row bindable by hand.
          kind: "no_evidence",
          source: "hr",
          source_id: "01900000-0000-7000-8000-00000000aa03",
          account_id: "921",
          email: null,
          username: null,
          display_name: "Nadia Orlov",
          job_title: "Office Manager",
          department: "Operations",
          status: "Inactive",
          manager_email: "carol.chen@example.com",
          candidates: [],
        },
      ],
      rates: { observed: 60, bound: 55, pending: 3, no_source_id: 0, no_evidence: 2, excluded: 1 },
      truncated: false,
      items_truncated: false,
    });
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
  ...usageHandlers(),
  ...feedbackHandlers(),
];

// ── Product feedback (`/v1/feedback`) ──────────────────────────
// An in-memory store, so the dialog's send and the usage surface's listing
// round-trip in mock, Storybook, and `VITE_ENABLE_MOCKS=true` dev runs.

interface MockFeedback {
  feedback_id: string;
  ts: string;
  person_id: string;
  display_name: string;
  username: string;
  message: string;
  path: string;
}

/** Mirrors the service's own cap. */
const MOCK_FEEDBACK_LIMIT = 200;

function mockSender(person: (typeof PEOPLE)[number] | undefined) {
  return {
    person_id: person?.person_id ?? "",
    display_name: person?.name ?? "",
    username: person?.email.split("@")[0] ?? "",
  };
}

const feedbackStore: MockFeedback[] = [
  {
    feedback_id: "22222222-2222-2222-2222-222222222222",
    ts: "2026-08-20 09:14:00",
    ...mockSender(PEOPLE[1]),
    message: "The cohort control does not say what it compares against.",
    path: "/portal/overview",
  },
  {
    feedback_id: "33333333-3333-3333-3333-333333333333",
    ts: "2026-08-19 16:02:00",
    ...mockSender(PEOPLE[2]),
    message: "Let me export the people table to a spreadsheet.",
    path: "/portal/people",
  },
];

function feedbackHandlers() {
  return [
    http.post("/api/analytics/v1/feedback", async ({ request }) => {
      const body = (await request.json().catch(() => null)) as {
        message?: string;
        path?: string;
      } | null;
      const message = body?.message?.trim();
      if (!message || [...message].length > FEEDBACK_MESSAGE_MAX) {
        return HttpResponse.json({ error: "invalid_argument" }, { status: 400 });
      }
      feedbackStore.unshift({
        feedback_id: `mock-${feedbackStore.length + 1}`,
        ts: new Date().toISOString().replace("T", " ").slice(0, 19),
        ...mockSender(defaultPerson),
        message,
        path: body?.path ?? "",
      });
      return new HttpResponse(null, { status: 204 });
    }),
    http.get("/api/analytics/v1/feedback", ({ request }) => {
      const params = new URL(request.url).searchParams;
      const since = params.get("since") ?? "";
      const until = params.get("until") ?? "";
      // Answering the window the caller asked for, as the service does: a mock
      // that ignores it hides every date-range regression from mock runs.
      const items = feedbackStore
        .filter((row) => {
          const day = row.ts.slice(0, 10);
          return (!since || day >= since) && (!until || day <= until);
        })
        .slice(0, MOCK_FEEDBACK_LIMIT);

      return HttpResponse.json({ since, until, items });
    }),
  ];
}

// ── Platform usage (`/v1/usage/*`) ─────────────────────────────

function syntheticDays(count: number) {
  const days = [];
  for (let i = count - 1; i >= 0; i -= 1) {
    const day = new Date();
    day.setUTCDate(day.getUTCDate() - i);
    days.push({
      day: day.toISOString().slice(0, 10),
      visits: 2 + ((i * 7) % 9),
      visitors: 1 + ((i * 3) % 5),
    });
  }
  return days;
}

function usageHandlers() {
  return [
    http.get("/api/analytics/v1/usage/config", () =>
      HttpResponse.json({ enabled: true }),
    ),
    http.post("/api/analytics/v1/usage/events", () =>
      new HttpResponse(null, { status: 204 }),
    ),
    http.get("/api/analytics/v1/usage/summary", () => {
      const by_day = syntheticDays(30);
      return HttpResponse.json({
        since: by_day[0]?.day ?? "",
        until: by_day.at(-1)?.day ?? "",
        totals: {
          visits: by_day.reduce((sum, d) => sum + d.visits, 0),
          visitors: 4,
          page_views: 214,
        },
        by_day,
        by_person: [
          {
            person_id: defaultPerson?.person_id ?? "",
            display_name: defaultPerson?.name ?? "",
            username: defaultPerson?.email.split("@")[0] ?? "",
            visits: 31,
            page_views: 96,
            last_seen: `${by_day.at(-1)?.day ?? ""} 09:12`,
          },
          {
            person_id: PEOPLE[1]?.person_id ?? "",
            display_name: PEOPLE[1]?.name ?? "",
            username: PEOPLE[1]?.email.split("@")[0] ?? "",
            visits: 18,
            page_views: 64,
            last_seen: `${by_day.at(-1)?.day ?? ""} 08:40`,
          },
          {
            person_id: PEOPLE[2]?.person_id ?? "",
            display_name: PEOPLE[2]?.name ?? "",
            username: PEOPLE[2]?.email.split("@")[0] ?? "",
            visits: 7,
            page_views: 54,
            last_seen: `${by_day.at(-2)?.day ?? ""} 17:05`,
          },
        ],
        by_event: [
          { event_name: "drill", target: "pr_cycle_time", opens: 34, people: 3 },
          { event_name: "drill", target: "review_load", opens: 21, people: 3 },
          { event_name: "drill", target: "ai_share", opens: 12, people: 2 },
          { event_name: "session_start", target: "", opens: 57, people: 4 },
        ],
        by_page: [
          { path: "/portal/overview", views: 88, visitors: 4 },
          { path: "/portal/people", views: 61, visitors: 3 },
          { path: "/portal/manage/metric-catalog", views: 42, visitors: 2 },
          { path: "/portal/manage/platform-usage", views: 23, visitors: 1 },
        ],
      });
    }),
  ];
}
