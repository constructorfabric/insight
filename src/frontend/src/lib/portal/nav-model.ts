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

/**
 * Why a navigation entry has nothing behind it. Three genuinely different
 * causes that must not look alike to the reader:
 *
 * - **absent** (the default, no marker) — the surface is built and backed by
 *   data. If it still renders empty, that is a per-tenant data gap and the
 *   view says which source is missing. Always visible: the gap IS the signal.
 * - **`planned`** — the product does not model this yet (a metric family is
 *   not in the semantic layer). Identical for every tenant.
 * - **`unbuilt`** — WE have not built the screen yet, though the data path
 *   exists. This is our backlog, not roadmap communication.
 *
 * Rendering a tenant data gap and our own unfinished UI the same way is what
 * makes both meaningless, which is why the distinction is in the model rather
 * than in prose. It shapes how an entry reads, not whether the reader's
 * "show planned sections" choice reaches it: either marker hides when that is
 * off, because neither renders a view.
 */
export type Readiness = "planned" | "unbuilt";

export interface Zone {
  id: string;
  label: string;
  icon: LucideIcon;
  kind: ZoneKind;
  readiness?: Readiness;
}

export const ZONES: readonly Zone[] = [
  { id: "overview", label: "Overview", icon: LayoutGrid, kind: "theme" },
  { id: "directions", label: "Directions", icon: Layers, kind: "directions" },
  { id: "person", label: "Person", icon: User, kind: "person" },
  { id: "people", label: "People", icon: Users, kind: "people" },
  { id: "aicost", label: "AI & Cost", icon: DollarSign, kind: "theme" },
  // Pure scaffolds: no view, no data path. Our backlog, not a tenant gap.
  { id: "scorecard", label: "Scorecard", icon: BarChart3, kind: "theme", readiness: "unbuilt" },
  { id: "reports", label: "Reports", icon: FileText, kind: "theme" },
  { id: "manage", label: "Manage", icon: Settings2, kind: "manage" },
];

/** The zone a URL names, or undefined for an id no longer in the rail. */
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
  /** See {@link Readiness}. Absent = built and data-backed. */
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
        { id: "per-tool", label: "Per-tool", icon: Sparkles, readiness: "unbuilt" },
        { id: "autofix", label: "Autofix", icon: Activity, readiness: "planned" },
        { id: "ai-audit", label: "AI Audit", icon: Radar, readiness: "unbuilt" },
      ],
    },
    {
      label: "Cost",
      items: [
        { id: "spend-by-tool", label: "Spend by tool", icon: DollarSign, readiness: "unbuilt" },
        { id: "cost-by-unit", label: "Cost by unit / user", icon: Users, readiness: "unbuilt" },
        { id: "idle-seats", label: "Idle seats", icon: Clock, readiness: "unbuilt" },
        { id: "credits", label: "Credits burn-down", icon: TrendingUp, readiness: "planned" },
        {
          id: "ai-pricing",
          label: "AI pricing",
          icon: DollarSign,
          badge: { text: "ai.cost", tone: "error" },
          readiness: "unbuilt",
        },
      ],
    },
  ],
  scorecard: [
    {
      items: [
        { id: "fixed", label: "Fixed scorecard", icon: LayoutGrid, readiness: "unbuilt" },
        { id: "detailed", label: "Detailed breakdown", icon: Layers, readiness: "unbuilt" },
        { id: "quarterly", label: "Quarter over quarter", icon: TrendingUp, readiness: "unbuilt" },
      ],
    },
  ],
  reports: [
    {
      label: "Generated reports",
      items: [
        { id: "delivery-trend", label: "Delivery trend", icon: FileText, readiness: "unbuilt" },
        { id: "ttm", label: "Trailing twelve months", icon: FileText, readiness: "unbuilt" },
      ],
    },
    {
      label: "Custom",
      items: [
        { id: "report-builder", label: "Report builder", icon: LayoutGrid },
        { id: "dashboards", label: "Saved dashboards", icon: Layers, readiness: "unbuilt" },
        { id: "new-report", label: "New report", icon: Plus, readiness: "unbuilt" },
      ],
    },
  ],
};

/* ── People zone ─────────────────────────────────────────────────────── */

// No "Person" item here — the individual view is the dedicated Person rail
// zone (reached by drilling into any name); listing it again would duplicate it.
export const PEOPLE_ITEMS: readonly PaneItem[] = [
  { id: "roster", label: "People (roster)", icon: Users },
  { id: "median-by-role", label: "Median by Role", icon: BarChart3, readiness: "unbuilt" },
  { id: "employees", label: "Employees", icon: Fingerprint },
];

/* ── Manage zone ─────────────────────────────────────────────────────── */

/** The Manage pane for one viewer: admin-only surfaces drop for everyone else. */
export function manageItemsFor(isAdmin: boolean): readonly PaneItem[] {
  return MANAGE_ITEMS.filter((item) => !item.adminOnly || isAdmin);
}

export const MANAGE_ITEMS: readonly PaneItem[] = [
  { id: "metric-catalog", label: "Metric catalog", icon: LayoutGrid },
  { id: "identities", label: "Identities", icon: Fingerprint, adminOnly: true },
  { id: "taxonomy", label: "Roles & taxonomy", icon: Boxes, readiness: "unbuilt" },
  { id: "exclusions", label: "Data exclusions", icon: Filter, readiness: "unbuilt" },
  { id: "snapshots", label: "Org snapshots", icon: Clock, readiness: "unbuilt" },
  { id: "group-mgmt", label: "Group management", icon: Users, readiness: "unbuilt" },
  { id: "scorecard-mgmt", label: "Scorecard management", icon: BarChart3, readiness: "unbuilt" },
  { id: "data-health", label: "Data health", icon: ShieldCheck },
  { id: "platform-usage", label: "Platform usage", icon: Activity, readiness: "unbuilt" },
  { id: "mcp", label: "MCP servers", icon: Server, readiness: "unbuilt" },
  { id: "config", label: "Config & setup", icon: Settings2, readiness: "unbuilt" },
  { id: "whats-new", label: "What's new", icon: Megaphone },
];

/* ── Zone item resolution ────────────────────────────────────────────── */

/** Every pane item a zone lists, in display order, planned ones included. */
export function zoneItems(zoneId: string): readonly PaneItem[] {
  if (zoneId === "people") return PEOPLE_ITEMS;
  if (zoneId === "manage") return MANAGE_ITEMS;
  return (ZONE_SECTIONS[zoneId] ?? []).flatMap((g) => g.items);
}

/**
 * The item a zone falls back to when the URL names none: its first BUILT entry.
 * Planned and unbuilt ones are skipped because the pane filters them out (see
 * {@link partitionByReadiness}), and a default it filters out marks a row that
 * is not on screen.
 */
export function defaultZoneItem(zoneId: string): string | null {
  return zoneItems(zoneId).find((i) => i.readiness == null)?.id ?? null;
}

/**
 * The item a zone is showing: the one the URL names if this zone has it, else
 * the zone's default. Pane and content resolve through here so the menu marks
 * the view on screen — a bare `?zone=` used to highlight nothing while the
 * content rendered a default, and an `item` left behind by another zone still
 * matched nothing here while that zone's view fell back.
 */
export function resolveZoneItem(zoneId: string, item: string | null): string | null {
  if (item && zoneItems(zoneId).some((i) => i.id === item)) return item;
  return defaultZoneItem(zoneId);
}
