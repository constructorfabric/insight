import {
  Activity,
  AlertTriangle,
  BarChart3,
  BookOpen,
  Bot,
  Clock,
  Copy,
  DollarSign,
  FileText,
  Filter,
  Database,
  Fingerprint,
  FlaskConical,
  GitPullRequest,
  LayoutGrid,
  Layers,
  List,
  Lock,
  Megaphone,
  MessageSquare,
  Plug,
  Radar,
  ScanEye,
  Server,
  Settings2,
  Share2,
  Sparkles,
  Ticket,
  TrendingUp,
  User,
  Users,
  type LucideIcon,
} from "lucide-react";

import {
  itemHidden,
  itemPlanned,
  navPolicy,
  type InstanceNavPolicy,
} from "./nav-policy";


/**
 * Portal navigation model (Phase 1 buildout — mirrors the design mockup).
 *
 * This is the static composition the mockup demonstrates. Directions, their
 * lenses and the connector chips are hand-declared here for now; a later phase
 * derives them from the Analytics API metric catalog (see _work/portal-nav/SPEC.md).
 * Meaning stays server-owned; this file only carries structure + presentation.
 */

/* ── Rail zones ──────────────────────────────────────────────────────── */

export type ZoneKind =
  | "person"
  | "directions"
  | "theme"
  | "manage"
  | "people"
  | "custom";

export type Readiness = "planned";

export interface Zone {
  id: string;
  label: string;
  icon: LucideIcon;
  kind: ZoneKind;
}

export const ZONES: readonly Zone[] = [
  { id: "overview", label: "Overview", icon: LayoutGrid, kind: "theme" },
  { id: "directions", label: "Directions", icon: Layers, kind: "directions" },
  { id: "person", label: "Person", icon: User, kind: "person" },
  { id: "people", label: "People", icon: Users, kind: "people" },
  { id: "aicost", label: "AI & Cost", icon: DollarSign, kind: "theme" },
  { id: "reports", label: "Reports", icon: FileText, kind: "theme" },
  { id: "custom", label: "Custom", icon: Sparkles, kind: "custom" },
  { id: "manage", label: "Manage", icon: Settings2, kind: "manage" },
];

/** The zone a URL names, or undefined for an id no longer in the rail. */
export function lensSlug(lens: string): string {
  return lens
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

export function lensBySlug(direction: Direction, slug: string): string | undefined {
  return direction.lenses.find((lens) => lensSlug(lens) === slug);
}

export function zoneById(id: string | null): Zone | undefined {
  if (!id) return undefined;
  return ZONES.find((z) => z.id === id);
}

/* ── Directions (catalog-driven family list) ─────────────────────────── */

export type DirectionSource = "semantic" | "bullet";

export interface Direction {
  id: string;
  name: string;
  icon: LucideIcon;
  source: DirectionSource;
  lenses: readonly string[];
}

export const DIRECTIONS: readonly Direction[] = [
  {
    id: "dev",
    name: "Development",
    icon: GitPullRequest,
    source: "semantic",
    lenses: [
      "Overview",
      "Git output",
      "Delivery",
      "Activity",
      "Flow",
      "Quality",
      "CI",
      "Continuity",
      "Repositories",
      "Elements",
    ],
  },
  {
    id: "collab",
    name: "Collaboration",
    icon: MessageSquare,
    source: "semantic",
    lenses: ["Overview", "Messaging", "Meetings", "Email", "Focus time", "Files & sharing"],
  },
  {
    id: "wiki",
    name: "Knowledge / Wiki",
    icon: BookOpen,
    source: "semantic",
    lenses: ["Overview", "Authoring", "Edits & comments", "Active authors"],
  },
  {
    id: "sales",
    name: "Sales / CRM",
    icon: DollarSign,
    source: "bullet",
    lenses: ["Pipeline", "Deal flow", "Activity", "Velocity & quality"],
  },
  {
    id: "support",
    name: "Support",
    icon: Ticket,
    source: "bullet",
    lenses: ["Tickets", "CSAT", "Knowledge base", "Comments & updates"],
  },
];

/* ── Theme-zone section lists ────────────────────────────────────────── */

export interface PaneItem {
  id: string;
  label: string;
  icon: LucideIcon;
  badge?: { text: string; tone: "warn" | "new" | "error" };
  readiness?: Readiness;
  /**
   * Rendered only for viewers holding the active `admin` identity role
   * (`useIsAdmin`) — a UI courtesy over the server-side gate, which refuses
   * regardless of what the frontend draws.
   */
  adminOnly?: boolean;
  /**
   * Rendered only when the previews gate passes (`usePreviewsGate`) — the
   * same courtesy-over-server-gate doctrine as `adminOnly`.
   */
  previewsGated?: boolean;
  unbuilt?: boolean;
}

export interface PaneGroup {
  label?: string;
  icon?: LucideIcon;
  items: readonly PaneItem[];
}

/** The label the pane uses for the demoted group of planned entries. */
export const PLANNED_GROUP_LABEL = "Planned";

/**
 * Split entries into the views a reader can open and the marked ones that
 * belong under the demoted "Planned" group. Nothing marked survives
 * `showPlanned: false` — a reader who turned planned sections off is asking
 * for navigation that only lists what renders.
 */
export function partitionByReadiness<T extends { readiness?: Readiness }>(
  entries: readonly T[],
  showPlanned: boolean,
): { live: T[]; planned: T[] } {
  const live: T[] = [];
  const planned: T[] = [];
  for (const e of entries) {
    if (e.readiness == null) live.push(e);
    else if (showPlanned) planned.push(e);
  }
  return { live, planned };
}

function withConfigReadiness(
  zoneId: string,
  item: PaneItem,
  policy: InstanceNavPolicy,
): PaneItem {
  if (!itemPlanned(zoneId, item.id, policy)) return item;
  return { ...item, readiness: "planned" };
}

export function zoneSections(
  zoneId: string,
  policy: InstanceNavPolicy = navPolicy(),
): readonly PaneGroup[] {
  return (ZONE_SECTIONS[zoneId] ?? [])
    .map((group) => ({
      ...group,
      items: group.items
        .filter((item) => !itemHidden(zoneId, item.id, policy))
        .map((item) => withConfigReadiness(zoneId, item, policy)),
    }))
    .filter((group) => group.items.length > 0);
}

export const ZONE_SECTIONS: Record<string, readonly PaneGroup[]> = {
  overview: [
    {
      label: "Themes",
      items: [
        { id: "at-a-glance", label: "At a glance", icon: LayoutGrid },
        { id: "by-direction", label: "By direction", icon: Layers },
        { id: "trend", label: "Trend", icon: TrendingUp },
        { id: "attention", label: "Attention needed", icon: AlertTriangle },
        { id: "health", label: "Data coverage", icon: ScanEye },
        { id: "contribution", label: "Contribution breakdown", icon: Users },
      ],
    },
  ],
  aicost: [
    {
      items: [{ id: "overview", label: "Overview", icon: LayoutGrid }],
    },
    {
      label: "AI adoption",
      icon: Bot,
      items: [
        { id: "adoption-funnel", label: "Adoption funnel", icon: Activity },
        { id: "by-unit-role", label: "By unit / role", icon: Layers },
        { id: "per-tool", label: "Per-tool", icon: Sparkles },
        { id: "autofix", label: "Autofix", icon: Activity },
        { id: "ai-audit", label: "AI Audit", icon: Radar },
      ],
    },
    {
      label: "Cost",
      icon: DollarSign,
      items: [
        { id: "spend-by-tool", label: "Spend by tool", icon: DollarSign },
        { id: "cost-by-unit", label: "Cost by unit / user", icon: Users },
        { id: "idle-seats", label: "Idle seats", icon: Clock },
        { id: "credits", label: "Credits burn-down", icon: TrendingUp },
        {
          id: "ai-pricing",
          label: "AI pricing",
          icon: DollarSign,
          badge: { text: "ai.cost", tone: "error" },
        },
      ],
    },
  ],
  reports: [
    {
      label: "Planned",
      items: [
        { id: "snapshots", label: "Snapshots", icon: FileText, unbuilt: true },
        { id: "export", label: "Export PDF / HTML", icon: Share2, unbuilt: true },
        { id: "templates", label: "Templates", icon: Copy, unbuilt: true },
      ],
    },
  ],
};

export const HOME_DASHBOARD_GROUP: PaneGroup = {
  label: "Dashboards",
  items: [
    { id: "my-dashboards", label: "My dashboards", icon: LayoutGrid, unbuilt: true },
    { id: "starter-dashboards", label: "Starter dashboards", icon: Sparkles, unbuilt: true },
  ],
};

/* ── People zone ─────────────────────────────────────────────────────── */

export const PEOPLE_ITEMS: readonly PaneItem[] = [
  { id: "roster", label: "My team", icon: Users },
  { id: "employees", label: "Roster · by role", icon: List },
];

/**
 * The same two views under names a flat organisation can use: there is no
 * employees-versus-roster distinction to draw.
 *
 * INVARIANT: the ids match {@link PEOPLE_ITEMS} — the pane routes on them, so a
 * new People view has to be named for both shapes rather than one.
 */
const FLAT_PEOPLE_ITEMS: readonly PaneItem[] = [
  { id: "roster", label: "Overview", icon: LayoutGrid },
  { id: "employees", label: "Roster", icon: Users },
];

export function peopleItemsFor(
  isFlat: boolean,
  policy: InstanceNavPolicy = navPolicy(),
): readonly PaneItem[] {
  const items = isFlat ? FLAT_PEOPLE_ITEMS : PEOPLE_ITEMS;
  return items
    .filter((item) => !itemHidden("people", item.id, policy))
    .map((item) => withConfigReadiness("people", item, policy));
}

/* ── Manage zone ─────────────────────────────────────────────────────── */

/** The viewer facts that decide which gated Manage surfaces exist for them. */
export interface ManageGates {
  isAdmin: boolean;
  canManagePreviews: boolean;
}

/** The Manage pane for one viewer: gated surfaces drop for everyone else. */
export function manageGroupsFor(
  gates: ManageGates,
  policy: InstanceNavPolicy = navPolicy(),
): readonly PaneGroup[] {
  return MANAGE_GROUPS.map((group) => ({
    ...group,
    items: group.items
      .filter(
        (item) =>
          (!item.adminOnly || gates.isAdmin) &&
          (!item.previewsGated || gates.canManagePreviews) &&
          !itemHidden("manage", item.id, policy),
      )
      .map((item) => withConfigReadiness("manage", item, policy)),
  })).filter((group) => group.items.length > 0);
}

export const MANAGE_GROUPS: readonly PaneGroup[] = [
  {
    label: "Data",
    items: [
      {
        id: "connector-health",
        label: "Sources & connectors",
        icon: Plug,
        adminOnly: true,
      },
      { id: "exclusions", label: "Data exclusions", icon: Filter },
      { id: "snapshots", label: "Org snapshots", icon: Clock },
    ],
  },
  {
    label: "People & access",
    items: [
      {
        id: "identities",
        label: "Identities · roles & taxonomy",
        icon: Fingerprint,
        adminOnly: true,
      },
      { id: "access", label: "Access", icon: Lock, unbuilt: true },
      { id: "group-mgmt", label: "Group management", icon: Users },
    ],
  },
  {
    label: "Platform",
    items: [
      { id: "ingestion", label: "Ingestion", icon: Database, adminOnly: true },
      { id: "previews", label: "Previews", icon: FlaskConical, previewsGated: true },
      { id: "ai-assistant", label: "AI assistant config", icon: Sparkles },
      { id: "whats-new", label: "What's new", icon: Megaphone },
      { id: "platform-usage", label: "Platform usage", icon: Activity, adminOnly: true },
      { id: "mcp", label: "MCP servers", icon: Server },
      { id: "config", label: "Config & setup", icon: Settings2 },
      { id: "scorecard-mgmt", label: "Scorecard management", icon: BarChart3 },
    ],
  },
];

export const MANAGE_ITEMS: readonly PaneItem[] = MANAGE_GROUPS.flatMap(
  (group) => group.items,
);

/* ── Zone item resolution ────────────────────────────────────────────── */

export function zoneItems(zoneId: string): readonly PaneItem[] {
  if (zoneId === "people") return PEOPLE_ITEMS;
  if (zoneId === "manage") return MANAGE_ITEMS;
  return (ZONE_SECTIONS[zoneId] ?? []).flatMap((g) => g.items);
}

export function defaultZoneItem(zoneId: string): string | null {
  return zoneItems(zoneId).find((i) => !i.unbuilt && !i.adminOnly)?.id ?? null;
}

export function resolveZoneItem(
  zoneId: string,
  item: string | null,
  policy: InstanceNavPolicy = navPolicy(),
): string | null {
  const shown = (i: PaneItem) => !i.unbuilt && !itemHidden(zoneId, i.id, policy);
  const live = (i: PaneItem) => !itemPlanned(zoneId, i.id, policy);
  const items = zoneItems(zoneId);
  if (item && items.some((i) => i.id === item && shown(i))) return item;
  return items.find((i) => live(i) && shown(i) && !i.adminOnly)?.id ?? null;
}
