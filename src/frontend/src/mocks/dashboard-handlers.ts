import { http, HttpResponse } from "msw";

import type { Dashboard, Listed } from "@/api/custom-client";

interface StoredDashboard extends Listed {
  body: Dashboard;
}

const dashboards: StoredDashboard[] = [
  {
    id: "0192f0aa-0000-7000-8000-00000000d001",
    name: "Delivery overview",
    body: { title: "Delivery overview", widgets: [] },
  },
  {
    id: "0192f0aa-0000-7000-8000-00000000d002",
    name: "Review health",
    body: { title: "Review health", widgets: [] },
  },
  {
    id: "0192f0aa-0000-7000-8000-00000000d003",
    name: "Team activity",
    body: { title: "Team activity", widgets: [] },
  },
];

export const dashboardHandlers = [
  http.get("/api/v3/v1/dashboards", ({ request }) => {
    const query = new URL(request.url).searchParams;
    const search = (query.get("q") ?? "").toLowerCase();
    const found = dashboards.filter(({ name }) =>
      name.toLowerCase().includes(search)
    );
    const offset = Number(query.get("offset") ?? 0);
    const limit = Number(query.get("limit") ?? 50);

    return HttpResponse.json({
      items: found
        .slice(offset, offset + limit)
        .map(({ id, name }) => ({ id, name })),
      total: found.length,
      limit,
      offset,
    });
  }),
  http.get("/api/v3/v1/dashboards/:id", ({ params }) => {
    const stored = dashboards.find(({ id }) => id === String(params.id));

    return stored
      ? HttpResponse.json({ ...stored, folder: null, tags: [] })
      : new HttpResponse(null, { status: 404 });
  }),
  http.get("/api/v3/v1/pins", () => HttpResponse.json({ pins: [] })),
  http.get("/api/v3/v1/folders", () =>
    HttpResponse.json({ folders: [], unfiled: dashboards.length })
  ),
  http.get("/api/v3/v1/tags", () => HttpResponse.json({ tags: [] })),
];
