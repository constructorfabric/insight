import { http, HttpResponse } from "msw";

import type {
  Dashboard,
  DashboardAccess,
  FolderAccess,
  Grant,
  GrantLevel,
} from "@/api/custom-client";

import { isAdministrator } from "./admin-assignments";
import { PEOPLE } from "./registry";

const CALLER = PEOPLE[0]?.person_id ?? "";
const COLLEAGUE = PEOPLE[1]?.person_id ?? "";
const LEAVER = PEOPLE[2]?.person_id ?? "";
const UPDATED_AT = "2026-09-01T09:00:00.000Z";
const EVERYONE = "everyone";

interface MockDashboard {
  name: string;
  body: Dashboard;
  owner: string | null;
  grants: Grant[];
}

interface MockEntry {
  dashboard: string;
  carries: boolean;
}

interface MockFolder {
  name: string;
  owner: string | null;
  grants: Grant[];
  entries: MockEntry[];
}

const granted = (grantee: string, level: GrantLevel, by: string): Grant => ({
  grantee,
  level,
  granted_by: by,
  created_at: UPDATED_AT,
});

const dashboards: Record<string, MockDashboard> = {
  "delivery-overview": {
    name: "Delivery overview",
    body: { title: "Delivery overview", widgets: [] },
    owner: CALLER,
    grants: [granted(COLLEAGUE, "editor", CALLER)],
  },
  "review-health": {
    name: "Review health",
    body: { title: "Review health", widgets: [] },
    owner: COLLEAGUE,
    grants: [granted(CALLER, "viewer", COLLEAGUE)],
  },
  "team-activity": {
    name: "Team activity",
    body: { title: "Team activity", widgets: [] },
    owner: null,
    grants: [],
  },
  "release-notes": {
    name: "Release notes",
    body: { title: "Release notes", widgets: [] },
    owner: LEAVER,
    grants: [granted(EVERYONE, "viewer", LEAVER)],
  },
};

const folders: Record<string, MockFolder> = {
  "folder-delivery": {
    name: "Delivery",
    owner: CALLER,
    grants: [granted(COLLEAGUE, "viewer", CALLER)],
    entries: [{ dashboard: "delivery-overview", carries: true }],
  },
  "folder-handover": {
    name: "Handover",
    owner: LEAVER,
    grants: [granted(CALLER, "editor", LEAVER)],
    entries: [{ dashboard: "release-notes", carries: true }],
  },
};

const RANK: Record<DashboardAccess, number> = {
  viewer: 1,
  editor: 2,
  owner: 3,
  ownerless: 3,
};

function strongest<T extends DashboardAccess>(levels: (T | null)[]): T | null {
  return levels.reduce<T | null>(
    (best, level) =>
      level !== null && (best === null || RANK[level] > RANK[best])
        ? level
        : best,
    null
  );
}

function grantedLevel(grants: readonly Grant[]): GrantLevel | null {
  return strongest(
    grants
      .filter((grant) => grant.grantee === CALLER || grant.grantee === EVERYONE)
      .map((grant) => grant.level)
  );
}

function folderAccessOf(id: string): FolderAccess | null {
  const folder = folders[id];
  if (!folder) return null;
  if (folder.owner === null) return "ownerless";
  if (folder.owner === CALLER) return "owner";

  return grantedLevel(folder.grants);
}

function accessOf(id: string): DashboardAccess | null {
  const dashboard = dashboards[id];
  if (!dashboard) return null;
  if (dashboard.owner === null) return "ownerless";

  const owned = dashboard.owner === CALLER ? "owner" : null;
  const throughFolders = Object.keys(folders).map((folderId) => {
    const carried = folders[folderId]?.entries.some(
      (entry) => entry.dashboard === id && entry.carries
    );
    const reached = carried ? folderAccessOf(folderId) : null;
    if (reached === null) return null;

    return reached === "viewer" ? "viewer" : "editor";
  });

  return strongest<DashboardAccess>([
    owned,
    grantedLevel(dashboard.grants),
    ...throughFolders,
  ]);
}

/** Whether an entry the caller adds would carry: owner, or a direct or Everyone Editor grant. */
function carriesFor(id: string): boolean {
  const dashboard = dashboards[id];
  if (!dashboard) return false;
  if (dashboard.owner === null || dashboard.owner === CALLER) return true;

  return grantedLevel(dashboard.grants) === "editor";
}

function isOrphaned(owner: string | null): owner is string {
  return owner !== null && !isAdministrator(owner);
}

function visibleFolders(): string[] {
  return Object.keys(folders).filter((id) => folderAccessOf(id) !== null);
}

function foldersHolding(id: string) {
  return visibleFolders()
    .filter((folderId) =>
      folders[folderId]?.entries.some((entry) => entry.dashboard === id)
    )
    .map((folderId) => ({ id: folderId, name: folders[folderId]?.name ?? "" }));
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
const done = () => new HttpResponse(null, { status: 204 });

const denied = (detail: string) =>
  HttpResponse.json({ detail }, { status: 403 });

const refusedTransfer = (kind: string, description: string) =>
  HttpResponse.json(
    {
      detail: "Failed precondition",
      context: { violations: [{ type: kind, subject: "owner", description }] },
    },
    { status: 409 }
  );

const notAdministrator = () =>
  HttpResponse.json(
    {
      detail: "Invalid argument",
      context: {
        field_violations: [
          {
            field: "to",
            description:
              "ownership is transferred only to a current administrator",
          },
        ],
      },
    },
    { status: 400 }
  );

/** The grants of a dashboard or folder the caller may share, or a refusal. */
function sharedThing(
  kind: string,
  id: string
): { grants: Grant[]; set: (grants: Grant[]) => void } | Response {
  const thing = kind === "dashboards" ? dashboards[id] : folders[id];
  const access = kind === "dashboards" ? accessOf(id) : folderAccessOf(id);
  if (!thing || !access) return notFound();
  if (access !== "owner" && access !== "editor") {
    return denied("only the owner or an editor can share this");
  }

  return {
    grants: thing.grants,
    set: (grants) => {
      thing.grants = grants;
    },
  };
}

function inFolderFilter(id: string, folder: string | null): boolean {
  if (!folder) return true;

  const holding = foldersHolding(id).map((one) => one.id);
  return folder === "unfiled" ? holding.length === 0 : holding.includes(folder);
}

export const dashboardHandlers = [
  http.get("/api/v3/v1/dashboards", ({ request }) => {
    const query = new URL(request.url).searchParams;
    const search = (query.get("q") ?? "").toLowerCase();
    const owned = query.get("owned");
    const folder = query.get("folder");
    const items = Object.keys(dashboards)
      .filter((id) => inFolderFilter(id, folder))
      .map(listed)
      .filter((item) => item !== null)
      .filter((item) => item.name.toLowerCase().includes(search))
      .filter(
        (item) =>
          (owned !== "me" || item.access === "owner") &&
          (owned !== "shared" ||
            item.access === "viewer" ||
            item.access === "editor")
      );

    return HttpResponse.json(page(items, query));
  }),
  http.get("/api/v3/v1/orphaned-dashboards", ({ request }) => {
    const query = new URL(request.url).searchParams;
    const items = Object.entries(dashboards).flatMap(([id, dashboard]) =>
      isOrphaned(dashboard.owner)
        ? [
            {
              id,
              name: dashboard.name,
              owner_person_id: dashboard.owner,
              updated_at: UPDATED_AT,
            },
          ]
        : []
    );

    return HttpResponse.json(page(items, query));
  }),
  http.get("/api/v3/v1/orphaned-folders", ({ request }) => {
    const query = new URL(request.url).searchParams;
    const items = Object.entries(folders).flatMap(([id, folder]) =>
      isOrphaned(folder.owner) && folder.grants.length > 0
        ? [{ id, name: folder.name, owner_person_id: folder.owner }]
        : []
    );

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
      folders: foldersHolding(id),
      carries: carriesFor(id),
      tags: [],
      updated_at: UPDATED_AT,
    });
  }),
  http.get("/api/v3/v1/dashboards/:id/widgets", ({ params }) =>
    accessOf(String(params.id)) ? HttpResponse.json({ items: [] }) : notFound()
  ),
  http.get("/api/v3/v1/:kind/:id/grants", ({ params }) => {
    const shared = sharedThing(String(params.kind), String(params.id));
    if (shared instanceof Response) return shared;

    return HttpResponse.json({ items: shared.grants });
  }),
  http.put(
    "/api/v3/v1/:kind/:id/grants/:grantee",
    async ({ params, request }) => {
      const shared = sharedThing(String(params.kind), String(params.id));
      if (shared instanceof Response) return shared;

      const grantee = String(params.grantee);
      const { level } = (await request.json()) as { level: GrantLevel };
      shared.set([
        ...shared.grants.filter((grant) => grant.grantee !== grantee),
        granted(grantee, level, CALLER),
      ]);
      return done();
    }
  ),
  http.delete("/api/v3/v1/:kind/:id/grants/:grantee", ({ params }) => {
    const shared = sharedThing(String(params.kind), String(params.id));
    if (shared instanceof Response) return shared;

    const grantee = String(params.grantee);
    shared.set(shared.grants.filter((grant) => grant.grantee !== grantee));
    return done();
  }),
  http.post("/api/v3/v1/:kind/:id/transfer", async ({ params, request }) => {
    const kind = String(params.kind);
    const id = String(params.id);
    const thing = kind === "dashboards" ? dashboards[id] : folders[id];
    if (!thing) return notFound();
    if (thing.owner === null) {
      return refusedTransfer(
        "ownerless",
        "an ownerless one cannot be transferred"
      );
    }
    if (!isOrphaned(thing.owner)) {
      return refusedTransfer(
        "owner_is_administrator",
        "its owner is still an administrator"
      );
    }

    const { to } = (await request.json()) as { to: string };
    if (!isAdministrator(to)) return notAdministrator();

    thing.owner = to;
    return done();
  }),
  http.get("/api/v3/v1/pins", () => HttpResponse.json({ pins: [] })),
  http.get("/api/v3/v1/folders", () => {
    const shown = visibleFolders().flatMap((id) => {
      const folder = folders[id];
      const access = folderAccessOf(id);
      if (!folder || !access) return [];

      return [
        {
          id,
          name: folder.name,
          owner_person_id: folder.owner,
          access,
          dashboards: folder.entries.filter((entry) =>
            accessOf(entry.dashboard)
          ).length,
        },
      ];
    });
    const unfiled = Object.keys(dashboards).filter(
      (id) => accessOf(id) && foldersHolding(id).length === 0
    ).length;

    return HttpResponse.json({ folders: shown, unfiled });
  }),
  http.post("/api/v3/v1/folders", async ({ request }) => {
    const { name } = (await request.json()) as { name: string };
    const id = `folder-${Object.keys(folders).length + 1}`;
    folders[id] = { name, owner: CALLER, grants: [], entries: [] };

    return HttpResponse.json({ id, name }, { status: 201 });
  }),
  http.put("/api/v3/v1/folders/:id/dashboards/:dashboard", ({ params }) => {
    const folder = folders[String(params.id)];
    const dashboard = String(params.dashboard);
    const access = folderAccessOf(String(params.id));
    if (!folder || !access || !accessOf(dashboard)) return notFound();
    if (access === "viewer")
      return denied("only the owner or an editor can change this folder");

    const carries = carriesFor(dashboard);
    folder.entries = [
      ...folder.entries.filter((entry) => entry.dashboard !== dashboard),
      { dashboard, carries },
    ];
    return HttpResponse.json({ carries });
  }),
  http.delete("/api/v3/v1/folders/:id/dashboards/:dashboard", ({ params }) => {
    const folder = folders[String(params.id)];
    const access = folderAccessOf(String(params.id));
    if (!folder || !access) return notFound();
    if (access === "viewer")
      return denied("only the owner or an editor can change this folder");

    const dashboard = String(params.dashboard);
    folder.entries = folder.entries.filter(
      (entry) => entry.dashboard !== dashboard
    );
    return done();
  }),
  http.get("/api/v3/v1/tags", () => HttpResponse.json({ tags: [] })),
];
