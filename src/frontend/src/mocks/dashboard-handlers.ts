import { http, HttpResponse } from "msw";

import type {
  Dashboard,
  DashboardAccess,
  DashboardShare,
} from "@/api/custom-client";

import { isAdministrator } from "./admin-assignments";
import { PEOPLE } from "./registry";

const CALLER = PEOPLE[0]?.person_id ?? "";
const COLLEAGUE = PEOPLE[1]?.person_id ?? "";
const LEAVER = PEOPLE[2]?.person_id ?? "";
const UPDATED_AT = "2026-09-01T09:00:00.000Z";

interface MockDashboard {
  name: string;
  body: Dashboard;
  owner: string | null;
}

const dashboards: Record<string, MockDashboard> = {
  "delivery-overview": {
    name: "Delivery overview",
    body: { title: "Delivery overview", widgets: [] },
    owner: CALLER,
  },
  "review-health": {
    name: "Review health",
    body: { title: "Review health", widgets: [] },
    owner: COLLEAGUE,
  },
  "team-activity": {
    name: "Team activity",
    body: { title: "Team activity", widgets: [] },
    owner: null,
  },
  "release-notes": {
    name: "Release notes",
    body: { title: "Release notes", widgets: [] },
    owner: LEAVER,
  },
};

const shares = new Map<string, DashboardShare[]>([
  [
    "review-health",
    [{ person_id: CALLER, granted_by: COLLEAGUE, created_at: UPDATED_AT }],
  ],
]);

function accessOf(id: string): DashboardAccess | null {
  const dashboard = dashboards[id];
  if (!dashboard) return null;
  if (dashboard.owner === null) return "ownerless";
  if (dashboard.owner === CALLER) return "owner";
  const shared = shares.get(id)?.some((one) => one.person_id === CALLER);

  return shared ? "viewer" : null;
}

function isOrphaned(dashboard: MockDashboard): boolean {
  return dashboard.owner !== null && !isAdministrator(dashboard.owner);
}

function listed(id: string) {
  const dashboard = dashboards[id];
  const access = accessOf(id);
  if (!dashboard || !access) return null;

  return { id, name: dashboard.name, owner_person_id: dashboard.owner, access };
}

function page<T>(items: T[], query: URLSearchParams) {
  const offset = Number(query.get("offset") ?? 0);
  const limit = Number(query.get("limit") ?? 50);

  return {
    items: items.slice(offset, offset + limit),
    total: items.length,
    limit,
    offset,
  };
}

const notFound = () => new HttpResponse(null, { status: 404 });

const refusedTransfer = (kind: string, description: string) =>
  HttpResponse.json(
    {
      detail: "Failed precondition",
      context: { violations: [{ type: kind, subject: "owner", description }] },
    },
    { status: 409 }
  );

const ownerOnly = () =>
  HttpResponse.json(
    { detail: "Only the owner may share this dashboard." },
    { status: 403 }
  );

export const dashboardHandlers = [
  http.get("/api/v3/v1/dashboards", ({ request }) => {
    const query = new URL(request.url).searchParams;
    const search = (query.get("q") ?? "").toLowerCase();
    const owned = query.get("owned");
    const items = Object.keys(dashboards)
      .map(listed)
      .filter((item) => item !== null)
      .filter((item) => item.name.toLowerCase().includes(search))
      .filter(
        (item) =>
          (owned !== "me" || item.access === "owner") &&
          (owned !== "shared" || item.access === "viewer")
      );

    return HttpResponse.json(page(items, query));
  }),
  http.get("/api/v3/v1/orphaned-dashboards", ({ request }) => {
    const query = new URL(request.url).searchParams;
    const items = Object.entries(dashboards)
      .filter(([, dashboard]) => isOrphaned(dashboard))
      .map(([id, dashboard]) => ({
        id,
        name: dashboard.name,
        owner_person_id: dashboard.owner,
        updated_at: UPDATED_AT,
      }));

    return HttpResponse.json(page(items, query));
  }),
  http.get("/api/v3/v1/dashboards/:id", ({ params }) => {
    const id = String(params.id);
    const shown = listed(id);
    const dashboard = dashboards[id];
    if (!shown || !dashboard) return notFound();

    return HttpResponse.json({
      ...shown,
      body: dashboard.body,
      folder: null,
      tags: [],
      updated_at: UPDATED_AT,
    });
  }),
  http.get("/api/v3/v1/dashboards/:id/shares", ({ params }) => {
    const id = String(params.id);
    const access = accessOf(id);
    if (!access) return notFound();
    if (access !== "owner") return ownerOnly();

    return HttpResponse.json({ items: shares.get(id) ?? [] });
  }),
  http.put("/api/v3/v1/dashboards/:id/shares/:personId", ({ params }) => {
    const id = String(params.id);
    const personId = String(params.personId);
    const access = accessOf(id);
    if (!access) return notFound();
    if (access !== "owner") return ownerOnly();

    const held = shares.get(id) ?? [];
    if (!held.some((one) => one.person_id === personId)) {
      shares.set(id, [
        ...held,
        { person_id: personId, granted_by: CALLER, created_at: UPDATED_AT },
      ]);
    }
    return new HttpResponse(null, { status: 204 });
  }),
  http.delete("/api/v3/v1/dashboards/:id/shares/:personId", ({ params }) => {
    const id = String(params.id);
    const personId = String(params.personId);
    const access = accessOf(id);
    if (!access) return notFound();
    if (access !== "owner") return ownerOnly();

    shares.set(
      id,
      (shares.get(id) ?? []).filter((one) => one.person_id !== personId)
    );
    return new HttpResponse(null, { status: 204 });
  }),
  http.post("/api/v3/v1/dashboards/:id/transfer", async ({ params, request }) => {
    const dashboard = dashboards[String(params.id)];
    if (!dashboard) return notFound();
    if (dashboard.owner === null) {
      return refusedTransfer(
        "ownerless",
        "an ownerless dashboard cannot be shared or transferred"
      );
    }
    if (!isOrphaned(dashboard)) {
      return refusedTransfer(
        "owner_is_administrator",
        "the dashboard's owner is still an administrator"
      );
    }

    const { to } = (await request.json()) as { to: string };
    if (!isAdministrator(to)) {
      return HttpResponse.json(
        {
          detail: "Invalid argument",
          context: {
            field_violations: [
              {
                field: "to",
                description:
                  "a dashboard is transferred only to a current administrator",
              },
            ],
          },
        },
        { status: 400 }
      );
    }

    dashboard.owner = to;
    return new HttpResponse(null, { status: 204 });
  }),
  http.get("/api/v3/v1/pins", () => HttpResponse.json({ pins: [] })),
  http.get("/api/v3/v1/folders", () =>
    HttpResponse.json({
      folders: [],
      unfiled: Object.keys(dashboards).filter((id) => accessOf(id)).length,
    })
  ),
  http.get("/api/v3/v1/tags", () => HttpResponse.json({ tags: [] })),
];
