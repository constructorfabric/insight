import {
  Activity,
  AlertTriangle,
  BarChart3,
  BookOpen,
  Boxes,
  Clock,
  DollarSign,
  FileText,
  Filter,
  Fingerprint,
  GitPullRequest,
  LayoutGrid,
  Layers,
  Megaphone,
  MessageSquare,
  Plus,
  Radar,
  ScanEye,
  Server,
  Settings2,
  ShieldCheck,
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

export type ZoneKind = "person" | "directions" | "theme" | "manage" | "people";

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
  { id: "scorecard", label: "Scorecard", icon: BarChart3, kind: "theme" },
  { id: "reports", label: "Reports", icon: FileText, kind: "theme" },
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
}

export interface PaneGroup {
  label?: string;
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
  scorecard: [
    {
      items: [
        { id: "fixed", label: "Fixed scorecard", icon: LayoutGrid },
        { id: "detailed", label: "Detailed breakdown", icon: Layers },
        { id: "quarterly", label: "Quarter over quarter", icon: TrendingUp },
      ],
    },
  ],
  reports: [
    {
      label: "Generated reports",
      items: [
        { id: "delivery-trend", label: "Delivery trend", icon: FileText },
        { id: "ttm", label: "Trailing twelve months", icon: FileText },
      ],
    },
    {
      label: "Custom",
      items: [
        { id: "report-builder", label: "Report builder", icon: LayoutGrid },
        { id: "dashboards", label: "Saved dashboards", icon: Layers },
        { id: "new-report", label: "New report", icon: Plus },
      ],
    },
  ],
};

/* ── People zone ─────────────────────────────────────────────────────── */

// No "Person" item here — the individual view is the dedicated Person rail
// zone (reached by drilling into any name); listing it again would duplicate it.
export const PEOPLE_ITEMS: readonly PaneItem[] = [
  { id: "roster", label: "People (roster)", icon: Users },
  { id: "median-by-role", label: "Median by Role", icon: BarChart3 },
  { id: "employees", label: "Employees", icon: Fingerprint },
];

/**
 * The same two views under names a flat organisation can use: there is no
 * employees-versus-roster distinction to draw, and no job titles to cut a
 * median by, so that entry is absent rather than empty.
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

/** The Manage pane for one viewer: admin-only surfaces drop for everyone else. */
export function manageItemsFor(
  isAdmin: boolean,
  policy: InstanceNavPolicy = navPolicy(),
): readonly PaneItem[] {
  return MANAGE_ITEMS.filter(
    (item) => (!item.adminOnly || isAdmin) && !itemHidden("manage", item.id, policy),
  ).map((item) => withConfigReadiness("manage", item, policy));
}

export const MANAGE_ITEMS: readonly PaneItem[] = [
  { id: "metric-catalog", label: "Metric catalog", icon: LayoutGrid },
  { id: "identities", label: "Identities", icon: Fingerprint, adminOnly: true },
  { id: "taxonomy", label: "Roles & taxonomy", icon: Boxes },
  { id: "exclusions", label: "Data exclusions", icon: Filter },
  { id: "snapshots", label: "Org snapshots", icon: Clock },
  { id: "group-mgmt", label: "Group management", icon: Users },
  { id: "scorecard-mgmt", label: "Scorecard management", icon: BarChart3 },
  { id: "data-health", label: "Data health", icon: ShieldCheck },
  { id: "platform-usage", label: "Platform usage", icon: Activity, adminOnly: true },
  { id: "mcp", label: "MCP servers", icon: Server },
  { id: "config", label: "Config & setup", icon: Settings2 },
  { id: "ai-assistant", label: "AI assistant", icon: Sparkles },
  { id: "whats-new", label: "What's new", icon: Megaphone },
];

/* ── Zone item resolution ────────────────────────────────────────────── */

export function zoneItems(zoneId: string): readonly PaneItem[] {
  if (zoneId === "people") return PEOPLE_ITEMS;
  if (zoneId === "manage") return MANAGE_ITEMS;
  return (ZONE_SECTIONS[zoneId] ?? []).flatMap((g) => g.items);
}

export function defaultZoneItem(zoneId: string): string | null {
  return zoneItems(zoneId)[0]?.id ?? null;
}

export function resolveZoneItem(
  zoneId: string,
  item: string | null,
  policy: InstanceNavPolicy = navPolicy(),
): string | null {
  const shown = (i: PaneItem) => !itemHidden(zoneId, i.id, policy);
  const live = (i: PaneItem) => !itemPlanned(zoneId, i.id, policy);
  const items = zoneItems(zoneId);
  if (item && items.some((i) => i.id === item && shown(i))) return item;
  return items.find((i) => live(i) && shown(i))?.id ?? null;
}
