import { http, HttpResponse } from "msw";

import type {
  Dashboard,
  DashboardAccess,
  DashboardVisibility,
  DashboardWidget,
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
const MAX_FOLDERS = 200;
const MAX_PINS = 20;

const DASHBOARD_NOT_EDITOR =
  "only the owner or an editor can change this dashboard";
const DASHBOARD_NOT_SHARER =
  "only the owner or an editor granted this dashboard directly or through everyone can share it";
const FOLDER_NOT_EDITOR = "only the owner or an editor can change this folder";
const FOLDER_NOT_OWNER = "only the owner can delete this folder";
const ADMIN_ONLY = "admin role required for this operation";

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
    body: { title: "Delivery overview", widgets: ["deploys-per-week"] },
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

const widgets: Record<string, DashboardWidget> = {
  "deploys-per-week": {
    id: "deploys-per-week",
    name: "Deploys per week",
    body: { type: "table", metric: "deploys", columns: ["week", "deploys"] },
    clock: null,
  },
};

const ROWS = {
  columns: ["week", "deploys"],
  rows: [
    ["2026-08-24", 12],
    ["2026-08-31", 9],
  ],
};

const pins: string[] = [];

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
  "folder-archive": {
    name: "Archive",
    owner: null,
    grants: [],
    entries: [{ dashboard: "team-activity", carries: false }],
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
  if (folder.owner === null) {
    return isAdministrator(CALLER) ? "ownerless" : null;
  }
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

function visibilityOf(grants: readonly Grant[]): DashboardVisibility {
  if (grants.some((grant) => grant.grantee === EVERYONE)) return "everyone";

  return grants.length > 0 ? "people" : "private";
}

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

  return {
    id,
    name: dashboard.name,
    owner_person_id: dashboard.owner,
    access,
    updated_at: UPDATED_AT,
    visibility: visibilityOf(dashboard.grants),
  };
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

const denied = (reason: string) =>
  HttpResponse.json(
    {
      detail: "You do not have permission to perform this operation",
      context: { reason },
    },
    { status: 403 }
  );

const conflict = (subject: string, kind: string, description: string) =>
  HttpResponse.json(
    {
      detail: "Failed precondition",
      context: { violations: [{ type: kind, subject, description }] },
    },
    { status: 409 }
  );

const refusedTransfer = (kind: string, description: string) =>
  conflict("owner", kind, description);

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

function shareRefusal(kind: string, id: string): Response | null {
  const dashboard = kind === "dashboards";
  const access = dashboard ? accessOf(id) : folderAccessOf(id);
  const noun = dashboard ? "dashboard" : "folder";

  if (!access) return notFound();
  if (access === "ownerless") {
    return conflict(
      "owner",
      "ownerless",
      `an ownerless ${noun} cannot be shared or transferred`
    );
  }
  if (access === "viewer") {
    return denied(dashboard ? DASHBOARD_NOT_EDITOR : FOLDER_NOT_EDITOR);
  }
  if (access === "owner") return null;

  const direct = grantedLevel(
    dashboards[id]?.grants ?? folders[id]?.grants ?? []
  );
  if (direct === "editor") return null;

  return denied(dashboard ? DASHBOARD_NOT_SHARER : FOLDER_NOT_EDITOR);
}

function sharedThing(
  kind: string,
  id: string
): { grants: Grant[]; set: (grants: Grant[]) => void } | Response {
  if (!isAdministrator(CALLER)) return denied(ADMIN_ONLY);

  const thing = kind === "dashboards" ? dashboards[id] : folders[id];
  const refused = shareRefusal(kind, id);
  if (refused) return refused;
  if (!thing) return notFound();

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
        ? [
            {
              id,
              name: folder.name,
              owner_person_id: folder.owner,
              updated_at: UPDATED_AT,
            },
          ]
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
  http.get("/api/v3/v1/dashboards/:id/widgets", ({ params }) => {
    const id = String(params.id);
    const dashboard = dashboards[id];
    if (!dashboard || !accessOf(id)) return notFound();

    const items = (dashboard.body.widgets ?? []).flatMap((widget) => {
      const drawn = widgets[widget];
      return drawn ? [drawn] : [];
    });
    return HttpResponse.json({ items });
  }),
  http.post("/api/v3/v1/dashboards/:id/widgets/:widget/run", ({ params }) => {
    const id = String(params.id);
    const widget = String(params.widget);
    const drawn = dashboards[id]?.body.widgets?.includes(widget);
    if (!accessOf(id) || !drawn || !widgets[widget]) return notFound();

    return HttpResponse.json(ROWS);
  }),
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
  http.get("/api/v3/v1/pins", () =>
    HttpResponse.json({
      pins: pins.flatMap((id) => {
        const shown = listed(id);
        return shown ? [{ id, name: shown.name }] : [];
      }),
    })
  ),
  http.put("/api/v3/v1/pins/:id", ({ params }) => {
    const id = String(params.id);
    if (!accessOf(id)) return notFound();
    if (pins.includes(id)) return done();
    if (pins.length >= MAX_PINS) {
      return conflict(
        "name",
        "too_many",
        `at most ${MAX_PINS} dashboards can be pinned; unpin one first`
      );
    }

    pins.push(id);
    return done();
  }),
  http.delete("/api/v3/v1/pins/:id", ({ params }) => {
    const id = String(params.id);
    const at = pins.indexOf(id);
    if (at === -1) return accessOf(id) ? done() : notFound();

    pins.splice(at, 1);
    return done();
  }),
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
    const owned = Object.values(folders).filter(
      (folder) => folder.owner === CALLER
    ).length;
    if (owned >= MAX_FOLDERS) {
      return conflict(
        "name",
        "too_many",
        `at most ${MAX_FOLDERS} folders can be yours; delete one first`
      );
    }

    const id = `folder-${Object.keys(folders).length + 1}`;
    folders[id] = { name, owner: CALLER, grants: [], entries: [] };

    return HttpResponse.json(
      { id, name, owner_person_id: CALLER, access: "owner" },
      { status: 201 }
    );
  }),
  http.patch("/api/v3/v1/folders/:id", async ({ params, request }) => {
    const id = String(params.id);
    const folder = folders[id];
    const access = folderAccessOf(id);
    if (!folder || !access) return notFound();
    if (access === "viewer") return denied(FOLDER_NOT_EDITOR);

    const { name } = (await request.json()) as { name: string };
    folder.name = name.trim();
    return HttpResponse.json({
      id,
      name: folder.name,
      owner_person_id: folder.owner,
      access,
    });
  }),
  http.delete("/api/v3/v1/folders/:id", ({ params }) => {
    const id = String(params.id);
    const access = folderAccessOf(id);
    if (!access) return notFound();
    if (access !== "owner" && access !== "ownerless") {
      return denied(FOLDER_NOT_OWNER);
    }

    delete folders[id];
    return done();
  }),
  http.put("/api/v3/v1/folders/:id/dashboards/:dashboard", ({ params }) => {
    const folder = folders[String(params.id)];
    const dashboard = String(params.dashboard);
    const access = folderAccessOf(String(params.id));
    if (!folder || !access) return notFound();
    if (access === "viewer") return denied(FOLDER_NOT_EDITOR);

    const seen = accessOf(dashboard);
    if (!seen) return notFound();
    if ((seen === "ownerless") !== (access === "ownerless")) {
      return conflict(
        "folder",
        "wrong_shelf",
        seen === "ownerless"
          ? "an ownerless dashboard goes only in an ownerless folder"
          : "an ownerless folder holds only ownerless dashboards"
      );
    }

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
    if (access === "viewer") return denied(FOLDER_NOT_EDITOR);

    const dashboard = String(params.dashboard);
    folder.entries = folder.entries.filter(
      (entry) => entry.dashboard !== dashboard
    );
    return done();
  }),
  http.get("/api/v3/v1/tags", () => HttpResponse.json({ tags: [] })),
];
