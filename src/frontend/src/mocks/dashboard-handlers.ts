import { http, HttpResponse } from "msw";

import type { Dashboard } from "@/api/custom-client";

const dashboards: Record<string, Dashboard> = {
  "delivery-overview": { title: "Delivery overview", widgets: [] },
  "review-health": { title: "Review health", widgets: [] },
  "team-activity": { title: "Team activity", widgets: [] },
};

export const dashboardHandlers = [
  http.get("/api/v3/v1/dashboards", ({ request }) => {
    const query = new URL(request.url).searchParams;
    const search = (query.get("q") ?? "").toLowerCase();
    const names = Object.keys(dashboards).filter((name) =>
      name.includes(search)
    );
    const offset = Number(query.get("offset") ?? 0);
    const limit = Number(query.get("limit") ?? 50);

    return HttpResponse.json({
      names: names.slice(offset, offset + limit),
      total: names.length,
    });
  }),
  http.get("/api/v3/v1/dashboards/:name", ({ params }) => {
    const body = dashboards[String(params.name)];

    return body
      ? HttpResponse.json({ body, tags: [] })
      : new HttpResponse(null, { status: 404 });
  }),
  http.get("/api/v3/v1/pins", () => HttpResponse.json({ pins: [] })),
  http.get("/api/v3/v1/folders", () =>
    HttpResponse.json({ folders: [], unfiled: Object.keys(dashboards).length })
  ),
  http.get("/api/v3/v1/tags", () => HttpResponse.json({ tags: [] })),
];
